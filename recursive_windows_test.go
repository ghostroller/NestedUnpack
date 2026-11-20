package main

import (
	"archive/zip"
	"bytes"
	"context"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"time"
)

func TestRecursiveSplitArchiveWithDocument(t *testing.T) {
	t.Setenv("PATH", t.TempDir())
	t.Setenv("LOCALAPPDATA", t.TempDir())
	work := t.TempDir()
	const password = "nestedunpack-regression-only"
	deepPayload := bytes.Repeat([]byte("nested media payload\n"), 600)
	volumes := test7ZipVolumes(t)
	epub := makeRecursiveTestZip(t, map[string][]byte{"mimetype": []byte("application/epub+zip")})
	members := map[string][]byte{
		"10241/巨龙的时空.epub": epub,
		"10241/sibling.zip": makeRecursiveTestZip(t, map[string][]byte{
			"side.txt": []byte("another branch"),
		}),
	}
	for index, volume := range volumes {
		members[fmt.Sprintf("10241/10241.7z.%03d", index+1)] = volume
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

	volumeNames := make([]string, 0, len(volumes))
	for index := range volumes {
		volumeNames = append(volumeNames, filepath.Join("10241", fmt.Sprintf("10241.7z.%03d", index+1)))
	}
	splitName := filepath.Join("10241", "10241.7z")
	siblingName := filepath.Join("10241", "sibling.zip")
	partial := &recursiveTestUI{skipPassword: true}
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	summary := executeNested(ctx, []string{source}, partial)
	cancel()
	if summary.Skipped != 1 || partial.prompts != 1 || partial.result.Output != "" {
		t.Fatalf("expected to skip at the encrypted split archive, got %+v: %+v", summary, partial.result)
	}
	checkRecursiveTestStructure(t, partial.result.Structure, &archiveStructure{
		Name: filepath.Base(source), Format: "ZIP", Status: "opened", Files: 1,
		Children: []*archiveStructure{
			{Name: splitName, Format: "7z 分卷", Status: "skipped", Files: -1, Volumes: volumeNames},
			{Name: siblingName, Format: "ZIP", Status: "pending", Files: -1},
		},
	})
	checkRecursiveTestStructureUpdates(t, partial.events, []string{filepath.Base(source), splitName})

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
		checkRecursiveTestStructure(t, ui.result.Structure, &archiveStructure{
			Name: filepath.Base(source), Format: "ZIP", Status: "opened", Files: 1,
			Children: []*archiveStructure{
				{
					Name: splitName, Format: "7z 分卷", Status: "opened", Files: 0, Volumes: volumeNames,
					Children: []*archiveStructure{
						{Name: "inner.zip", Format: "ZIP", Status: "opened", Files: 1},
					},
				},
				{Name: siblingName, Format: "ZIP", Status: "opened", Files: 1},
			},
		})
		checkRecursiveTestStructureUpdates(t, ui.events, []string{filepath.Base(source), splitName, "inner.zip", siblingName})
		checkRecursiveTestOutput(t, ui.result.Output, epub, deepPayload)
	}
}

func TestArchiveStructurePreservesFlattenedWrappers(t *testing.T) {
	work := t.TempDir()
	t.Setenv("LOCALAPPDATA", work)
	leaf := makeRecursiveTestZip(t, map[string][]byte{"document.txt": []byte("payload")})
	wrapper := makeRecursiveTestZip(t, map[string][]byte{
		"left/same.zip":  leaf,
		"right/same.zip": leaf,
	})
	path := filepath.Join(work, "outer.zip")
	if err := os.WriteFile(path, makeRecursiveTestZip(t, map[string][]byte{"wrapper/inside.zip": wrapper}), 0600); err != nil {
		t.Fatal(err)
	}
	ui := &recursiveTestUI{}
	if summary := executeNested(context.Background(), []string{path}, ui); summary.Success != 1 {
		t.Fatalf("failed to extract test archive: %+v (%s)", summary, ui.result.Message)
	}
	checkRecursiveTestStructure(t, ui.result.Structure, &archiveStructure{
		Name: "outer.zip", Format: "ZIP", Status: "opened", Files: 0,
		Children: []*archiveStructure{
			{
				Name: filepath.Join("wrapper", "inside.zip"), Format: "ZIP", Status: "opened", Files: 0,
				Children: []*archiveStructure{
					{Name: filepath.Join("left", "same.zip"), Format: "ZIP", Status: "opened", Files: 1},
					{Name: filepath.Join("right", "same.zip"), Format: "ZIP", Status: "opened", Files: 1},
				},
			},
		},
	})
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
		} else if event.Output != "" || event.Message == "" || event.ArchivesCompleted != 0 || event.Structure != nil {
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
	reply        passwordReply
	skipPassword bool
	prompts      int
	result       nestedEvent
	stage        nestedEvent
	events       []nestedEvent
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
	if ui.skipPassword {
		return passwordReply{}, nil
	}
	switch {
	case ui.prompts == 1 && !request.Retry:
		return passwordReply{Value: "incorrect-manual-password", Remember: true}, nil
	case ui.prompts == 2 && request.Retry:
		return ui.reply, nil
	default:
		return passwordReply{}, fmt.Errorf("unexpected password retry")
	}
}

func checkRecursiveTestStructure(t *testing.T, actual, expected *archiveStructure) {
	t.Helper()
	if actual == nil || actual.Name != expected.Name || actual.Format != expected.Format ||
		actual.Status != expected.Status || actual.Files != expected.Files ||
		!slices.Equal(actual.Volumes, expected.Volumes) || len(actual.Children) != len(expected.Children) {
		t.Fatalf("archive structure node = %+v; want %+v", actual, expected)
	}
	for index, child := range actual.Children {
		checkRecursiveTestStructure(t, child, expected.Children[index])
	}
}

func checkRecursiveTestStructureUpdates(t *testing.T, events []nestedEvent, expectedActive []string) {
	t.Helper()
	var active []string
	nodes := make(map[int]*archiveStructure)
	for _, event := range events {
		if event.Kind == "result" {
			if !slices.Equal(active, expectedActive) {
				t.Fatalf("live archive updates = %q; want %q before final result", active, expectedActive)
			}
			var checkLatest func(*archiveStructure)
			checkLatest = func(node *archiveStructure) {
				update := nodes[node.ID]
				if update == nil || update.Name != node.Name || update.Status != node.Status ||
					update.Files != node.Files || update.ParentID != node.ParentID || !slices.Equal(update.Volumes, node.Volumes) {
					t.Fatalf("result node %+v was not already reflected in live update %+v", node, update)
				}
				for _, child := range node.Children {
					checkLatest(child)
				}
			}
			checkLatest(event.Structure)
			return
		}
		if event.Kind != "structure" {
			continue
		}
		node := event.Structure
		if node == nil || node.ID <= 0 || len(node.Children) != 0 ||
			(node.ParentID != 0 && nodes[node.ParentID] == nil) {
			t.Fatalf("invalid live tree update: %+v", node)
		}
		if previous := nodes[node.ID]; previous != nil &&
			(previous.Name != node.Name || previous.ParentID != node.ParentID) {
			t.Fatalf("archive tree node changed identity: %+v then %+v", previous, node)
		}
		if node.Status == "active" {
			active = append(active, node.Name)
		}
		nodes[node.ID] = node
	}
	t.Fatal("missing final result event")
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
