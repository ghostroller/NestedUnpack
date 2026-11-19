package main

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"unicode/utf8"
	"unsafe"
)

type batch struct {
	success, skipped, failed int
	outputs, notes           []string
}

func (result *batch) note(value string) {
	fmt.Println(value)
	result.notes = append(result.notes, value)
}

func (result *batch) process(source string) {
	file, err := os.Open(source)
	if err != nil {
		result.failed++
		result.note(fmt.Sprintf("失败 %s · %s", filepath.Base(source), err))
		return
	}
	defer file.Close()
	info, err := file.Stat()
	if err != nil || !info.Mode().IsRegular() {
		result.failed++
		result.note(fmt.Sprintf("失败 %s · 无法读取普通文件", filepath.Base(source)))
		return
	}
	archive, err := openOuter(file, info.Size())
	if errors.Is(err, errNoZip) {
		result.skipped++
		result.note(fmt.Sprintf("跳过 %s · 未识别到 ZIP", filepath.Base(source)))
		return
	}
	if err != nil {
		result.failed++
		result.note(fmt.Sprintf("失败 %s · %s", filepath.Base(source), err))
		return
	}
	entries := rarEntries(archive)
	if len(entries) == 0 {
		result.skipped++
		result.note(fmt.Sprintf("跳过 %s · ZIP 中没有 RAR 成员", filepath.Base(source)))
		return
	}
	failed := false
	for _, entry := range entries {
		path, err := extractRAR(entry, source)
		if err != nil {
			failed = true
			result.note(fmt.Sprintf("失败 %s · %s · %s", filepath.Base(source), entry.Name, err))
			continue
		}
		fmt.Println("已解出", path)
		result.outputs = append(result.outputs, filepath.Base(path))
	}
	if failed {
		result.failed++
	} else {
		result.success++
	}
}

func main() { os.Exit(run(os.Args[1:])) }

func run(args []string) int {
	nested := len(args) > 0 && (args[0] == "--nested" || args[0] == "--nested-selection-file")
	fromExplorer := len(args) > 0 && (args[0] == "--selection-file" || args[0] == "--nested-selection-file")
	if fromExplorer {
		if len(args) != 2 {
			notify("无法读取资源管理器选区。", true)
			return 2
		}
		var err error
		args, err = readSelection(args[1])
		if err != nil {
			notify("无法读取资源管理器选区。\n"+err.Error(), true)
			return 2
		}
	} else if nested {
		args = args[1:]
	}
	if len(args) == 0 {
		fmt.Fprintln(os.Stderr, "用法: OuterRar.exe [--nested] <文件> [更多文件...]")
		if fromExplorer {
			notify("选区内没有可处理的文件。", true)
		}
		return 2
	}
	if nested {
		return runNested(args, fromExplorer)
	}
	var result batch
	seen := make(map[string]bool)
	for _, selected := range args {
		path, err := filepath.Abs(selected)
		if err != nil {
			result.failed++
			result.note(fmt.Sprintf("失败 %s · %s", selected, err))
			continue
		}
		key := strings.ToLower(path)
		if !seen[key] {
			seen[key] = true
			result.process(path)
		}
	}
	summary := fmt.Sprintf("完成 · 成功 %d 个 · 跳过 %d 个 · 失败 %d 个\n共写出 %d 个 RAR", result.success, result.skipped, result.failed, len(result.outputs))
	fmt.Println(summary)
	if fromExplorer {
		var details []string
		details = append(details, result.notes...)
		details = append(details, result.outputs...)
		if len(details) > 8 {
			details = append(details[:8], "……")
		}
		if len(details) > 0 {
			summary += "\n\n" + strings.Join(details, "\n")
		}
		notify(summary, result.failed > 0)
	}
	if result.failed > 0 {
		return 1
	}
	return 0
}

func readSelection(file string) ([]string, error) {
	name := filepath.Base(file)
	absolute, err := filepath.Abs(file)
	if err != nil {
		return nil, err
	}
	temp, err := filepath.Abs(os.TempDir())
	if err != nil {
		return nil, err
	}
	if !strings.EqualFold(filepath.Dir(absolute), temp) || !strings.HasPrefix(name, "OuterRar-Selection-") || !strings.HasSuffix(name, ".utf8") {
		return nil, errors.New("无效的选区清单路径")
	}
	defer os.Remove(absolute)
	data, err := os.ReadFile(absolute)
	if err != nil {
		return nil, err
	}
	if !utf8.Valid(data) {
		return nil, errors.New("选区清单编码无效")
	}
	paths := strings.Split(strings.TrimSuffix(string(data), "\n"), "\n")
	if len(paths) == 1 && paths[0] == "" {
		return nil, nil
	}
	for index, path := range paths {
		path = strings.TrimSuffix(path, "\r")
		if !filepath.IsAbs(path) {
			return nil, errors.New("选区清单含无效路径")
		}
		paths[index] = path
	}
	return paths, nil
}

func notify(message string, warning bool) {
	title, _ := syscall.UTF16PtrFromString("解出外层 RAR")
	text, err := syscall.UTF16PtrFromString(message)
	if err != nil {
		return
	}
	style := uintptr(0x00000040 | 0x00010000)
	if warning {
		style = 0x00000030 | 0x00010000
	}
	box := syscall.NewLazyDLL("user32.dll").NewProc("MessageBoxW")
	box.Call(0, uintptr(unsafe.Pointer(text)), uintptr(unsafe.Pointer(title)), style)
}
