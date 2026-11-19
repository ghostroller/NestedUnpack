package main

import (
	"bufio"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"syscall"
)

type nestedEvent struct {
	Kind, Source, Archive, Format, Stage, Message, Output, Status string
	Index, Total, Depth, Percent                                  int
	ArchiveIndex, ArchivesDiscovered, ArchivesCompleted           int
	MaxDepth, Parts, FilesWritten                                 int
}

type passwordRequest struct {
	Source, Archive, Format string
	Depth                   int
	Retry                   bool
}

type passwordReply struct {
	Value    string
	Remember bool
}

type nestedUI interface {
	Update(nestedEvent)
	RequestPassword(context.Context, passwordRequest) (passwordReply, error)
}

type batchSummary struct {
	Total, Success, Skipped, Failed int
	Canceled                        bool
}

func (summary batchSummary) exitCode() int {
	if summary.Canceled {
		return 2
	}
	if summary.Failed > 0 {
		return 1
	}
	return 0
}

type unpackSession struct {
	ctx            context.Context
	ui             nestedUI
	sevenZip       string
	recentPassword string
	book           *passwordBook
	copyBuffer     []byte
	stageVersion   uint64
}

func (session *unpackSession) stage(event nestedEvent, label string, percent int) {
	session.stageVersion++
	event.Kind, event.Stage, event.Percent = "stage", label, percent
	session.ui.Update(event)
}

func (session *unpackSession) sourceStage(event nestedEvent, label string) {
	event.ArchiveIndex = 0
	session.stage(event, label, -1)
}

func (session *unpackSession) progress(event nestedEvent, label string) func(int) {
	event.Kind, event.Stage = "progress", label
	last := -1
	version := session.stageVersion
	return func(percent int) {
		if percent != last || version != session.stageVersion {
			event.Percent = percent
			session.ui.Update(event)
			last, version = percent, session.stageVersion
		}
	}
}

func (session *unpackSession) tryPasswords(layer nestedEvent, wrongPassword error, tryUnencrypted bool, attempt func(string) error) error {
	if err := session.ctx.Err(); err != nil {
		return err
	}
	if tryUnencrypted {
		if err := attempt(""); !errors.Is(err, wrongPassword) {
			return err
		}
	}

	var candidates []string
	if session.recentPassword != "" {
		candidates = append(candidates, session.recentPassword)
	}
	if session.book != nil {
		for _, password := range session.book.Passwords() {
			if password != session.recentPassword {
				candidates = append(candidates, password)
			}
		}
	}
	for index, password := range candidates {
		if err := session.ctx.Err(); err != nil {
			return err
		}
		session.stage(layer, fmt.Sprintf("尝试已知密码（%d/%d）", index+1, len(candidates)), -1)
		err := attempt(password)
		if err == nil {
			session.recentPassword = password
		}
		if !errors.Is(err, wrongPassword) {
			return err
		}
	}

	request := passwordRequest{Source: layer.Source, Archive: layer.Archive, Format: layer.Format, Depth: layer.Depth}
	for {
		if err := session.ctx.Err(); err != nil {
			return err
		}
		session.stage(layer, "等待输入密码", -1)
		reply, err := session.ui.RequestPassword(session.ctx, request)
		if err != nil {
			return err
		}
		if reply.Value == "" {
			return errPasswordSkipped
		}
		session.stage(layer, "验证输入的密码", -1)
		err = attempt(reply.Value)
		if errors.Is(err, wrongPassword) {
			request.Retry = true
			continue
		}
		if err != nil {
			return err
		}
		session.recentPassword = reply.Value
		if reply.Remember {
			var saveErr error
			if session.book == nil {
				saveErr = errors.New("密码簿不可用")
			} else {
				saveErr = session.book.Remember(reply.Value)
			}
			if saveErr != nil {
				layer.Kind, layer.Message = "warning", "密码未能保存 · "+saveErr.Error()
				session.ui.Update(layer)
			}
		}
		return nil
	}
}

func (session *unpackSession) unpackUsing7Zip(layer nestedEvent, dest, work string, progress func(int)) error {
	if session.sevenZip == "" {
		var err error
		session.sevenZip, err = find7Zip()
		if err != nil {
			return err
		}
	}
	archive := layer.Archive
	if layer.Format == "ZIP" {
		var err error
		archive, err = session.prepareEncryptedZip(layer, work)
		if err != nil {
			return err
		}
		if archive != layer.Archive {
			defer os.Remove(archive)
		}
	}
	if layer.Format == "ZIP" {
		session.stage(layer, "读取压缩包目录", -1)
	}
	return unpackWith7Zip(session.ctx, session.sevenZip, archive, dest, func(attempt func(string) error) error {
		return session.tryPasswords(layer, err7ZipPassword, true, attempt)
	}, progress)
}

func (session *unpackSession) unpack(item *nestedEvent, kind string) (string, error) {
	item.ArchivesDiscovered = 1
	work, err := os.MkdirTemp(filepath.Dir(item.Source), ".nestedunpack-nested-")
	if err != nil {
		return "", err
	}
	defer os.RemoveAll(work)

	root := nestedArchive{Path: item.Source, Kind: kind}
	result, err := session.unpackTree(item, work, root, 1)
	if err != nil {
		return "", err
	}
	if err := session.ctx.Err(); err != nil {
		return "", err
	}
	session.sourceStage(*item, "写入解压结果")
	output, err := publishDirectory(session.ctx, result.path, item.Source)
	if err != nil {
		return "", fmt.Errorf("写出解压目录 · %w", err)
	}
	item.FilesWritten = result.files
	return output, nil
}

type unpackedDirectory struct {
	path  string
	files int
}

func (session *unpackSession) unpackTree(item *nestedEvent, work string, archive nestedArchive, depth int) (unpackedDirectory, error) {
	var previous string
	for ; depth <= 64; depth++ {
		if err := session.ctx.Err(); err != nil {
			return unpackedDirectory{}, err
		}
		item.ArchiveIndex++
		item.MaxDepth = max(item.MaxDepth, depth)
		layer := *item
		layer.Archive, layer.Format, layer.Depth = archive.Path, archive.Kind, depth
		layer.Parts = max(1, len(archive.Parts))
		session.stage(layer, "读取压缩包目录", -1)
		dest, err := os.MkdirTemp(work, "layer-")
		if err != nil {
			return unpackedDirectory{}, layerError(layer, err)
		}
		progress := session.progress(layer, "解压当前压缩包")
		if archive.Kind == "ZIP" {
			err = session.unpackZipLayer(layer, dest, progress)
		}
		if archive.Kind != "ZIP" || errors.Is(err, errZipNeeds7Zip) {
			err = session.unpackUsing7Zip(layer, dest, work, progress)
		}
		if err != nil {
			return unpackedDirectory{}, layerError(layer, err)
		}
		session.stage(layer, "检查解出的文件并查找内层压缩包", -1)
		files, err := extractedFiles(dest)
		if err != nil {
			return unpackedDirectory{}, layerError(layer, err)
		}
		if previous != "" {
			if err := os.RemoveAll(previous); err != nil {
				return unpackedDirectory{}, layerError(layer, fmt.Errorf("清理上一层暂存目录 · %w", err))
			}
		}
		item.ArchivesCompleted++
		archives, err := findNestedArchives(files)
		if err != nil {
			return unpackedDirectory{}, layerError(layer, err)
		}
		item.ArchivesDiscovered += len(archives)
		if len(archives) == 1 && len(archives[0].Parts) == len(files) {
			archive, previous = archives[0], dest
			continue
		}
		finalFiles := len(files)
		for _, child := range archives {
			result, err := session.unpackTree(item, work, child, depth+1)
			if err != nil {
				return unpackedDirectory{}, err
			}
			if err := session.ctx.Err(); err != nil {
				return unpackedDirectory{}, err
			}
			session.sourceStage(*item, "整理已解出的内层文件")
			outputName := child.Path
			if child.Kind == "7z 分卷" {
				if base, _, ok := splitVolumeName(child.Path); ok && strings.EqualFold(filepath.Ext(base), ".7z") {
					outputName = base
				}
			}
			if _, err := publishDirectory(session.ctx, result.path, outputName); err != nil {
				return unpackedDirectory{}, fmt.Errorf("写出内层压缩包 %q · %w", filepath.Base(child.Path), err)
			}
			for _, part := range child.Parts {
				if err := os.Remove(part); err != nil {
					return unpackedDirectory{}, fmt.Errorf("清理已解开的暂存压缩包 · %w", err)
				}
			}
			finalFiles += result.files - len(child.Parts)
		}
		return unpackedDirectory{path: dest, files: finalFiles}, nil
	}
	return unpackedDirectory{}, errors.New("嵌套层数超过 64")
}

func layerError(layer nestedEvent, err error) error {
	return fmt.Errorf("第 %d 层 %s · %w", layer.Depth, filepath.Base(layer.Archive), err)
}

func executeNested(ctx context.Context, paths []string, ui nestedUI) batchSummary {
	book, err := loadPasswordBook()
	if err != nil {
		ui.Update(nestedEvent{Kind: "warning", Message: "无法读取密码簿 · " + err.Error()})
	}
	session := unpackSession{ctx: ctx, ui: ui, book: book}
	type job struct {
		path string
		err  error
	}
	jobs := make([]job, 0, len(paths))
	seen := make(map[string]bool)
	for _, selected := range paths {
		path, err := filepath.Abs(selected)
		if err != nil {
			jobs = append(jobs, job{selected, err})
			continue
		}
		key := strings.ToLower(path)
		if !seen[key] {
			seen[key] = true
			jobs = append(jobs, job{path, nil})
		}
	}

	summary := batchSummary{Total: len(jobs)}
	for index, job := range jobs {
		if ctx.Err() != nil {
			summary.Canceled = true
			break
		}
		item := nestedEvent{Kind: "source", Source: job.path, Index: index + 1, Total: len(jobs), Percent: -1}
		ui.Update(item)
		err := job.err
		var kind, output string
		if err == nil {
			kind, err = detectArchive(job.path)
		}
		if err == nil && kind != "" {
			item.Format = kind
			output, err = session.unpack(&item, kind)
		}

		item.Kind, item.Output = "result", output
		switch {
		case err != nil && ctx.Err() != nil:
			summary.Canceled = true
			item.Status, item.Message = "canceled", "已停止处理当前文件"
		case errors.Is(err, errPasswordSkipped):
			summary.Skipped++
			item.Status, item.Message = "skipped", err.Error()
		case err != nil:
			summary.Failed++
			item.Status, item.Message = "failed", err.Error()
		case kind == "":
			summary.Skipped++
			item.Status, item.Message = "skipped", "无法识别为 ZIP、RAR 或 7z"
		default:
			summary.Success++
			item.Status = "success"
		}
		ui.Update(item)
		if summary.Canceled {
			break
		}
	}
	return summary
}

type consoleNestedUI struct {
	input       *bufio.Reader
	showingLine bool
	archive     int
}

func (ui *consoleNestedUI) endLine() {
	if ui.showingLine {
		fmt.Println()
		ui.showingLine = false
	}
}

func (ui *consoleNestedUI) Update(event nestedEvent) {
	switch event.Kind {
	case "source":
		ui.endLine()
		ui.archive = 0
		fmt.Printf("\n文件 %d/%d · %s\n", event.Index, event.Total, event.Source)
	case "stage":
		ui.endLine()
		if event.ArchiveIndex > 0 && event.ArchiveIndex != ui.archive {
			ui.archive = event.ArchiveIndex
			fmt.Printf("第 %d 层 · 本文件第 %d 个压缩包 · %s · %s",
				event.Depth, event.ArchiveIndex, event.Format, filepath.Base(event.Archive))
			if event.Parts > 1 {
				fmt.Printf("（%d 个分卷）", event.Parts)
			}
			fmt.Println()
		}
		fmt.Println(event.Stage)
	case "progress":
		fmt.Printf("\r%s %3d%%", event.Stage, event.Percent)
		ui.showingLine = true
	case "warning":
		ui.endLine()
		fmt.Println(event.Message)
	case "result":
		ui.endLine()
		switch event.Status {
		case "success":
			fmt.Printf("解压完成 · %d 个压缩包 · 最深 %d 层 · %d 个文件\n",
				event.ArchivesCompleted, event.MaxDepth, event.FilesWritten)
			fmt.Println("已解压到", event.Output)
		case "skipped":
			fmt.Println("跳过 ·", event.Message)
		case "failed":
			fmt.Println("失败 ·", event.Message)
		case "canceled":
			fmt.Println("已取消 ·", event.Message)
		}
	}
}

func (ui *consoleNestedUI) RequestPassword(_ context.Context, request passwordRequest) (passwordReply, error) {
	ui.endLine()
	if request.Retry {
		fmt.Println("密码校验失败，请重试。")
	}
	fmt.Print("\n请输入压缩包密码（留空则跳过这个源文件）：")
	var mode uint32
	handle := syscall.Handle(os.Stdin.Fd())
	if syscall.GetConsoleMode(handle, &mode) == nil {
		setMode := syscall.NewLazyDLL("kernel32.dll").NewProc("SetConsoleMode")
		setMode.Call(uintptr(handle), uintptr(mode&^4)) // ENABLE_ECHO_INPUT
		defer setMode.Call(uintptr(handle), uintptr(mode))
	}
	line, err := ui.input.ReadString('\n')
	fmt.Println()
	if err != nil && !errors.Is(err, io.EOF) {
		return passwordReply{}, err
	}
	return passwordReply{Value: strings.TrimRight(line, "\r\n")}, nil
}

func runNested(paths []string) int {
	ui := &consoleNestedUI{input: bufio.NewReader(os.Stdin)}
	summary := executeNested(context.Background(), paths, ui)
	ui.endLine()
	fmt.Printf("\n处理结束 · 共 %d 个文件 · 成功 %d 个 · 跳过 %d 个 · 失败 %d 个\n",
		summary.Total, summary.Success, summary.Skipped, summary.Failed)
	return summary.exitCode()
}
