package main

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"runtime"
	"slices"
	"sync"
	"syscall"
	"unsafe"
)

var (
	passwordBookCrypt32    = syscall.NewLazyDLL("crypt32.dll")
	passwordBookProtect    = passwordBookCrypt32.NewProc("CryptProtectData")
	passwordBookUnprotect  = passwordBookCrypt32.NewProc("CryptUnprotectData")
	passwordBookKernel32   = syscall.NewLazyDLL("kernel32.dll")
	passwordBookLockFile   = passwordBookKernel32.NewProc("LockFileEx")
	passwordBookUnlockFile = passwordBookKernel32.NewProc("UnlockFileEx")
)

type passwordBook struct {
	path      string
	mu        sync.Mutex
	passwords []string
}

func loadPasswordBook() (*passwordBook, error) {
	config := os.Getenv("LOCALAPPDATA")
	if !filepath.IsAbs(config) {
		var err error
		config, err = os.UserConfigDir()
		if err != nil {
			return nil, err
		}
	}
	if !filepath.IsAbs(config) {
		return nil, errors.New("无法定位当前用户的密码簿目录")
	}
	book := &passwordBook{path: filepath.Join(config, "NestedUnpack", "passwords.dpapi")}
	unlock, err := lockPasswordBook(book.path, false)
	if errors.Is(err, os.ErrNotExist) {
		return book, nil
	}
	if err != nil {
		return book, err
	}
	defer unlock()
	book.passwords, err = readPasswordBook(book.path)
	return book, err
}

func (book *passwordBook) Passwords() []string {
	book.mu.Lock()
	defer book.mu.Unlock()
	return slices.Clone(book.passwords)
}

func (book *passwordBook) Remember(password string) error {
	if password == "" {
		return nil
	}
	return book.update(func(passwords []string) ([]string, bool) {
		if slices.Contains(passwords, password) {
			return passwords, false
		}
		return append([]string{password}, passwords...), true
	})
}

func (book *passwordBook) Forget(password string) error {
	return book.update(func(passwords []string) ([]string, bool) {
		index := slices.Index(passwords, password)
		if index == -1 {
			return passwords, false
		}
		return slices.Delete(passwords, index, index+1), true
	})
}

func (book *passwordBook) update(change func([]string) ([]string, bool)) error {
	book.mu.Lock()
	defer book.mu.Unlock()
	if err := os.MkdirAll(filepath.Dir(book.path), 0700); err != nil {
		return err
	}
	unlock, err := lockPasswordBook(book.path, true)
	if err != nil {
		return err
	}
	defer unlock()
	// Reload under the lock so concurrent jobs and editors preserve each other's changes.
	passwords, err := readPasswordBook(book.path)
	if err != nil {
		return err
	}
	passwords, changed := change(passwords)
	if changed {
		if err := writePasswordBook(book.path, passwords); err != nil {
			return err
		}
	}
	book.passwords = passwords
	return nil
}

func lockPasswordBook(path string, exclusive bool) (func(), error) {
	file, err := os.OpenFile(path+".lock", os.O_CREATE|os.O_RDWR, 0600)
	if err != nil {
		return nil, err
	}
	var flags uintptr
	if exclusive {
		flags = 2 // LOCKFILE_EXCLUSIVE_LOCK
	}
	var position syscall.Overlapped
	result, _, err := passwordBookLockFile.Call(file.Fd(), flags, 0, 1, 0, uintptr(unsafe.Pointer(&position)))
	if result == 0 {
		_ = file.Close()
		return nil, fmt.Errorf("锁定密码簿失败：%w", err)
	}
	return func() {
		passwordBookUnlockFile.Call(file.Fd(), 0, 1, 0, uintptr(unsafe.Pointer(&position)))
		_ = file.Close()
	}, nil
}

func readPasswordBook(path string) ([]string, error) {
	encrypted, err := os.ReadFile(path)
	if errors.Is(err, os.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	plain, err := cryptPasswordBook(passwordBookUnprotect, encrypted)
	if err != nil {
		return nil, fmt.Errorf("无法解密密码簿：%w", err)
	}
	defer clear(plain)
	var entries [][]byte
	decodeErr := json.Unmarshal(plain, &entries)
	defer func() {
		for _, entry := range entries {
			clear(entry)
		}
	}()
	if decodeErr != nil {
		return nil, errors.New("密码簿格式无效")
	}
	passwords := make([]string, 0, len(entries))
	seen := make(map[string]struct{}, len(entries))
	for _, entry := range entries {
		password := string(entry)
		if _, exists := seen[password]; password != "" && !exists {
			passwords = append(passwords, password)
			seen[password] = struct{}{}
		}
	}
	return passwords, nil
}

func writePasswordBook(path string, passwords []string) error {
	entries := make([][]byte, len(passwords))
	for index, password := range passwords {
		entries[index] = []byte(password)
	}
	plain, err := json.Marshal(entries)
	for _, entry := range entries {
		clear(entry)
	}
	if err != nil {
		return errors.New("无法编码密码簿")
	}
	encrypted, err := cryptPasswordBook(passwordBookProtect, plain)
	clear(plain)
	if err != nil {
		return fmt.Errorf("无法加密密码簿：%w", err)
	}
	temp, err := os.CreateTemp(filepath.Dir(path), "passwords-*.tmp")
	if err != nil {
		return err
	}
	defer os.Remove(temp.Name())
	_, writeErr := temp.Write(encrypted)
	if writeErr == nil {
		writeErr = temp.Sync()
	}
	if err := errors.Join(writeErr, temp.Close()); err != nil {
		return err
	}
	return os.Rename(temp.Name(), path)
}

type passwordBookBlob struct {
	size uint32
	data *byte
}

func cryptPasswordBook(proc *syscall.LazyProc, data []byte) ([]byte, error) {
	if len(data) == 0 || uint64(len(data)) > math.MaxUint32 {
		return nil, errors.New("密码簿数据长度无效")
	}
	input := passwordBookBlob{size: uint32(len(data)), data: &data[0]}
	var output passwordBookBlob
	const noUI = 1 // CRYPTPROTECT_UI_FORBIDDEN
	result, _, err := proc.Call(uintptr(unsafe.Pointer(&input)), 0, 0, 0, 0, noUI, uintptr(unsafe.Pointer(&output)))
	runtime.KeepAlive(data)
	if result == 0 {
		return nil, err
	}
	defer syscall.LocalFree(syscall.Handle(uintptr(unsafe.Pointer(output.data))))
	buffer := unsafe.Slice(output.data, output.size)
	defer clear(buffer)
	return bytes.Clone(buffer), nil
}
