package main

import (
	"archive/zip"
	"bufio"
	"bytes"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"math"
	"os"
	"path/filepath"
	"slices"
	"strconv"
	"strings"
	"syscall"
	"unicode/utf8"
	"unsafe"
)

const reparsePoint = 0x400

func detectArchive(name string) (string, error) {
	file, err := os.Open(name)
	if err != nil {
		return "", err
	}
	defer file.Close()
	info, err := file.Stat()
	if err != nil {
		return "", err
	}
	if !info.Mode().IsRegular() {
		return "", errors.New("无法读取普通文件")
	}
	var head [8]byte
	n, err := file.ReadAt(head[:], 0)
	if err != nil && !errors.Is(err, io.EOF) {
		return "", err
	}
	switch {
	case bytes.HasPrefix(head[:n], []byte("Rar!\x1a\x07\x00")), bytes.HasPrefix(head[:n], []byte("Rar!\x1a\x07\x01\x00")):
		return "RAR", nil
	case bytes.HasPrefix(head[:n], []byte("7z\xbc\xaf\x27\x1c")):
		return "7z", nil
	}
	if _, err = openOuter(file, info.Size()); errors.Is(err, errNoZip) {
		return "", nil
	}
	return "ZIP", err
}

func cleanArchivePath(name string) (string, error) {
	normalized := strings.TrimSuffix(strings.ReplaceAll(name, `\`, "/"), "/")
	for _, part := range strings.Split(normalized, "/") {
		if part == "" || part == "." || part == ".." || strings.TrimRight(part, " .") != part || strings.ContainsAny(part, `<>:"|?*`) || strings.IndexFunc(part, func(character rune) bool { return character < 32 }) >= 0 || !filepath.IsLocal(part) {
			return "", fmt.Errorf("不支持的压缩包路径 %q", name)
		}
	}
	return filepath.FromSlash(normalized), nil
}

func (session *unpackSession) unpackZipLayer(source, destination string, progress func(int)) error {
	file, err := os.Open(source)
	if err != nil {
		return err
	}
	defer file.Close()
	info, err := file.Stat()
	if err != nil {
		return err
	}
	reader, err := openOuter(file, info.Size())
	if err != nil {
		return err
	}
	var total, completed uint64
	relative := make([]string, len(reader.File))
	for index, entry := range reader.File {
		relative[index], err = cleanArchivePath(entry.Name)
		if err != nil {
			return err
		}
		if !entry.FileInfo().IsDir() && !entry.Mode().IsRegular() {
			return fmt.Errorf("不支持的 ZIP 成员 %q", entry.Name)
		}
		if entry.Flags&1 != 0 && !isZipCrypto(entry) {
			return errZipNeeds7Zip
		}
		if entry.CompressedSize64 > math.MaxInt64 || entry.UncompressedSize64 > math.MaxInt64 || entry.UncompressedSize64 > math.MaxUint64-total {
			return fmt.Errorf("ZIP 成员大小超出支持范围 %q", entry.Name)
		}
		total += entry.UncompressedSize64
	}
	for index, entry := range reader.File {
		path := filepath.Join(destination, relative[index])
		if entry.FileInfo().IsDir() {
			if err := os.MkdirAll(path, 0700); err != nil {
				return err
			}
			continue
		}
		if err := os.MkdirAll(filepath.Dir(path), 0700); err != nil {
			return err
		}
		output, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0600)
		if err != nil {
			return err
		}
		err = session.copyZipLayerMember(output, entry, func(written uint64) {
			if total != 0 {
				progress(min(99, int(float64(completed+written)*100/float64(total))))
			}
		})
		closeErr := output.Close()
		if err != nil || closeErr != nil {
			return fmt.Errorf("%s · %w", entry.Name, errors.Join(err, closeErr))
		}
		completed += entry.UncompressedSize64
	}
	return nil
}

func (session *unpackSession) copyZipLayerMember(output *os.File, entry *zip.File, progress func(uint64)) error {
	if entry.Flags&1 == 0 {
		return copyMember(output, entry, progress)
	}
	for {
		if session.password == "" {
			password, err := session.askPassword()
			if err != nil {
				return err
			}
			session.password = password
		}
		meter := &progressWriter{file: output, observe: progress}
		err := copyZipCrypto(entry, meter, session.password)
		if !errors.Is(err, errZipCryptoPassword) {
			return err
		}
		fmt.Println("\nZIP 密码校验失败，请重试。")
		session.password = ""
		if err := output.Truncate(0); err != nil {
			return err
		}
		if _, err := output.Seek(0, io.SeekStart); err != nil {
			return err
		}
	}
}

var errZipNeeds7Zip = errors.New("ZIP 成员需要 7-Zip 解密")
var errPasswordSkipped = errors.New("未输入密码")

// A decoder that expects a ZIP at byte zero can use the ZIP section alone.
// The common ZipCrypto case streams directly from the original file instead.
func prepareEncryptedZip(source, work string) (string, error) {
	file, err := os.Open(source)
	if err != nil {
		return "", err
	}
	defer file.Close()
	info, err := file.Stat()
	if err != nil {
		return "", err
	}
	section, err := locateZip(file, info.Size())
	if errors.Is(err, errNoZip) {
		// The caller has already opened a ZIP through archive/zip.
		return source, nil
	}
	if err != nil {
		return "", err
	}
	if section.start == 0 && section.length == info.Size() {
		return source, nil
	}
	output, err := os.CreateTemp(work, "encrypted-*.zip")
	if err != nil {
		return "", err
	}
	last := -1
	meter := &progressWriter{file: output, observe: func(n uint64) {
		percent := int(float64(n) * 100 / float64(section.length))
		if percent != last {
			fmt.Printf("\r准备加密 ZIP %3d%%", percent)
			last = percent
		}
	}}
	fmt.Println()
	written, copyErr := io.CopyBuffer(meter, io.NewSectionReader(file, section.start, section.length), make([]byte, 8<<20))
	closeErr := output.Close()
	fmt.Println()
	if copyErr != nil || closeErr != nil {
		return "", errors.Join(copyErr, closeErr)
	}
	if written != section.length {
		return "", io.ErrUnexpectedEOF
	}
	return output.Name(), nil
}

func extractedFiles(directory string) ([]string, error) {
	var files []string
	err := filepath.WalkDir(directory, func(name string, entry fs.DirEntry, walkErr error) error {
		if walkErr != nil {
			return walkErr
		}
		info, err := entry.Info()
		if err != nil {
			return err
		}
		attributes, ok := info.Sys().(*syscall.Win32FileAttributeData)
		if (ok && attributes.FileAttributes&reparsePoint != 0) || (!info.IsDir() && !info.Mode().IsRegular()) {
			return fmt.Errorf("不支持的解压成员 %q", name)
		}
		if !info.IsDir() {
			files = append(files, name)
		}
		return nil
	})
	return files, err
}

func nextArchive(files []string) (path, kind string, err error) {
	if len(files) == 1 {
		kind, err := detectArchive(files[0])
		return files[0], kind, err
	}
	if len(files) < 2 {
		return "", "", nil
	}
	var stem, first string
	indices := make([]int, 0, len(files))
	for _, name := range files {
		extension := filepath.Ext(name)
		if len(extension) < 4 {
			return "", "", nil
		}
		part, err := strconv.Atoi(extension[1:])
		if err != nil || part < 1 || fmt.Sprintf(".%03d", part) != extension {
			return "", "", nil
		}
		base := strings.TrimSuffix(name, extension)
		if stem != "" && !strings.EqualFold(stem, base) {
			return "", "", nil
		}
		stem = base
		if part == 1 {
			first = name
		}
		indices = append(indices, part)
	}
	slices.Sort(indices)
	for index, part := range indices {
		if part != index+1 {
			return "", "", nil
		}
	}
	kind, err = detectArchive(first)
	if kind != "7z" {
		return "", "", err
	}
	return first, "7z 分卷", err
}

func publishDirectory(stage, source string) (string, error) {
	base := filepath.Base(source)
	stem := strings.TrimSuffix(base, filepath.Ext(base))
	if stem == "" {
		stem = base
	}
	for number := 1; ; number++ {
		suffix := "-解压"
		if number > 1 {
			suffix += fmt.Sprintf(" (%d)", number)
		}
		name := truncateName(stem, 255-utf8.RuneCountInString(suffix)) + suffix
		path := filepath.Join(filepath.Dir(source), name)
		err := moveNew(stage, path)
		if err == nil {
			return path, nil
		}
		if _, statErr := os.Lstat(path); statErr == nil || errors.Is(err, syscall.ERROR_FILE_EXISTS) || errors.Is(err, syscall.ERROR_ALREADY_EXISTS) {
			continue
		}
		return "", err
	}
}

type unpackSession struct {
	sevenZip string
	password string
	input    *bufio.Reader
}

func (session *unpackSession) askPassword() (string, error) {
	fmt.Print("\n请输入压缩包密码（留空则跳过这个源文件）：")
	var mode uint32
	handle := syscall.Handle(os.Stdin.Fd())
	if syscall.GetConsoleMode(handle, &mode) == nil {
		setMode := syscall.NewLazyDLL("kernel32.dll").NewProc("SetConsoleMode")
		setMode.Call(uintptr(handle), uintptr(mode&^4)) // ENABLE_ECHO_INPUT
		defer setMode.Call(uintptr(handle), uintptr(mode))
	}
	line, err := session.input.ReadString('\n')
	fmt.Println()
	password := strings.TrimRight(line, "\r\n")
	if password == "" {
		return "", errPasswordSkipped
	}
	if err != nil && !errors.Is(err, io.EOF) {
		return "", err
	}
	return password, nil
}

func (session *unpackSession) unpackUsing7Zip(input, dest, work string, zipInput bool, progress func(int)) error {
	if session.sevenZip == "" {
		var err error
		session.sevenZip, err = find7Zip()
		if err != nil {
			return err
		}
	}
	path := input
	if zipInput {
		var err error
		path, err = prepareEncryptedZip(input, work)
		if err != nil {
			return err
		}
		if path != input {
			defer os.Remove(path)
		}
	}
	return unpackWith7Zip(session.sevenZip, path, dest, &session.password, session.askPassword, progress)
}

func (session *unpackSession) unpack(source, kind string) (string, error) {
	parent := filepath.Dir(source)
	work, err := os.MkdirTemp(parent, ".nestedunpack-nested-")
	if err != nil {
		return "", err
	}
	defer os.RemoveAll(work)
	input, previous := source, ""
	for depth := 1; depth <= 64; depth++ {
		dest := filepath.Join(work, strconv.Itoa(depth))
		if err := os.Mkdir(dest, 0700); err != nil {
			return "", err
		}
		fmt.Printf("第 %d 层 · %s · %s\n", depth, kind, filepath.Base(input))
		last := -1
		progress := func(percent int) {
			if percent == last || percent < last {
				return
			}
			last = percent
			fmt.Printf("\r解压进度 %3d%%", percent)
		}
		progress(0)
		if kind == "ZIP" {
			err = session.unpackZipLayer(input, dest, progress)
		}
		if kind != "ZIP" || errors.Is(err, errZipNeeds7Zip) {
			err = session.unpackUsing7Zip(input, dest, work, kind == "ZIP", progress)
		}
		if err != nil {
			fmt.Println()
			return "", fmt.Errorf("第 %d 层 %s · %w", depth, kind, err)
		}
		progress(100)
		fmt.Println()
		files, err := extractedFiles(dest)
		if err != nil {
			return "", err
		}
		if previous != "" {
			if err := os.RemoveAll(previous); err != nil {
				return "", err
			}
		}
		input, kind, err = nextArchive(files)
		if err != nil {
			return "", err
		}
		if kind == "" {
			return publishDirectory(dest, source)
		}
		previous = dest
	}
	return "", errors.New("嵌套层数超过 64")
}

func runNested(paths []string, fromExplorer bool) int {
	session := unpackSession{input: bufio.NewReader(os.Stdin)}
	if fromExplorer {
		title, _ := syscall.UTF16PtrFromString("解开嵌套压缩包")
		syscall.NewLazyDLL("kernel32.dll").NewProc("SetConsoleTitleW").Call(uintptr(unsafe.Pointer(title)))
		defer func() {
			fmt.Print("\n按 Enter 关闭窗口。")
			_, _ = session.input.ReadString('\n')
		}()
	}
	var success, skipped, failed int
	seen := make(map[string]bool)
	for index, selected := range paths {
		name, err := filepath.Abs(selected)
		if err != nil {
			fmt.Printf("失败 · %s · %s\n", selected, err)
			failed++
			continue
		}
		key := strings.ToLower(name)
		if seen[key] {
			continue
		}
		seen[key] = true
		fmt.Printf("\n[%d/%d] %s\n", index+1, len(paths), name)
		kind, err := detectArchive(name)
		if err == nil && kind == "" {
			fmt.Println("跳过 · 未识别到 ZIP、RAR 或 7z")
			skipped++
			continue
		}
		var destination string
		if err == nil {
			destination, err = session.unpack(name, kind)
		}
		if errors.Is(err, errPasswordSkipped) {
			fmt.Println("跳过 ·", err)
			skipped++
			continue
		}
		if err != nil {
			fmt.Println("失败 ·", err)
			failed++
			continue
		}
		fmt.Println("已解压到", destination)
		success++
	}
	fmt.Printf("\n完成 · 成功 %d 个 · 跳过 %d 个 · 失败 %d 个\n", success, skipped, failed)
	if failed > 0 {
		return 1
	}
	return 0
}
