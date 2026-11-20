package main

import (
	"bytes"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"syscall"
	"testing"
	"unsafe"
)

func TestExplorerCommandKeepsWholeSelection(t *testing.T) {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	if result, _, _ := guiCOMInitialize.Call(0, 0x6); int32(result) < 0 {
		t.Fatalf("CoInitializeEx: 0x%08x", uint32(result))
	}
	defer guiCOMUninitialize.Call()
	explorerServer = &explorerServerState{}
	defer func() { explorerServer = nil }()

	dir := t.TempDir()
	paths := []string{filepath.Join(dir, "第一 个.zip"), filepath.Join(dir, "第二 个.rar")}
	parse := guiShell32.NewProc("SHParseDisplayName")
	arrayFromIDs := guiShell32.NewProc("SHCreateShellItemArrayFromIDLists")
	ids := make([]uintptr, len(paths))
	for index, path := range paths {
		if err := os.WriteFile(path, nil, 0600); err != nil {
			t.Fatal(err)
		}
		result, _, _ := parse.Call(uintptr(unsafe.Pointer(guiString(path))), 0, uintptr(unsafe.Pointer(&ids[index])), 0, 0)
		if int32(result) < 0 {
			t.Fatalf("SHParseDisplayName(%q): 0x%08x", path, uint32(result))
		}
		defer explorerCoFree.Call(ids[index])
	}
	var items *explorerShellItemArray
	result, _, _ := arrayFromIDs.Call(uintptr(len(ids)), uintptr(unsafe.Pointer(&ids[0])), uintptr(unsafe.Pointer(&items)))
	if int32(result) < 0 {
		t.Fatalf("SHCreateShellItemArrayFromIDLists: 0x%08x", uint32(result))
	}
	defer syscall.SyscallN(items.table.release, uintptr(unsafe.Pointer(items)))

	factory := &explorerFactory{table: &explorerFactoryTable, refs: 1}
	var commandAddress unsafe.Pointer
	result, _, _ = syscall.SyscallN(explorerFactoryTable[3], uintptr(unsafe.Pointer(factory)), 0,
		uintptr(unsafe.Pointer(&explorerExecuteID)), uintptr(unsafe.Pointer(&commandAddress)))
	if result != explorerOK {
		t.Fatalf("CreateInstance: 0x%08x", uint32(result))
	}
	command := (*explorerCommand)(commandAddress)
	defer command.release()
	var selectionAddress unsafe.Pointer
	result, _, _ = syscall.SyscallN(command.table[0], uintptr(commandAddress), uintptr(unsafe.Pointer(&explorerSelectionID)),
		uintptr(unsafe.Pointer(&selectionAddress)))
	if result != explorerOK {
		t.Fatalf("QueryInterface(IObjectWithSelection): 0x%08x", uint32(result))
	}
	selection := (*explorerSelection)(selectionAddress)
	defer selection.release()
	result, _, _ = syscall.SyscallN(selection.table[3], uintptr(selectionAddress), uintptr(unsafe.Pointer(items)))
	if result != explorerOK {
		t.Fatalf("SetSelection: 0x%08x", uint32(result))
	}
	var output bytes.Buffer
	if err := writeExplorerSelection(&output, command.items); err != nil {
		t.Fatal(err)
	}
	if want := strings.Join(paths, "\n") + "\n"; !strings.EqualFold(output.String(), want) {
		t.Fatalf("selection = %q, want %q", output.String(), want)
	}
}
