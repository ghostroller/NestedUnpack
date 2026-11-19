package main

import (
	"archive/zip"
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"math"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"time"
	"unicode/utf8"
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

func (session *unpackSession) unpackZipLayer(layer nestedEvent, destination string, progress func(int)) error {
	file, err := os.Open(layer.Archive)
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
		if err := session.ctx.Err(); err != nil {
			return err
		}
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
	if total == 0 {
		session.stage(layer, "解压当前压缩包", -1)
	} else {
		progress(0)
	}
	for index, entry := range reader.File {
		if err := session.ctx.Err(); err != nil {
			return err
		}
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
		err = session.copyZipLayerMember(layer, output, entry, func(written uint64) {
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

func (session *unpackSession) copyZipLayerMember(layer nestedEvent, output *os.File, entry *zip.File, progress func(uint64)) error {
	if entry.Flags&1 == 0 {
		if session.copyBuffer == nil {
			session.copyBuffer = make([]byte, 4<<20)
		}
		return copyMember(output, entry, progress, session.ctx, session.copyBuffer)
	}
	return session.tryPasswords(layer, errZipCryptoPassword, false, func(password string) error {
		meter := &progressWriter{ctx: session.ctx, file: output, observe: progress}
		err := copyZipCrypto(entry, meter, password)
		if !errors.Is(err, errZipCryptoPassword) {
			return err
		}
		if truncateErr := output.Truncate(0); truncateErr != nil {
			return truncateErr
		}
		if _, seekErr := output.Seek(0, io.SeekStart); seekErr != nil {
			return seekErr
		}
		return err
	})
}

var errZipNeeds7Zip = errors.New("ZIP 成员需要 7-Zip 解密")
var errPasswordSkipped = errors.New("未输入密码")

// A decoder that expects a ZIP at byte zero can use the ZIP section alone.
// The common ZipCrypto case streams directly from the original file instead.
func (session *unpackSession) prepareEncryptedZip(layer nestedEvent, work string) (string, error) {
	file, err := os.Open(layer.Archive)
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
		return layer.Archive, nil
	}
	if err != nil {
		return "", err
	}
	if section.start == 0 && section.length == info.Size() {
		return layer.Archive, nil
	}
	output, err := os.CreateTemp(work, "encrypted-*.zip")
	if err != nil {
		return "", err
	}
	session.stage(layer, "准备加密 ZIP", 0)
	progress := session.progress(layer, "准备加密 ZIP")
	meter := &progressWriter{ctx: session.ctx, file: output, observe: func(n uint64) {
		progress(int(float64(n) * 100 / float64(section.length)))
	}}
	written, copyErr := io.CopyBuffer(meter, io.NewSectionReader(file, section.start, section.length), make([]byte, 8<<20))
	closeErr := output.Close()
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

func publishDirectory(ctx context.Context, stage, source string) (string, error) {
	base := filepath.Base(source)
	stem := strings.TrimSuffix(base, filepath.Ext(base))
	if stem == "" {
		stem = base
	}
	delay := 25 * time.Millisecond
	for number := 1; ; {
		if err := ctx.Err(); err != nil {
			return "", err
		}
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
			number++
			continue
		}
		const sharingViolation syscall.Errno = 32
		const lockViolation syscall.Errno = 33
		if delay > 400*time.Millisecond || !(errors.Is(err, syscall.ERROR_ACCESS_DENIED) ||
			errors.Is(err, sharingViolation) || errors.Is(err, lockViolation)) {
			return "", &os.PathError{Op: "发布解压目录", Path: path, Err: err}
		}
		// A scanner or indexer can temporarily keep a staged file open.
		select {
		case <-ctx.Done():
			return "", ctx.Err()
		case <-time.After(delay):
			delay *= 2
		}
	}
}
