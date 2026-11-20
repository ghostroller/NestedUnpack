package main

import (
	"bufio"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"syscall"
	"time"
	"unsafe"
)

const (
	explorerOK             = 0
	explorerNoInterface    = 0x80004002
	explorerBadPointer     = 0x80004003
	explorerFailure        = 0x80004005
	explorerUnexpected     = 0x8000ffff
	explorerOutOfMemory    = 0x8007000e
	explorerNoAggregation  = 0x80040110
	explorerRunSelection   = 0x8002 // WM_APP + 2
	explorerFilesystemPath = 0x80058000
)

var (
	explorerClassID     = syscall.GUID{Data1: 0x9e941531, Data2: 0x6a19, Data3: 0x4358, Data4: [8]byte{0xa7, 0x73, 0x30, 0xcd, 0x66, 0x4b, 0xe0, 0x34}}
	explorerIUnknownID  = syscall.GUID{Data4: [8]byte{0xc0, 0, 0, 0, 0, 0, 0, 0x46}}
	explorerFactoryID   = syscall.GUID{Data1: 1, Data4: [8]byte{0xc0, 0, 0, 0, 0, 0, 0, 0x46}}
	explorerExecuteID   = syscall.GUID{Data1: 0x7f9185b0, Data2: 0xcb92, Data3: 0x43c5, Data4: [8]byte{0x80, 0xa9, 0x92, 0x27, 0x7a, 0x4f, 0x7b, 0x54}}
	explorerSelectionID = syscall.GUID{Data1: 0x1c9cd5bb, Data2: 0x98e9, Data3: 0x4491, Data4: [8]byte{0xa6, 0x0f, 0x31, 0xaa, 0xcc, 0x72, 0xb8, 0x3c}}
	explorerCoRegister  = guiOle32.NewProc("CoRegisterClassObject")
	explorerCoRevoke    = guiOle32.NewProc("CoRevokeClassObject")
	explorerCoAlloc     = guiOle32.NewProc("CoTaskMemAlloc")
	explorerCoFree      = guiOle32.NewProc("CoTaskMemFree")
	explorerPeekMessage = guiUser32.NewProc("PeekMessageW")
	explorerPostThread  = guiUser32.NewProc("PostThreadMessageW")
	explorerSetTimer    = guiUser32.NewProc("SetTimer")
	explorerKillTimer   = guiUser32.NewProc("KillTimer")
	explorerGetThreadID = guiKernel32.NewProc("GetCurrentThreadId")
	explorerUTF16Length = guiKernel32.NewProc("lstrlenW")

	explorerFactoryTable   [5]uintptr
	explorerExecuteTable   [10]uintptr
	explorerSelectionTable [5]uintptr
	explorerServer         *explorerServerState
)

type explorerServerState struct {
	threadID    uintptr
	lastActive  time.Time
	pending     []*explorerCommand
	liveObjects int
	locks       int
	activated   bool
}

// These layouts mirror the COM interfaces in the Windows SDK. Objects have
// native allocations because COM can hold interface pointers across calls.
type explorerFactory struct {
	table *[5]uintptr
	refs  uint32
}

type explorerCommand struct {
	table     *[10]uintptr
	selection explorerSelection
	items     *explorerShellItemArray
	refs      uint32
	noUI      bool
	scheduled bool
}

type explorerSelection struct {
	table *[5]uintptr
	owner *explorerCommand
}

type explorerUnknownTable struct{ query, addRef, release uintptr }

type explorerShellItemArray struct {
	table *struct {
		explorerUnknownTable
		bind, propertyStore, propertyDescriptions, attributes uintptr
		getCount, getItemAt, enumItems                        uintptr
	}
}

type explorerShellItem struct {
	table *struct {
		explorerUnknownTable
		bind, parent, getDisplayName, attributes, compare uintptr
	}
}

func init() {
	explorerFactoryTable = [5]uintptr{
		syscall.NewCallback((*explorerFactory).query),
		syscall.NewCallback((*explorerFactory).addRef),
		syscall.NewCallback((*explorerFactory).release),
		syscall.NewCallback((*explorerFactory).createInstance),
		syscall.NewCallback((*explorerFactory).lockServer),
	}
	noOp := syscall.NewCallback(func(_, _ uintptr) uintptr { return explorerOK })
	position := noOp
	if unsafe.Sizeof(uintptr(0)) == 4 {
		// POINT is passed by value, occupying two argument slots on x86.
		position = syscall.NewCallback(func(_, _, _ uintptr) uintptr { return explorerOK })
	}
	explorerExecuteTable = [10]uintptr{
		syscall.NewCallback((*explorerCommand).query),
		syscall.NewCallback((*explorerCommand).addRef),
		syscall.NewCallback((*explorerCommand).release),
		noOp, // SetKeyState
		noOp, // SetParameters
		position,
		noOp, // SetShowWindow
		syscall.NewCallback((*explorerCommand).setNoUI),
		noOp, // SetDirectory
		syscall.NewCallback((*explorerCommand).execute),
	}
	explorerSelectionTable = [5]uintptr{
		syscall.NewCallback((*explorerSelection).query),
		syscall.NewCallback((*explorerSelection).addRef),
		syscall.NewCallback((*explorerSelection).release),
		syscall.NewCallback((*explorerSelection).set),
		syscall.NewCallback((*explorerSelection).get),
	}
}

func (factory *explorerFactory) query(iid *syscall.GUID, object *unsafe.Pointer) uintptr {
	if object == nil {
		return explorerBadPointer
	}
	*object = nil
	if iid == nil {
		return explorerBadPointer
	}
	if *iid != explorerIUnknownID && *iid != explorerFactoryID {
		return explorerNoInterface
	}
	*object = unsafe.Pointer(factory)
	factory.addRef()
	return explorerOK
}

func (factory *explorerFactory) addRef() uintptr {
	factory.refs++
	return uintptr(factory.refs)
}

func (factory *explorerFactory) release() uintptr {
	factory.refs--
	remaining := factory.refs
	if remaining == 0 {
		explorerCoFree.Call(uintptr(unsafe.Pointer(factory)))
	}
	return uintptr(remaining)
}

func (factory *explorerFactory) createInstance(outer uintptr, iid *syscall.GUID, object *unsafe.Pointer) uintptr {
	if object == nil {
		return explorerBadPointer
	}
	*object = nil
	if outer != 0 {
		return explorerNoAggregation
	}
	address, _, _ := explorerCoAlloc.Call(unsafe.Sizeof(explorerCommand{}))
	if address == 0 {
		return explorerOutOfMemory
	}
	command := guiMessagePointer[explorerCommand](address)
	*command = explorerCommand{
		table: &explorerExecuteTable, refs: 1,
		selection: explorerSelection{table: &explorerSelectionTable, owner: command},
	}
	explorerServer.activated = true
	explorerServer.liveObjects++
	result := command.query(iid, object)
	command.release()
	return result
}

func (factory *explorerFactory) lockServer(lock uintptr) uintptr {
	if uint32(lock) != 0 {
		explorerServer.locks++
	} else if explorerServer.locks > 0 {
		explorerServer.locks--
		explorerServer.lastActive = time.Now()
	}
	return explorerOK
}

func (command *explorerCommand) query(iid *syscall.GUID, object *unsafe.Pointer) uintptr {
	if object == nil {
		return explorerBadPointer
	}
	*object = nil
	if iid == nil {
		return explorerBadPointer
	}
	switch *iid {
	case explorerIUnknownID, explorerExecuteID:
		*object = unsafe.Pointer(command)
	case explorerSelectionID:
		*object = unsafe.Pointer(&command.selection)
	default:
		return explorerNoInterface
	}
	command.addRef()
	return explorerOK
}

func (command *explorerCommand) addRef() uintptr {
	command.refs++
	return uintptr(command.refs)
}

func (command *explorerCommand) release() uintptr {
	command.refs--
	remaining := command.refs
	if remaining == 0 {
		if command.items != nil {
			syscall.SyscallN(command.items.table.release, uintptr(unsafe.Pointer(command.items)))
		}
		explorerCoFree.Call(uintptr(unsafe.Pointer(command)))
		explorerServer.liveObjects--
		explorerServer.lastActive = time.Now()
	}
	return uintptr(remaining)
}

func (command *explorerCommand) setNoUI(value uintptr) uintptr {
	command.noUI = uint32(value) != 0
	return explorerOK
}

func (command *explorerCommand) execute() uintptr {
	if command.items == nil || command.scheduled {
		return explorerUnexpected
	}
	ok, _, err := explorerPostThread.Call(explorerServer.threadID, explorerRunSelection, 0, 0)
	if ok == 0 {
		if code, ok := err.(syscall.Errno); ok && code != 0 {
			return 0x80070000 | (uintptr(code) & 0xffff) // HRESULT_FROM_WIN32
		}
		return explorerFailure
	}
	command.scheduled = true
	command.addRef()
	explorerServer.pending = append(explorerServer.pending, command)
	return explorerOK
}

func (selection *explorerSelection) query(iid *syscall.GUID, object *unsafe.Pointer) uintptr {
	return selection.owner.query(iid, object)
}

func (selection *explorerSelection) addRef() uintptr { return selection.owner.addRef() }

func (selection *explorerSelection) release() uintptr { return selection.owner.release() }

func (selection *explorerSelection) set(items *explorerShellItemArray) uintptr {
	command := selection.owner
	if command.scheduled {
		return explorerUnexpected
	}
	if items != nil {
		syscall.SyscallN(items.table.addRef, uintptr(unsafe.Pointer(items)))
	}
	if command.items != nil {
		syscall.SyscallN(command.items.table.release, uintptr(unsafe.Pointer(command.items)))
	}
	command.items = items
	return explorerOK
}

func (selection *explorerSelection) get(iid *syscall.GUID, object *unsafe.Pointer) uintptr {
	if object == nil {
		return explorerBadPointer
	}
	*object = nil
	if iid == nil {
		return explorerBadPointer
	}
	items := selection.owner.items
	if items == nil {
		return explorerFailure
	}
	result, _, _ := syscall.SyscallN(items.table.query, uintptr(unsafe.Pointer(items)), uintptr(unsafe.Pointer(iid)), uintptr(unsafe.Pointer(object)))
	return result
}

func runExplorerCommandServer() int {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	defer func() { explorerServer = nil }()
	if result, _, _ := guiCOMInitialize.Call(0, 0x6); int32(result) < 0 { // STA | DISABLE_OLE1DDE
		return 1
	}
	defer guiCOMUninitialize.Call()
	threadID, _, _ := explorerGetThreadID.Call()
	explorerServer = &explorerServerState{threadID: threadID, lastActive: time.Now()}

	var message guiMessage
	explorerPeekMessage.Call(uintptr(unsafe.Pointer(&message)), 0, 0, 0, 0) // Ensure a thread message queue exists.
	timer, _, _ := explorerSetTimer.Call(0, 0, 1000, 0)
	if timer == 0 {
		return 1
	}
	defer explorerKillTimer.Call(0, timer)

	address, _, _ := explorerCoAlloc.Call(unsafe.Sizeof(explorerFactory{}))
	if address == 0 {
		return 1
	}
	factory := guiMessagePointer[explorerFactory](address)
	*factory = explorerFactory{table: &explorerFactoryTable, refs: 1}
	defer factory.release()
	var registration uint32
	result, _, _ := explorerCoRegister.Call(uintptr(unsafe.Pointer(&explorerClassID)), address, 4, 0,
		uintptr(unsafe.Pointer(&registration))) // CLSCTX_LOCAL_SERVER, REGCLS_SINGLEUSE
	if int32(result) < 0 {
		return 1
	}
	defer explorerCoRevoke.Call(uintptr(registration))

	for {
		status, _, _ := guiGetMessage.Call(uintptr(unsafe.Pointer(&message)), 0, 0, 0)
		if int32(status) <= 0 {
			if int32(status) < 0 {
				return 1
			}
			return 0
		}
		switch {
		case message.Window == 0 && message.Message == explorerRunSelection:
			if len(explorerServer.pending) == 0 {
				continue
			}
			command := explorerServer.pending[0]
			explorerServer.pending[0] = nil
			explorerServer.pending = explorerServer.pending[1:]
			err := launchExplorerSelection(command.items)
			noUI := command.noUI
			command.release()
			if err != nil && !noUI {
				guiNotify("无法启动批量解压。\n"+err.Error(), true)
			}
		case message.Window == 0 && message.Message == 0x0113 && message.WParam == timer: // WM_TIMER
			idleLimit := 30 * time.Second
			if explorerServer.activated {
				idleLimit = 5 * time.Second
			}
			if explorerServer.liveObjects == 0 && explorerServer.locks == 0 && time.Since(explorerServer.lastActive) >= idleLimit {
				return 0
			}
		default:
			guiTranslateMessage.Call(uintptr(unsafe.Pointer(&message)))
			guiDispatchMessage.Call(uintptr(unsafe.Pointer(&message)))
		}
	}
}

func launchExplorerSelection(items *explorerShellItemArray) error {
	exe, err := os.Executable()
	if err != nil {
		return err
	}
	manifest, err := os.CreateTemp("", "NestedUnpack-Selection-*.utf8")
	if err != nil {
		return err
	}
	started := false
	defer func() {
		if !started {
			os.Remove(manifest.Name())
		}
	}()
	writer := bufio.NewWriter(manifest)
	err = writeExplorerSelection(writer, items)
	if err == nil {
		err = writer.Flush()
	}
	if err = errors.Join(err, manifest.Close()); err != nil {
		return err
	}
	process := exec.Command(exe, "--nested-selection-file", manifest.Name())
	if err := process.Start(); err != nil {
		return err
	}
	started = true
	return process.Process.Release()
}

func writeExplorerSelection(writer io.Writer, items *explorerShellItemArray) error {
	var count uint32
	result, _, _ := syscall.SyscallN(items.table.getCount, uintptr(unsafe.Pointer(items)), uintptr(unsafe.Pointer(&count)))
	if int32(result) < 0 {
		return fmt.Errorf("读取选区失败 (HRESULT 0x%08X)", uint32(result))
	}
	if count == 0 {
		return errors.New("选区内没有可处理的文件")
	}
	for index := uint32(0); index < count; index++ {
		var item *explorerShellItem
		result, _, _ := syscall.SyscallN(items.table.getItemAt, uintptr(unsafe.Pointer(items)), uintptr(index), uintptr(unsafe.Pointer(&item)))
		if int32(result) < 0 {
			return fmt.Errorf("读取选区项目失败 (HRESULT 0x%08X)", uint32(result))
		}
		var name *uint16
		result, _, _ = syscall.SyscallN(item.table.getDisplayName, uintptr(unsafe.Pointer(item)), explorerFilesystemPath, uintptr(unsafe.Pointer(&name)))
		syscall.SyscallN(item.table.release, uintptr(unsafe.Pointer(item)))
		if int32(result) < 0 {
			return fmt.Errorf("读取选区文件路径失败 (HRESULT 0x%08X)", uint32(result))
		}
		if name == nil {
			return errors.New("选区含无效的文件路径")
		}
		length, _, _ := explorerUTF16Length.Call(uintptr(unsafe.Pointer(name)))
		path := syscall.UTF16ToString(unsafe.Slice(name, int(length)))
		explorerCoFree.Call(uintptr(unsafe.Pointer(name)))
		if !filepath.IsAbs(path) || strings.ContainsAny(path, "\r\n") {
			return errors.New("选区含无效的文件路径")
		}
		if _, err := io.WriteString(writer, path+"\n"); err != nil {
			return err
		}
	}
	return nil
}
