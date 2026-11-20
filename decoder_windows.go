package main

import (
	"archive/zip"
	"context"
	"errors"
	"fmt"
	"hash/crc32"
	"io"
	"io/fs"
	"math"
	"os"
	"path/filepath"
	"syscall"

	"github.com/bodgit/sevenzip"
	"github.com/nwaples/rardecode/v2"
	"go4.org/readerutil"
)

var errArchivePassword = errors.New("压缩包需要正确的密码，或加密数据已损坏")

// Wrap the read side so cancellation also reaches a decoder between writes.
type archiveInputFile struct {
	*os.File
	ctx context.Context
}

func (file archiveInputFile) Read(data []byte) (int, error) {
	if err := file.ctx.Err(); err != nil {
		return 0, err
	}
	return file.File.Read(data)
}

func (file archiveInputFile) ReadAt(data []byte, offset int64) (int, error) {
	if err := file.ctx.Err(); err != nil {
		return 0, err
	}
	return file.File.ReadAt(data, offset)
}

func openArchiveInput(ctx context.Context, name string) (archiveInputFile, int64, error) {
	if err := ctx.Err(); err != nil {
		return archiveInputFile{}, 0, err
	}
	file, err := os.Open(name)
	if err != nil {
		return archiveInputFile{}, 0, err
	}
	info, err := file.Stat()
	if err != nil {
		file.Close()
		return archiveInputFile{}, 0, err
	}
	if !info.Mode().IsRegular() {
		file.Close()
		return archiveInputFile{}, 0, fmt.Errorf("无法读取普通文件 %q", name)
	}
	return archiveInputFile{file, ctx}, info.Size(), nil
}

func (session *unpackSession) unpack7ZipLayer(layer nestedEvent, archive nestedArchive, dest string, progress func(int)) error {
	parts := archive.Parts
	if len(parts) == 0 {
		parts = []string{archive.Path}
	}
	volumes := make([]readerutil.SizeReaderAt, 0, len(parts))
	var size int64
	for _, path := range parts {
		file, length, err := openArchiveInput(session.ctx, path)
		if err != nil {
			return err
		}
		defer file.Close()
		if length > math.MaxInt64-size {
			return errors.New("7z 分卷总大小超出支持范围")
		}
		size += length
		volumes = append(volumes, io.NewSectionReader(file, 0, length))
	}
	input := readerutil.NewMultiReaderAt(volumes...)
	return session.tryArchivePasswords(layer, dest, func(password string) error {
		reader, err := sevenzip.NewReaderWithPassword(input, size, password)
		if err != nil {
			return sevenZipReadError(err)
		}
		var total, completed uint64
		paths := make([]string, len(reader.File))
		for i, entry := range reader.File {
			if err := session.ctx.Err(); err != nil {
				return err
			}
			paths[i], err = archiveDestination(dest, entry.Name, entry.Mode(), entry.Attributes&reparsePoint != 0)
			if err != nil {
				return err
			}
			if entry.UncompressedSize > math.MaxInt64 || entry.UncompressedSize > math.MaxUint64-total {
				return fmt.Errorf("7z 成员大小超出支持范围 %q", entry.Name)
			}
			total += entry.UncompressedSize
		}
		session.startArchiveProgress(layer, progress, total)
		for i, entry := range reader.File {
			if err := session.ctx.Err(); err != nil {
				return err
			}
			if entry.Mode().IsDir() {
				if err := os.MkdirAll(paths[i], 0700); err != nil {
					return err
				}
				continue
			}
			reader, err := entry.Open()
			if err != nil {
				return fmt.Errorf("%s · %w", entry.Name, sevenZipReadError(err))
			}
			checksum := crc32.NewIEEE()
			written, copyErr := session.writeArchiveFile(paths[i], reader, int64(entry.UncompressedSize), checksum, archiveFileProgress(completed, total, progress))
			closeErr := reader.Close()
			if copyErr != nil || closeErr != nil {
				return fmt.Errorf("%s · %w", entry.Name, errors.Join(sevenZipReadError(copyErr), closeErr))
			}
			// sevenzip exposes zero for entries without a CRC. Its AES-CBC
			// decoder can return bytes without an error under a wrong key.
			if entry.UncompressedSize != 0 && entry.CRC32 != 0 && checksum.Sum32() != entry.CRC32 {
				return fmt.Errorf("%s · %w", entry.Name, errors.Join(errArchivePassword, zip.ErrChecksum))
			}
			completed += uint64(written)
		}
		return nil
	})
}

func sevenZipReadError(err error) error {
	var readErr *sevenzip.ReadError
	if errors.As(err, &readErr) && readErr.Encrypted && !errors.Is(err, context.Canceled) && !errors.Is(err, context.DeadlineExceeded) {
		return errors.Join(errArchivePassword, err)
	}
	return err
}

type rarVolumeFS struct{ ctx context.Context }

func (volume rarVolumeFS) Open(name string) (fs.File, error) {
	file, _, err := openArchiveInput(volume.ctx, name)
	if err != nil {
		return nil, err
	}
	return file, nil
}

func (session *unpackSession) unpackRARLayer(layer nestedEvent, dest string, progress func(int)) error {
	var listing []*rardecode.File
	var paths []string
	var total uint64
	var listed, encryptedHeaders, encryptedFiles bool
	return session.tryArchivePasswords(layer, dest, func(password string) error {
		options := []rardecode.Option{rardecode.FileSystem(rarVolumeFS{session.ctx})}
		if password != "" {
			options = append(options, rardecode.Password(password))
		}
		if !listed {
			var err error
			listing, err = rardecode.List(layer.Archive, options...)
			if errors.Is(err, rardecode.ErrArchiveEncrypted) {
				encryptedHeaders = true
			}
			if err != nil {
				return rarReadError(err, encryptedHeaders)
			}
			paths = make([]string, len(listing))
			total, encryptedFiles = 0, false
			sizeKnown := true
			for i, entry := range listing {
				if err := session.ctx.Err(); err != nil {
					return err
				}
				paths[i], err = rarDestination(dest, &entry.FileHeader)
				if err != nil {
					return err
				}
				encryptedFiles = encryptedFiles || entry.Encrypted
				if entry.UnKnownSize {
					sizeKnown = false
					continue
				}
				if entry.UnPackedSize < 0 || uint64(entry.UnPackedSize) > math.MaxUint64-total {
					return fmt.Errorf("RAR 成员大小超出支持范围 %q", entry.Name)
				}
				total += uint64(entry.UnPackedSize)
			}
			if !sizeKnown {
				total = 0
			}
			listed = !encryptedHeaders
		}
		if password == "" && encryptedFiles {
			return errArchivePassword
		}
		reader, err := rardecode.OpenReader(layer.Archive, options...)
		if err != nil {
			return rarReadError(err, encryptedHeaders)
		}
		defer reader.Close()
		session.startArchiveProgress(layer, progress, total)
		var completed uint64
		for i, entry := range listing {
			if err := session.ctx.Err(); err != nil {
				return err
			}
			header, err := reader.Next()
			if err != nil {
				return rarReadError(err, encryptedHeaders || entry.Encrypted)
			}
			if header.Name != entry.Name || header.Mode() != entry.Mode() || header.LinkType != entry.LinkType || header.Attributes != entry.Attributes || header.UnPackedSize != entry.UnPackedSize || header.UnKnownSize != entry.UnKnownSize {
				return errors.New("RAR 压缩包目录在解压过程中发生变化")
			}
			if header.IsDir {
				if err := os.MkdirAll(paths[i], 0700); err != nil {
					return err
				}
				continue
			}
			size := header.UnPackedSize
			if header.UnKnownSize {
				size = -1
			}
			// Drop WriterTo so copying stays interruptible inside solid entries.
			source := struct{ io.Reader }{reader}
			written, err := session.writeArchiveFile(paths[i], source, size, nil, archiveFileProgress(completed, total, progress))
			if err != nil {
				var readErr archiveMemberReadError
				if errors.As(err, &readErr) {
					err = rarReadError(err, header.Encrypted)
				}
				return fmt.Errorf("%s · %w", header.Name, err)
			}
			completed += uint64(written)
		}
		if _, err := reader.Next(); err != io.EOF {
			if err != nil {
				return rarReadError(err, encryptedHeaders)
			}
			return errors.New("RAR 压缩包目录在解压过程中发生变化")
		}
		return nil
	})
}

func rarDestination(dest string, entry *rardecode.FileHeader) (string, error) {
	special := entry.LinkType != rardecode.LinkTypeNone
	if entry.HostOS == rardecode.HostOSWindows || entry.HostOS == rardecode.HostOSMSDOS {
		special = special || entry.Attributes&reparsePoint != 0
	}
	if entry.HostOS == rardecode.HostOSUnix || entry.HostOS == rardecode.HostOSMacOS {
		// rardecode exposes symlinks in Mode, but not Unix devices or FIFOs.
		kind := entry.Attributes & 0xf000
		special = special || (kind != 0 && kind != 0x4000 && kind != 0x8000)
	}
	return archiveDestination(dest, entry.Name, entry.Mode(), special)
}

func rarReadError(err error, encrypted bool) error {
	var pathErr *os.PathError
	switch {
	case errors.Is(err, rardecode.ErrBadPassword), errors.Is(err, rardecode.ErrArchiveEncrypted), errors.Is(err, rardecode.ErrArchivedFileEncrypted):
		return errors.Join(errArchivePassword, err)
	case encrypted && !errors.As(err, &pathErr) && !errors.Is(err, context.Canceled) && !errors.Is(err, context.DeadlineExceeded):
		// Older RAR variants do not always authenticate the password separately.
		return errors.Join(errArchivePassword, err)
	default:
		return err
	}
}

func (session *unpackSession) tryArchivePasswords(layer nestedEvent, dest string, attempt func(string) error) error {
	return session.tryPasswords(layer, errArchivePassword, true, func(password string) error {
		err := attempt(password)
		if errors.Is(err, errArchivePassword) {
			if cleanupErr := clearArchiveStage(dest); cleanupErr != nil {
				return fmt.Errorf("清理本次解压的文件失败 · %w", cleanupErr)
			}
		}
		return err
	})
}

func clearArchiveStage(dest string) error {
	info, err := os.Lstat(dest)
	if err != nil {
		return err
	}
	attrs, _ := info.Sys().(*syscall.Win32FileAttributeData)
	if !info.IsDir() || attrs == nil || attrs.FileAttributes&syscall.FILE_ATTRIBUTE_REPARSE_POINT != 0 {
		return errors.New("解压暂存位置必须是普通目录")
	}
	entries, err := os.ReadDir(dest)
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

func archiveDestination(dest, name string, mode fs.FileMode, special bool) (string, error) {
	relative, err := cleanArchivePath(name)
	if err != nil {
		return "", err
	}
	if special || (mode.Type() != fs.ModeDir && !mode.IsRegular()) {
		return "", fmt.Errorf("不支持的压缩包成员 %q", name)
	}
	return filepath.Join(dest, relative), nil
}

func (session *unpackSession) startArchiveProgress(layer nestedEvent, progress func(int), total uint64) {
	if total == 0 {
		session.stage(layer, "解压当前压缩包", -1)
	} else {
		progress(0)
	}
}

func archiveFileProgress(completed, total uint64, progress func(int)) func(uint64) {
	return func(written uint64) {
		if total != 0 {
			progress(min(99, int(float64(completed+written)*100/float64(total))))
		}
	}
}

type archiveMemberReadError struct{ error }

func (err archiveMemberReadError) Unwrap() error { return err.error }

func (session *unpackSession) writeArchiveFile(path string, input io.Reader, size int64, checksum io.Writer, observe func(uint64)) (int64, error) {
	if err := os.MkdirAll(filepath.Dir(path), 0700); err != nil {
		return 0, err
	}
	output, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0600)
	if err != nil {
		return 0, err
	}
	var destination io.Writer = &progressWriter{ctx: session.ctx, file: output, observe: observe}
	if checksum != nil {
		destination = io.MultiWriter(destination, checksum)
	}
	written, copyErr := copyArchiveData(destination, input, size, session.archiveCopyBuffer())
	return written, errors.Join(copyErr, output.Close())
}

func copyArchiveData(destination io.Writer, input io.Reader, size int64, buffer []byte) (int64, error) {
	var written int64
	for {
		block := buffer
		if size >= 0 && size-written < int64(len(block)) {
			// Probe for one byte over the advertised size without writing it.
			block = block[:size-written+1]
		}
		n, readErr := input.Read(block)
		if size >= 0 && int64(n) > size-written {
			return written, archiveMemberReadError{zip.ErrFormat}
		}
		if n > 0 {
			copied, err := destination.Write(block[:n])
			written += int64(copied)
			if err != nil {
				return written, err
			}
			if copied != n {
				return written, io.ErrShortWrite
			}
		}
		if readErr == io.EOF {
			if size >= 0 && written != size {
				return written, archiveMemberReadError{io.ErrUnexpectedEOF}
			}
			return written, nil
		}
		if readErr != nil {
			return written, archiveMemberReadError{readErr}
		}
	}
}

func (session *unpackSession) archiveCopyBuffer() []byte {
	if session.copyBuffer == nil {
		session.copyBuffer = make([]byte, 4<<20)
	}
	return session.copyBuffer
}
