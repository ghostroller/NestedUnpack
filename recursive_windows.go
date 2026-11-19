package main

import (
	"bytes"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"slices"
	"strconv"
	"strings"
)

type nestedArchive struct {
	Path, Kind string
	Parts      []string
}

type numberedVolume struct {
	path  string
	index int
}

func splitVolumeName(path string) (base string, index int, ok bool) {
	extension := filepath.Ext(path)
	if len(extension) < 4 {
		return "", 0, false
	}
	index, err := strconv.Atoi(extension[1:])
	if err != nil || index < 1 || fmt.Sprintf(".%03d", index) != extension {
		return "", 0, false
	}
	return strings.TrimSuffix(path, extension), index, true
}

func findNestedArchives(files []string) ([]nestedArchive, error) {
	groups := make(map[string][]numberedVolume)
	for _, path := range files {
		if base, index, ok := splitVolumeName(path); ok {
			key := strings.ToLower(base)
			groups[key] = append(groups[key], numberedVolume{path: path, index: index})
		}
	}

	var archives []nestedArchive
	consumed := make(map[string]bool)
	for _, path := range files {
		if consumed[path] {
			continue
		}
		if base, _, ok := splitVolumeName(path); ok {
			key := strings.ToLower(base)
			if group := groups[key]; len(group) > 0 {
				delete(groups, key)
				archive, err := find7ZipVolumes(base, group)
				if err != nil {
					return nil, err
				}
				if archive.Path != "" {
					archives = append(archives, archive)
					for _, part := range archive.Parts {
						consumed[part] = true
					}
					continue
				}
			}
		}
		kind, err := findNestedArchiveKind(path, len(files) == 1)
		if err != nil {
			return nil, err
		}
		if kind != "" {
			archives = append(archives, nestedArchive{Path: path, Kind: kind, Parts: []string{path}})
		}
	}
	return archives, nil
}

func find7ZipVolumes(base string, volumes []numberedVolume) (nestedArchive, error) {
	slices.SortFunc(volumes, func(a, b numberedVolume) int { return a.index - b.index })
	first := volumes[0]
	expected7z := strings.EqualFold(filepath.Ext(base), ".7z")
	if first.index != 1 {
		if expected7z {
			return nestedArchive{}, fmt.Errorf("缺少 7z 分卷 %q", filepath.Base(base)+".001")
		}
		return nestedArchive{}, nil
	}
	kind, err := detectArchive(first.path)
	if err != nil {
		return nestedArchive{}, err
	}
	if kind != "7z" {
		if expected7z {
			return nestedArchive{}, fmt.Errorf("无法识别 7z 首分卷 %q", filepath.Base(first.path))
		}
		return nestedArchive{}, nil
	}
	parts := make([]string, 0, len(volumes))
	for i, volume := range volumes {
		if volume.index != i+1 {
			return nestedArchive{}, fmt.Errorf("7z 分卷不连续，预期 %q", fmt.Sprintf("%s.%03d", filepath.Base(base), i+1))
		}
		parts = append(parts, volume.path)
	}
	return nestedArchive{Path: first.path, Kind: "7z 分卷", Parts: parts}, nil
}

func findNestedArchiveKind(path string, onlyFile bool) (string, error) {
	switch strings.ToLower(filepath.Ext(path)) {
	case ".epub", ".cbz", ".kmz", ".apk", ".jar",
		".docx", ".docm", ".dotx", ".dotm",
		".xlsx", ".xlsm", ".xlsb", ".xltx", ".xltm", ".xlam",
		".pptx", ".pptm", ".potx", ".potm", ".ppsx", ".ppsm", ".ppam",
		".odt", ".ods", ".odp", ".odg", ".odf", ".ott", ".ots", ".otp", ".otg",
		".vsdx", ".3mf":
		// These file formats use ZIP as their document container.
		return "", nil
	case ".zip", ".zipx", ".7z", ".rar", ".mp4":
		return detectArchive(path)
	}
	if onlyFile {
		return detectArchive(path)
	}

	// In a mixed directory, only read the ZIP trailer of plausible archives.
	// Disguised standalone archives can still be recognized by their header.
	file, err := os.Open(path)
	if err != nil {
		return "", err
	}
	var buffer [8]byte
	n, err := file.ReadAt(buffer[:], 0)
	file.Close()
	if err != nil && !errors.Is(err, io.EOF) {
		return "", err
	}
	header := buffer[:n]
	if bytes.HasPrefix(header, []byte("PK\x03\x04")) ||
		bytes.HasPrefix(header, []byte("PK\x05\x06")) ||
		bytes.HasPrefix(header, []byte("PK\x06\x06")) ||
		bytes.HasPrefix(header, []byte("Rar!")) ||
		bytes.HasPrefix(header, []byte("7z\xbc\xaf\x27\x1c")) {
		return detectArchive(path)
	}
	return "", nil
}
