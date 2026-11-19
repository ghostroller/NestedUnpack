package main

import (
	"archive/zip"
	"bufio"
	"compress/flate"
	"errors"
	"hash/crc32"
	"io"
	"math"
)

const zipCryptoHeaderSize = 12

var errZipCryptoPassword = errors.New("ZIP 密码错误或加密数据已损坏")

func isZipCrypto(entry *zip.File) bool {
	const unsupportedFlags = 1<<5 | 1<<6 | 1<<13 // patched data, strong encryption or masked headers
	if entry.Flags&1 == 0 || entry.Flags&unsupportedFlags != 0 || (entry.Method != zip.Store && entry.Method != zip.Deflate) {
		return false
	}
	for extra := entry.Extra; len(extra) >= 4; {
		id, length := le.Uint16(extra), int(le.Uint16(extra[2:]))
		if id == 0x9901 || id == 0x0017 { // WinZip AES or PKWARE strong encryption
			return false
		}
		if length > len(extra)-4 {
			break
		}
		extra = extra[4+length:]
	}
	return true
}

func copyZipCrypto(entry *zip.File, output io.Writer, password string) error {
	if !isZipCrypto(entry) {
		return zip.ErrAlgorithm
	}
	if entry.CompressedSize64 < zipCryptoHeaderSize || entry.CompressedSize64 > math.MaxInt64 || entry.UncompressedSize64 > math.MaxInt64 {
		return zip.ErrFormat
	}
	payloadSize := entry.CompressedSize64 - zipCryptoHeaderSize
	if entry.Method == zip.Store && payloadSize != entry.UncompressedSize64 {
		return zip.ErrFormat
	}

	raw, err := entry.OpenRaw()
	if err != nil {
		return err
	}
	decrypted := &zipCryptoReader{
		raw: raw, remaining: entry.CompressedSize64,
		key0: 0x12345678, key1: 0x23456789, key2: 0x34567890,
	}
	for index := 0; index < len(password); index++ {
		decrypted.update(password[index])
	}
	var header [zipCryptoHeaderSize]byte
	if _, err := io.ReadFull(decrypted, header[:]); err != nil {
		return err
	}
	check := byte(entry.CRC32 >> 24)
	if entry.Flags&8 != 0 {
		check = byte(entry.ModifiedTime >> 8)
	}
	if header[len(header)-1] != check {
		return errZipCryptoPassword
	}

	var plain io.Reader = decrypted
	var buffered *bufio.Reader
	if entry.Method == zip.Deflate {
		// The flate reader can consume this ByteReader without hiding extra bytes.
		buffered = bufio.NewReaderSize(decrypted, 128<<10)
		inflated := flate.NewReader(buffered)
		defer inflated.Close()
		plain = inflated
	}
	if err := copyZipCryptoPayload(plain, decrypted, output, entry.UncompressedSize64, entry.CRC32); err != nil {
		return err
	}
	if decrypted.remaining != 0 || (buffered != nil && buffered.Buffered() != 0) {
		return zip.ErrFormat
	}
	return nil
}

func copyZipCryptoPayload(plain io.Reader, decrypted *zipCryptoReader, output io.Writer, remaining uint64, expectedCRC uint32) error {
	buffer := make([]byte, 256<<10)
	var checksum uint32
	for {
		// Read at most one byte past the advertised end to detect overruns.
		limit := min(uint64(len(buffer)), remaining+1)
		n, readErr := plain.Read(buffer[:limit])
		if decrypted.readErr != nil {
			return decrypted.readErr
		}
		if uint64(n) > remaining {
			return errZipCryptoPassword
		}
		if n > 0 {
			block := buffer[:n]
			written, err := output.Write(block)
			if err != nil {
				return err
			}
			if written != n {
				return io.ErrShortWrite
			}
			checksum = crc32.Update(checksum, crc32.IEEETable, block)
			remaining -= uint64(n)
		}
		if readErr == io.EOF {
			break
		}
		if readErr != nil {
			return errZipCryptoPassword
		}
	}
	// The header verifies one byte. A wrong key can pass by chance, then fail
	// to inflate or fail the full size and CRC checks here.
	if remaining != 0 || checksum != expectedCRC {
		return errZipCryptoPassword
	}
	return nil
}

type zipCryptoReader struct {
	raw              io.Reader
	key0, key1, key2 uint32
	remaining        uint64
	readErr          error
}

func (reader *zipCryptoReader) update(plain byte) {
	reader.key0 = (reader.key0 >> 8) ^ crc32.IEEETable[byte(reader.key0)^plain]
	reader.key1 = (reader.key1+uint32(byte(reader.key0)))*134775813 + 1
	reader.key2 = (reader.key2 >> 8) ^ crc32.IEEETable[byte(reader.key2)^byte(reader.key1>>24)]
}

func (reader *zipCryptoReader) Read(p []byte) (int, error) {
	if reader.remaining == 0 {
		return 0, io.EOF
	}
	n, err := reader.raw.Read(p[:min(uint64(len(p)), reader.remaining)])
	reader.remaining -= uint64(n)
	if err == io.EOF && reader.remaining != 0 {
		err = io.ErrUnexpectedEOF
	}
	if err != nil && err != io.EOF {
		reader.readErr = err
	}
	for index, value := range p[:n] {
		mask := uint32(uint16(reader.key2)) | 2
		plain := value ^ byte(mask*(mask^1)>>8)
		p[index] = plain
		reader.update(plain)
	}
	return n, err
}
