package main

import (
	"archive/zip"
	"bufio"
	"compress/bzip2"
	"compress/flate"
	"crypto/aes"
	"crypto/cipher"
	"crypto/hmac"
	"crypto/pbkdf2"
	"crypto/sha1"
	"crypto/subtle"
	"errors"
	"hash/crc32"
	"io"
)

const (
	zipAESMethod   = 99
	zipAESExtra    = 0x9901
	zipAESAuthSize = 10
	zipBZip2       = 12
)

type zipAESFormat struct {
	version, method uint16
	keySize         int
}

func zipAESParameters(entry *zip.File) (zipAESFormat, error) {
	const unsupportedFlags = 1<<5 | 1<<6 | 1<<13 // patched data, PKWARE strong encryption or masked headers
	if entry.Method != zipAESMethod || entry.Flags&unsupportedFlags != 0 {
		return zipAESFormat{}, zip.ErrAlgorithm
	}
	for extra := entry.Extra; len(extra) >= 4; {
		id, length := le.Uint16(extra), int(le.Uint16(extra[2:]))
		if length > len(extra)-4 {
			return zipAESFormat{}, zip.ErrFormat
		}
		data := extra[4 : 4+length]
		extra = extra[4+length:]
		if id != zipAESExtra {
			continue
		}
		if length < 7 || string(data[2:4]) != "AE" {
			return zipAESFormat{}, zip.ErrFormat
		}
		format := zipAESFormat{version: le.Uint16(data), method: le.Uint16(data[5:])}
		if format.version != 1 && format.version != 2 || data[4] < 1 || data[4] > 3 {
			return zipAESFormat{}, zip.ErrAlgorithm
		}
		switch format.method {
		case zip.Store, zip.Deflate, zipBZip2:
		default:
			return zipAESFormat{}, zip.ErrAlgorithm
		}
		format.keySize = 8 + int(data[4])*8
		return format, nil
	}
	return zipAESFormat{}, zip.ErrFormat
}

// WinZip AE-1 and AE-2 use PBKDF2-SHA1, AES-CTR and a truncated HMAC-SHA1.
// OpenRaw can read the encrypted member through the existing ZIP64/MP4 view.
func (session *unpackSession) copyZipAES(entry *zip.File, output io.Writer, password string) error {
	format, err := zipAESParameters(entry)
	if err != nil {
		return err
	}
	saltSize := format.keySize / 2
	headerSize := saltSize + 2
	overhead := uint64(headerSize + zipAESAuthSize)
	if entry.CompressedSize64 < overhead {
		return zip.ErrFormat
	}
	ciphertextSize := entry.CompressedSize64 - overhead
	if format.method == zip.Store && ciphertextSize != entry.UncompressedSize64 {
		return zip.ErrFormat
	}
	raw, err := entry.OpenRaw()
	if err != nil {
		return err
	}
	var header [18]byte
	if _, err := io.ReadFull(raw, header[:headerSize]); err != nil {
		return err
	}
	key, err := pbkdf2.Key(sha1.New, password, header[:saltSize], 1000, 2*format.keySize+2)
	if err != nil {
		return err
	}
	defer clear(key)
	if subtle.ConstantTimeCompare(key[2*format.keySize:], header[saltSize:headerSize]) != 1 {
		return errArchivePassword
	}
	cipherBlock, err := aes.NewCipher(key[:format.keySize])
	if err != nil {
		return err
	}
	mac := hmac.New(sha1.New, key[format.keySize:2*format.keySize])
	payload := &io.LimitedReader{R: raw, N: int64(ciphertextSize)}
	authenticated := io.TeeReader(payload, mac)
	ctr := &zipAESCounter{block: cipherBlock, used: aes.BlockSize}
	decrypted := cipher.StreamReader{S: ctr, R: authenticated}
	var reader io.Reader = decrypted
	var buffered *bufio.Reader
	switch format.method {
	case zip.Deflate:
		buffered = bufio.NewReaderSize(decrypted, 64<<10)
		inflated := flate.NewReader(buffered)
		defer inflated.Close()
		reader = inflated
	case zipBZip2:
		buffered = bufio.NewReaderSize(decrypted, 64<<10)
		reader = bzip2.NewReader(buffered)
	}
	checksum := crc32.NewIEEE()
	_, copyErr := copyArchiveData(io.MultiWriter(output, checksum), reader, int64(entry.UncompressedSize64), session.archiveCopyBuffer())
	var readErr archiveMemberReadError
	if copyErr != nil && !errors.As(copyErr, &readErr) {
		return copyErr
	}
	extraData := payload.N != 0 || (buffered != nil && buffered.Buffered() != 0)
	// A wrong key can pass the two-byte verifier. Authenticate the entire
	// ciphertext before deciding whether a decompressor error is a bad password.
	buffer := session.archiveCopyBuffer()
	for payload.N > 0 {
		if err := session.ctx.Err(); err != nil {
			return err
		}
		if _, err := io.ReadFull(authenticated, buffer[:min(int64(len(buffer)), payload.N)]); err != nil {
			return err
		}
	}
	var tag [zipAESAuthSize]byte
	if _, err := io.ReadFull(raw, tag[:]); err != nil {
		return err
	}
	if !hmac.Equal(mac.Sum(nil)[:zipAESAuthSize], tag[:]) {
		return errArchivePassword
	}
	if copyErr != nil {
		return copyErr
	}
	if extraData {
		return zip.ErrFormat
	}
	if format.version == 1 && checksum.Sum32() != entry.CRC32 {
		return zip.ErrChecksum
	}
	return nil
}

// WinZip counts up from 1 in little-endian order, whereas cipher.NewCTR
// increments a big-endian counter. Preserve a partial block across reads.
type zipAESCounter struct {
	block   cipher.Block
	counter [aes.BlockSize]byte
	key     [aes.BlockSize]byte
	used    int
}

func (stream *zipAESCounter) XORKeyStream(dst, src []byte) {
	for len(src) > 0 {
		if stream.used == aes.BlockSize {
			for i := range stream.counter {
				stream.counter[i]++
				if stream.counter[i] != 0 {
					break
				}
			}
			stream.block.Encrypt(stream.key[:], stream.counter[:])
			stream.used = 0
		}
		n := subtle.XORBytes(dst, src, stream.key[stream.used:])
		stream.used += n
		dst, src = dst[n:], src[n:]
	}
}
