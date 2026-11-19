package main

import (
	"archive/zip"
	"bufio"
	"bytes"
	"context"
	"errors"
	"fmt"
	"hash/crc32"
	"io"
	"math"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"unicode/utf16"
)

var errCompressedBlock = errors.New("Deflate 流含压缩块")

func extractRAR(entry *zip.File, source string) (string, error) {
	if entry.Flags&1 != 0 {
		return "", errors.New("外层 ZIP 成员受密码保护")
	}
	if entry.CompressedSize64 > math.MaxInt64 || entry.UncompressedSize64 > math.MaxInt64 {
		return "", errors.New("成员大小超出支持范围")
	}
	parent := filepath.Dir(source)
	part, err := os.CreateTemp(parent, ".nestedunpack-*.part")
	if err != nil {
		return "", err
	}
	defer os.Remove(part.Name())
	defer part.Close()

	if err := copyMember(part, entry, nil, nil, nil); err != nil {
		return "", err
	}
	var header [8]byte
	read, err := part.ReadAt(header[:], 0)
	if err != nil && !errors.Is(err, io.EOF) {
		return "", err
	}
	if !bytes.HasPrefix(header[:read], []byte("Rar!\x1a\x07\x00")) && !bytes.HasPrefix(header[:read], []byte("Rar!\x1a\x07\x01\x00")) {
		return "", errors.New("成员没有 RAR 文件头")
	}
	if err := part.Sync(); err != nil {
		return "", err
	}
	if err := part.Close(); err != nil {
		return "", err
	}
	return publishRAR(part.Name(), source)
}

type progressWriter struct {
	ctx     context.Context
	file    *os.File
	written uint64
	observe func(uint64)
}

func (writer *progressWriter) Write(data []byte) (int, error) {
	if writer.ctx != nil {
		if err := writer.ctx.Err(); err != nil {
			return 0, err
		}
	}
	n, err := writer.file.Write(data)
	writer.written += uint64(n)
	writer.observe(writer.written)
	return n, err
}

func copyMember(output *os.File, entry *zip.File, observe func(uint64), ctx context.Context, buffer []byte) error {
	var destination io.Writer = output
	meter := &progressWriter{ctx: ctx, file: output, observe: observe}
	if observe != nil {
		destination = meter
	}
	if entry.Method == zip.Deflate {
		err := copyStoredDeflate(destination, entry)
		if !errors.Is(err, errCompressedBlock) {
			return err
		}
		if err := output.Truncate(0); err != nil {
			return err
		}
		if _, err := output.Seek(0, io.SeekStart); err != nil {
			return err
		}
		meter.written = 0
	}
	reader, err := entry.Open()
	if err != nil {
		return err
	}
	defer reader.Close()
	checksum := crc32.NewIEEE()
	if buffer == nil {
		buffer = make([]byte, 4<<20)
	}
	written, err := io.CopyBuffer(io.MultiWriter(destination, checksum), reader, buffer)
	if err != nil {
		return err
	}
	if uint64(written) != entry.UncompressedSize64 || checksum.Sum32() != entry.CRC32 {
		return zip.ErrChecksum
	}
	return nil
}

// Stored DEFLATE blocks are byte-aligned. Any Huffman block sends this member
// to the standard streaming decoder, which also handles mixed streams.
func copyStoredDeflate(destination io.Writer, entry *zip.File) error {
	raw, err := entry.OpenRaw()
	if err != nil {
		return err
	}
	input := bufio.NewReaderSize(raw, 8<<20)
	output := bufio.NewWriterSize(destination, 8<<20)
	var consumed, written uint64
	var crc uint32
	for {
		kind, err := input.ReadByte()
		if err != nil {
			return err
		}
		switch kind & 6 {
		case 2, 4:
			return errCompressedBlock
		case 6:
			return zip.ErrFormat
		}
		lengths, err := input.Peek(4)
		if err != nil {
			return err
		}
		length := le.Uint16(lengths)
		if length != ^le.Uint16(lengths[2:]) || uint64(length) > entry.UncompressedSize64-written {
			return zip.ErrFormat
		}
		input.Discard(4)
		block, err := input.Peek(int(length))
		if err != nil {
			return err
		}
		crc = crc32.Update(crc, crc32.IEEETable, block)
		if _, err := output.Write(block); err != nil {
			return err
		}
		input.Discard(int(length))
		consumed += uint64(length) + 5
		written += uint64(length)
		if kind&1 != 0 {
			break
		}
	}
	if consumed != entry.CompressedSize64 || written != entry.UncompressedSize64 {
		return zip.ErrFormat
	}
	if crc != entry.CRC32 {
		return zip.ErrChecksum
	}
	return output.Flush()
}

func publishRAR(part, source string) (string, error) {
	dir := filepath.Dir(source)
	base := filepath.Base(source)
	stem := strings.TrimSuffix(base, filepath.Ext(base))
	if stem == "" {
		stem = base
	}
	for number := 1; ; number++ {
		suffix := ".rar"
		if number > 1 {
			suffix = fmt.Sprintf(" (%d).rar", number)
		}
		name := truncateName(stem, 255-len(suffix)) + suffix
		destination := filepath.Join(dir, name)
		err := moveNew(part, destination)
		if err == nil {
			return destination, nil
		}
		if errors.Is(err, syscall.ERROR_ALREADY_EXISTS) || errors.Is(err, syscall.ERROR_FILE_EXISTS) {
			continue
		}
		if _, statErr := os.Lstat(destination); statErr == nil {
			continue
		}
		return "", &os.PathError{Op: "发布 RAR", Path: destination, Err: err}
	}
}

func truncateName(stem string, limit int) string {
	for index, char := range stem {
		limit -= utf16.RuneLen(char)
		if limit < 0 {
			return stem[:index]
		}
	}
	return stem
}

func moveNew(from, to string) error {
	longPath := func(path string) string {
		if strings.HasPrefix(path, `\\?\`) || strings.HasPrefix(path, `\\.\`) {
			return path
		}
		if strings.HasPrefix(path, `\\`) {
			return `\\?\UNC\` + path[2:]
		}
		return `\\?\` + path
	}
	oldPath, err := syscall.UTF16PtrFromString(longPath(from))
	if err != nil {
		return err
	}
	newPath, err := syscall.UTF16PtrFromString(longPath(to))
	if err != nil {
		return err
	}
	return syscall.MoveFile(oldPath, newPath)
}
