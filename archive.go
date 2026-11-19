package main

import (
	"archive/zip"
	"bytes"
	"encoding/binary"
	"errors"
	"io"
	"os"
	"path"
	"strings"
)

const tailWindow = 1 << 20

var le = binary.LittleEndian

var errNoZip = errors.New("未识别到 ZIP")

func rarEntries(reader *zip.Reader) []*zip.File {
	var result []*zip.File
	for _, entry := range reader.File {
		name := strings.ReplaceAll(entry.Name, `\`, "/")
		if entry.Mode().IsRegular() && strings.EqualFold(path.Ext(name), ".rar") {
			result = append(result, entry)
		}
	}
	return result
}

func openOuter(file *os.File, size int64) (*zip.Reader, error) {
	direct, directErr := zip.NewReader(file, size)
	if direct != nil && (directErr == nil || errors.Is(directErr, zip.ErrInsecurePath)) && len(rarEntries(direct)) > 0 {
		return direct, nil
	}
	bounded, err := locateZip(file, size)
	if err == nil {
		return bounded.reader, nil
	}
	if direct != nil && (directErr == nil || errors.Is(directErr, zip.ErrInsecurePath)) {
		return direct, nil
	}
	return nil, err
}

// Some writers leave legacy ZIP32 counts in range even when they include a
// ZIP64 directory. Expose the ZIP64 marker to archive/zip without editing disk.
type zip64View struct {
	*io.SectionReader
	countOffset int64
}

func (view zip64View) ReadAt(buffer []byte, offset int64) (int, error) {
	n, err := view.SectionReader.ReadAt(buffer, offset)
	for index := view.countOffset; index < view.countOffset+4; index++ {
		if index >= offset && index-offset < int64(n) {
			buffer[index-offset] = 0xff
		}
	}
	return n, err
}

type zipSection struct {
	reader *zip.Reader
	start  int64
	length int64
}

func locateZip(file *os.File, size int64) (*zipSection, error) {
	const eocdSize = 22
	tail := make([]byte, min(size, tailWindow))
	tailStart := size - int64(len(tail))
	if _, err := file.ReadAt(tail, tailStart); err != nil {
		return nil, err
	}
	for limit := len(tail); limit >= 4; {
		endIndex := bytes.LastIndex(tail[:limit], []byte("PK\x05\x06"))
		if endIndex < 0 {
			break
		}
		limit = endIndex
		if endIndex+eocdSize > len(tail) {
			continue
		}
		end := tail[endIndex:]
		archiveEnd := tailStart + int64(endIndex+eocdSize) + int64(le.Uint16(end[20:]))
		if archiveEnd > size || le.Uint16(end[4:]) != 0 || le.Uint16(end[6:]) != 0 || le.Uint16(end[8:]) != le.Uint16(end[10:]) {
			continue
		}
		if endIndex >= 20 && bytes.Equal(tail[endIndex-20:endIndex-16], []byte("PK\x06\x07")) {
			if section := locateZip64(file, tail, tailStart, endIndex, archiveEnd); section != nil {
				return section, nil
			}
			continue
		}
		count := uint64(le.Uint16(end[10:]))
		directorySize, directoryOffset := uint64(le.Uint32(end[12:])), uint64(le.Uint32(end[16:]))
		footer := tailStart + int64(endIndex)
		if directoryOffset+directorySize > uint64(footer) {
			continue
		}
		start := footer - int64(directoryOffset+directorySize)
		length := archiveEnd - start
		if reader := readZipView(io.NewSectionReader(file, start, length), length, count); reader != nil {
			return &zipSection{reader, start, length}, nil
		}
	}
	return nil, errNoZip
}

func locateZip64(file *os.File, tail []byte, tailStart int64, endIndex int, archiveEnd int64) *zipSection {
	locatorIndex := endIndex - 20
	locator := tail[locatorIndex:endIndex]
	if le.Uint32(locator[4:]) != 0 || le.Uint32(locator[16:]) != 1 {
		return nil
	}
	locatorOffset := le.Uint64(locator[8:])
	for limit := locatorIndex; limit >= 4; {
		footerIndex := bytes.LastIndex(tail[:limit], []byte("PK\x06\x06"))
		if footerIndex < 0 {
			break
		}
		limit = footerIndex
		record := tail[footerIndex:locatorIndex]
		if len(record) < 56 || le.Uint64(record[4:]) != uint64(len(record)-12) || le.Uint32(record[16:]) != 0 || le.Uint32(record[20:]) != 0 {
			continue
		}
		footer := tailStart + int64(footerIndex)
		count := le.Uint64(record[32:])
		directorySize, directoryOffset := le.Uint64(record[40:]), le.Uint64(record[48:])
		if count != le.Uint64(record[24:]) || locatorOffset > uint64(footer) || directoryOffset > locatorOffset || directorySize != locatorOffset-directoryOffset {
			continue
		}
		start := footer - int64(locatorOffset)
		length := archiveEnd - start
		view := zip64View{io.NewSectionReader(file, start, length), tailStart + int64(endIndex+8) - start}
		if reader := readZipView(view, length, count); reader != nil {
			return &zipSection{reader, start, length}
		}
	}
	return nil
}

func readZipView(view io.ReaderAt, length int64, count uint64) *zip.Reader {
	reader, err := zip.NewReader(view, length)
	if reader != nil && (err == nil || errors.Is(err, zip.ErrInsecurePath)) && uint64(len(reader.File)) == count {
		return reader
	}
	return nil
}
