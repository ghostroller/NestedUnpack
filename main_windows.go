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
	if len(args) == 0 {
		return runLauncherGUI()
	}
	switch args[0] {
	case "--explorer-server":
		if len(args) == 2 && (strings.EqualFold(args[1], "-Embedding") || strings.EqualFold(args[1], "/Embedding")) {
			return runExplorerCommandServer()
		}
		return 2
	case "--nested-selection-file", "--selection-file":
		if len(args) != 2 {
			guiNotify("无法读取资源管理器选区。", true)
			return 2
		}
		paths, err := readSelection(args[1])
		if err != nil {
			guiNotify("无法读取资源管理器选区。\n"+err.Error(), true)
			return 2
		}
		if len(paths) == 0 {
			guiNotify("选区内没有可处理的文件。", true)
			return 2
		}
		if args[0] == "--selection-file" {
			return runOuter(paths, true)
		}
		return runNestedGUI(paths)
	case "--nested", "--outer":
		connectConsole()
		if len(args) == 1 {
			fmt.Fprintln(os.Stderr, "用法: NestedUnpack.exe [--nested | --outer] <文件> [更多文件...]")
			return 2
		}
		if args[0] == "--outer" {
			return runOuter(args[1:], false)
		}
		return runNested(args[1:])
	default:
		connectConsole()
		return runOuter(args, false)
	}
}

// The GUI subsystem starts without a console for double-click and drag-and-drop.
// Explicit console modes can use the parent's terminal or inherited pipes.
func connectConsole() {
	attach := syscall.NewLazyDLL("kernel32.dll").NewProc("AttachConsole")
	attach.Call(0xffffffff) // ATTACH_PARENT_PROCESS
	for _, stream := range []struct {
		file   **os.File
		handle int
		name   string
	}{
		{&os.Stdin, syscall.STD_INPUT_HANDLE, "stdin"},
		{&os.Stdout, syscall.STD_OUTPUT_HANDLE, "stdout"},
		{&os.Stderr, syscall.STD_ERROR_HANDLE, "stderr"},
	} {
		if current := *stream.file; current != nil {
			if handle := syscall.Handle(current.Fd()); handle != 0 && handle != syscall.InvalidHandle {
				continue
			}
		}
		handle, err := syscall.GetStdHandle(stream.handle)
		if err == nil && handle != 0 && handle != syscall.InvalidHandle {
			*stream.file = os.NewFile(uintptr(handle), stream.name)
		}
	}
}

func runOuter(args []string, fromExplorer bool) int {
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
	if !strings.EqualFold(filepath.Dir(absolute), temp) || !strings.HasPrefix(name, "NestedUnpack-Selection-") || !strings.HasSuffix(name, ".utf8") {
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
