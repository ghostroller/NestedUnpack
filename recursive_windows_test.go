package main

import (
	"archive/zip"
	"bytes"
	"context"
	"fmt"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestRecursiveSplitArchiveWithDocument(t *testing.T) {
	sevenZip, err := find7Zip()
	if err != nil {
		t.Skip(err)
	}
	t.Setenv("LOCALAPPDATA", t.TempDir())
	work := t.TempDir()
	const password = "nestedunpack-regression-only"
	deepPayload := bytes.Repeat([]byte("nested media payload\n"), 600)
	inner := makeRecursiveTestZip(t, map[string][]byte{"media/detail.txt": deepPayload})
	if err := os.WriteFile(filepath.Join(work, "inner.zip"), inner, 0600); err != nil {
		t.Fatal(err)
	}
	command := exec.Command(sevenZip, "a", "-t7z", "-mx=0", "-mhe=on", "-p"+password, "-v8k", "10241.7z", "inner.zip")
	command.Dir = work
	if output, err := command.CombinedOutput(); err != nil {
		t.Fatalf("creating test volumes: %v\n%s", err, output)
	}
	volumes, err := filepath.Glob(filepath.Join(work, "10241.7z.*"))
	if err != nil || len(volumes) < 2 {
		t.Fatalf("creating %d test volumes: %v", len(volumes), err)
	}
	epub := makeRecursiveTestZip(t, map[string][]byte{"mimetype": []byte("application/epub+zip")})
	members := map[string][]byte{
		"10241/巨龙的时空.epub": epub,
		"10241/sibling.zip": makeRecursiveTestZip(t, map[string][]byte{
			"side.txt": []byte("another branch"),
		}),
	}
	for _, volume := range volumes {
		payload, err := os.ReadFile(volume)
		if err != nil {
			t.Fatal(err)
		}
		members["10241/"+filepath.Base(volume)] = payload
	}
	// Append the archive to an MP4-style header, with ZIP-relative offsets.
	prefix := []byte{0, 0, 0, 24, 'f', 't', 'y', 'p', 'm', 'p', '4', '2'}
	source := filepath.Join(work, "多层测试.mp4")
	if err := os.WriteFile(source, append(prefix, makeRecursiveTestZip(t, members)...), 0600); err != nil {
		t.Fatal(err)
	}
	book, err := loadPasswordBook()
	if err != nil {
		t.Fatal(err)
	}
	if err := book.Remember("incorrect-regression-password"); err != nil {
		t.Fatal(err)
	}

	for _, wantPrompts := range []int{2, 0} {
		ui := &recursiveTestUI{reply: passwordReply{Value: password, Remember: true}}
		ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
		summary := executeNested(ctx, []string{source}, ui)
		cancel()
		if summary.Success != 1 || summary.Failed != 0 || summary.Skipped != 0 || summary.Canceled {
			t.Fatalf("extraction returned %+v (%s), last stage %q at depth %d (%s)",
				summary, ui.result.Message, ui.stage.Stage, ui.stage.Depth, ui.stage.Archive)
		}
		if ui.prompts != wantPrompts {
			t.Fatalf("password prompts = %d, want %d", ui.prompts, wantPrompts)
		}
		checkRecursiveTestEvents(t, ui.events, ui.result)
		checkRecursiveTestOutput(t, ui.result.Output, epub, deepPayload)
	}
}

func TestNestedBatchFileResults(t *testing.T) {
	work := t.TempDir()
	t.Setenv("LOCALAPPDATA", work)
	archive := filepath.Join(work, "empty.zip")
	if err := os.WriteFile(archive, makeRecursiveTestZip(t, nil), 0600); err != nil {
		t.Fatal(err)
	}
	media := filepath.Join(work, "plain.mp4")
	if err := os.WriteFile(media, []byte("no archive here"), 0600); err != nil {
		t.Fatal(err)
	}
	missing := filepath.Join(work, "missing.zip")

	ui := &recursiveTestUI{}
	summary := executeNested(context.Background(), []string{archive, archive, media, missing}, ui)
	if summary != (batchSummary{Total: 3, Success: 1, Skipped: 1, Failed: 1}) {
		t.Fatalf("unexpected batch summary %+v", summary)
	}
	statuses := []string{"success", "skipped", "failed"}
	index := 0
	for _, event := range ui.events {
		if event.Kind != "result" {
			continue
		}
		if index >= len(statuses) || event.Status != statuses[index] || event.Total != len(statuses) || event.Index != index+1 {
			t.Fatalf("unexpected file result %+v at position %d", event, index)
		}
		if index == 0 {
			if event.Output == "" || event.FilesWritten != 0 || event.ArchivesCompleted != 1 {
				t.Fatalf("unexpected empty ZIP result %+v", event)
			}
		} else if event.Output != "" || event.Message == "" || event.ArchivesCompleted != 0 {
			t.Fatalf("expected a reason and no output, got %+v", event)
		}
		index++
	}
	if index != len(statuses) {
		t.Fatalf("received %d file results, want %d", index, len(statuses))
	}
}

func TestNested7ZipVolumeGap(t *testing.T) {
	work := t.TempDir()
	first, last := filepath.Join(work, "split.7z.001"), filepath.Join(work, "split.7z.003")
	if err := os.WriteFile(first, []byte("7z\xbc\xaf\x27\x1c"), 0600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(last, nil, 0600); err != nil {
		t.Fatal(err)
	}
	if _, err := findNestedArchives([]string{last, first}); err == nil || !strings.Contains(err.Error(), "split.7z.002") {
		t.Fatalf("expected a missing middle volume error, got %v", err)
	}
}

type recursiveTestUI struct {
	reply   passwordReply
	prompts int
	result  nestedEvent
	stage   nestedEvent
	events  []nestedEvent
}

func (ui *recursiveTestUI) Update(event nestedEvent) {
	ui.events = append(ui.events, event)
	if event.Kind == "result" {
		ui.result = event
	}
	if event.Kind == "stage" {
		ui.stage = event
	}
}

func checkRecursiveTestEvents(t *testing.T, events []nestedEvent, result nestedEvent) {
	t.Helper()
	depths := []int{1, 2, 3, 2}
	current, discovered, completed := 0, 0, 0
	for _, event := range events {
		if event.Index == 0 {
			continue
		}
		if event.ArchivesDiscovered < discovered || event.ArchivesCompleted < completed ||
			event.ArchivesCompleted > event.ArchivesDiscovered {
			t.Fatalf("inconsistent archive progress: %+v", event)
		}
		discovered, completed = event.ArchivesDiscovered, event.ArchivesCompleted
		if event.ArchiveIndex > current {
			if event.ArchiveIndex != current+1 || current >= len(depths) || event.Depth != depths[current] {
				t.Fatalf("unexpected traversal after archive %d: %+v", current, event)
			}
			current = event.ArchiveIndex
		}
		if event.Kind == "progress" && (event.ArchiveIndex == 0 || event.Depth == 0 || event.Percent < 0 || event.Percent > 100) {
			t.Fatalf("unscoped archive percentage: %+v", event)
		}
	}
	if current != len(depths) || result.Status != "success" || result.ArchivesCompleted != len(depths) ||
		result.ArchivesDiscovered != len(depths) || result.MaxDepth != 3 || result.FilesWritten != 3 {
		t.Fatalf("incomplete recursive result after %d started archives: %+v", current, result)
	}
}

func (ui *recursiveTestUI) RequestPassword(_ context.Context, request passwordRequest) (passwordReply, error) {
	ui.prompts++
	switch {
	case ui.prompts == 1 && !request.Retry:
		return passwordReply{Value: "incorrect-manual-password", Remember: true}, nil
	case ui.prompts == 2 && request.Retry:
		return ui.reply, nil
	default:
		return passwordReply{}, fmt.Errorf("unexpected password retry")
	}
}

func makeRecursiveTestZip(t *testing.T, members map[string][]byte) []byte {
	t.Helper()
	var data bytes.Buffer
	writer := zip.NewWriter(&data)
	for name, payload := range members {
		member, err := writer.CreateHeader(&zip.FileHeader{Name: name, Method: zip.Store})
		if err != nil {
			t.Fatal(err)
		}
		if _, err := member.Write(payload); err != nil {
			t.Fatal(err)
		}
	}
	if err := writer.Close(); err != nil {
		t.Fatal(err)
	}
	return data.Bytes()
}

func checkRecursiveTestOutput(t *testing.T, output string, epub, deepPayload []byte) {
	t.Helper()
	parent := filepath.Join(output, "10241")
	storedEPUB, err := os.ReadFile(filepath.Join(parent, "巨龙的时空.epub"))
	if err != nil || !bytes.Equal(storedEPUB, epub) {
		t.Fatalf("EPUB was not preserved: %v", err)
	}
	want := map[string][]byte{
		"巨龙的时空.epub":                                     epub,
		filepath.Join("10241-解压", "media", "detail.txt"): deepPayload,
		filepath.Join("sibling-解压", "side.txt"):          []byte("another branch"),
	}
	err = filepath.WalkDir(parent, func(path string, entry fs.DirEntry, err error) error {
		if err != nil || entry.IsDir() {
			return err
		}
		relative, err := filepath.Rel(parent, path)
		if err != nil {
			return err
		}
		payload, expected := want[relative]
		if !expected {
			return fmt.Errorf("unexpected file %s", path)
		}
		actual, err := os.ReadFile(path)
		if err != nil || !bytes.Equal(actual, payload) {
			return fmt.Errorf("wrong content in %s: %v", path, err)
		}
		delete(want, relative)
		return nil
	})
	if err != nil || len(want) != 0 {
		t.Fatalf("recursive extraction is incomplete (%d missing): %v", len(want), err)
	}
}
