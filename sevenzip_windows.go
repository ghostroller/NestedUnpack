package main

import (
	"bufio"
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"syscall"
	"unicode/utf8"
)

var err7ZipPassword = errors.New("压缩包需要正确的密码")

func find7Zip() (string, error) {
	for _, variable := range []string{"ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"} {
		if dir := os.Getenv(variable); filepath.IsAbs(dir) {
			candidate := filepath.Join(dir, "7-Zip", "7z.exe")
			if info, err := os.Stat(candidate); err == nil && info.Mode().IsRegular() {
				return candidate, nil
			}
		}
	}
	const local = `D:\Program_Files\7-Zip\7z.exe`
	if info, err := os.Stat(local); err == nil && info.Mode().IsRegular() {
		return local, nil
	}
	if candidate, err := exec.LookPath("7z.exe"); err == nil {
		if info, err := os.Stat(candidate); err == nil && info.Mode().IsRegular() {
			return filepath.Abs(candidate)
		}
	}
	return "", errors.New("找不到 7-Zip 命令行程序 7z.exe")
}

func unpackWith7Zip(ctx context.Context, exe, archive, dest string, tryPasswords func(func(string) error) error, progress func(int)) error {
	dest, err := filepath.Abs(dest)
	if err != nil {
		return err
	}
	entries, err := sevenZipStageEntries(dest)
	if err != nil {
		return err
	}
	if len(entries) != 0 {
		return errors.New("解压暂存目录必须为空")
	}
	var listing sevenZipListing
	listed := false
	return tryPasswords(func(candidate string) error {
		if !listed {
			listing = sevenZipListing{}
			// Bare listing omits free-form comments and container metadata.
			args := []string{"l", "-slt", "-ba", "-sccUTF-8", "-bsp0", "--", archive}
			if err := run7Zip(ctx, exe, candidate, args, nil, listing.inspect); err != nil {
				return err
			}
			listed = true
		}
		if listing.encrypted && candidate == "" {
			return err7ZipPassword
		}
		progress(0)
		err := extractListed7Zip(ctx, exe, archive, dest, candidate, progress)
		if errors.Is(err, err7ZipPassword) {
			if cleanupErr := clear7ZipStage(dest); cleanupErr != nil {
				return fmt.Errorf("清理本次解压的文件失败 · %w", cleanupErr)
			}
		}
		return err
	})
}

func extractListed7Zip(ctx context.Context, exe, archive, dest, password string, report func(int)) error {
	args := []string{"x", "-sccUTF-8", "-bsp1", "-bse2", "-bb0", "-y", "-sns-", "-o" + dest, "--", archive}
	return run7Zip(ctx, exe, password, args, split7ZipProgress, func(line string) error {
		if number, _, ok := strings.Cut(strings.TrimSpace(line), "%"); ok && len(number) <= 3 {
			if percent, err := strconv.Atoi(number); err == nil && percent >= 0 && percent <= 100 {
				report(min(percent, 99))
			}
		}
		return nil
	})
}

func sevenZipStageEntries(dest string) ([]os.DirEntry, error) {
	info, err := os.Lstat(dest)
	if err != nil {
		return nil, err
	}
	attrs, _ := info.Sys().(*syscall.Win32FileAttributeData)
	if !info.IsDir() || attrs == nil || attrs.FileAttributes&syscall.FILE_ATTRIBUTE_REPARSE_POINT != 0 {
		return nil, errors.New("解压暂存位置必须是普通目录")
	}
	return os.ReadDir(dest)
}

func clear7ZipStage(dest string) error {
	entries, err := sevenZipStageEntries(dest)
	if err != nil {
		return err
	}
	for _, entry := range entries {
		name := entry.Name()
		if !filepath.IsLocal(name) || filepath.Base(name) != name || name == "." {
			return fmt.Errorf("解压暂存目录含异常文件名 %q", name)
		}
		if err := os.RemoveAll(filepath.Join(dest, name)); err != nil {
			return err
		}
	}
	return nil
}

type sevenZipListing struct {
	path      string
	encrypted bool
}

func (listing *sevenZipListing) inspect(line string) error {
	if line == "" {
		listing.path = ""
		return nil
	}
	key, value, ok := strings.Cut(line, " = ")
	if !ok {
		return nil
	}
	switch key {
	case "Path":
		if _, err := cleanArchivePath(value); err != nil {
			return fmt.Errorf("7-Zip 成员路径 %q 不安全 · %w", value, err)
		}
		listing.path = value
	case "Encrypted":
		listing.encrypted = listing.encrypted || value == "+"
	case "Hard Link", "Symbolic Link", "Junction", "Copy Link", "Reparse Point", "Alternate Stream":
		if value != "" && value != "-" {
			return fmt.Errorf("7-Zip 成员 %q 包含链接或备用数据流", listing.path)
		}
	case "Attributes", "Mode":
		for _, field := range strings.Fields(value) {
			if strings.ContainsRune(field, 'L') || (len(field) == 10 && strings.ContainsRune("lbcps", rune(field[0]))) {
				return fmt.Errorf("7-Zip 成员 %q 的属性包含链接或特殊文件", listing.path)
			}
		}
	}
	return nil
}

func run7Zip(ctx context.Context, exe, password string, arguments []string, split bufio.SplitFunc, inspect func(string) error) error {
	command := exec.CommandContext(ctx, exe, arguments...)
	const createNoWindow = 0x08000000
	command.SysProcAttr = &syscall.SysProcAttr{CreationFlags: createNoWindow}
	if password != "" {
		if !utf8.ValidString(password) || strings.ContainsAny(password, "\x00\r\n") {
			return errors.New("解压密码包含命令行程序无法读取的字符")
		}
		// Without -p, 7-Zip reads its UTF-8 password prompt from stdin.
		// This keeps the password out of the process command line.
		command.Stdin = strings.NewReader(password + "\n")
	}
	stdout, err := command.StdoutPipe()
	if err != nil {
		return err
	}
	diagnostic := &sevenZipDiagnostic{}
	command.Stderr = diagnostic
	if err := command.Start(); err != nil {
		return fmt.Errorf("无法启动 7-Zip · %w", err)
	}
	scanner := bufio.NewScanner(stdout)
	scanner.Buffer(make([]byte, 4096), 1<<20)
	if split != nil {
		scanner.Split(split)
	}
	var inspectionErr error
	prompted := false
	for scanner.Scan() {
		line := scanner.Text()
		prompted = prompted || line == "Enter password (will not be echoed):"
		if inspectionErr == nil {
			inspectionErr = inspect(line)
		}
	}
	if scanErr := scanner.Err(); scanErr != nil {
		if inspectionErr == nil {
			inspectionErr = fmt.Errorf("无法读取 7-Zip 输出 · %w", scanErr)
		}
		_, _ = io.Copy(io.Discard, stdout)
	}
	commandErr := command.Wait()
	if err := ctx.Err(); err != nil {
		return err
	}
	if inspectionErr != nil {
		return inspectionErr
	}
	if commandErr == nil {
		return nil
	}
	message := strings.TrimSpace(diagnostic.String())
	lower := strings.ToLower(message)
	if strings.Contains(lower, "wrong password?") || strings.Contains(lower, "error: wrong password") || (prompted && password == "") {
		return err7ZipPassword
	}
	if prompted && strings.Contains(lower, "break signaled") {
		return errors.New("当前 7-Zip 无法通过标准输入读取密码")
	}
	if message == "" {
		return fmt.Errorf("7-Zip %s 失败 · %w", arguments[0], commandErr)
	}
	return fmt.Errorf("7-Zip %s 失败 · %s", arguments[0], message)
}

type sevenZipDiagnostic struct{ bytes.Buffer }

func (diagnostic *sevenZipDiagnostic) Write(p []byte) (int, error) {
	const limit = 8 << 10
	_, _ = diagnostic.Buffer.Write(p[:min(len(p), limit-diagnostic.Len())])
	return len(p), nil
}

func split7ZipProgress(data []byte, atEOF bool) (int, []byte, error) {
	if index := bytes.IndexAny(data, "\r\n"); index >= 0 {
		return index + 1, data[:index], nil
	}
	if atEOF && len(data) > 0 {
		return len(data), data, nil
	}
	return 0, nil, nil
}
