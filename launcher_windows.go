package main

import (
	"runtime"
	"strings"
	"syscall"
	"unsafe"
)

const (
	launcherPasswordBook = 201
	launcherRegisterMenu = 202
	launcherRemoveMenu   = 203
	launcherMenuStatus   = 204
	launcherNotices      = 205
	launcherNoticesText  = 206
)

var (
	launcherDialogCallback  = syscall.NewCallback(launcherDialogProc)
	launcherNoticesCallback = syscall.NewCallback(launcherNoticesProc)
)

func runLauncherGUI() int {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	guiEnablePerMonitorDPI()
	instance, _, err := guiGetModuleHandle.Call(0)
	if instance == 0 {
		guiNotify("无法打开管理窗口。\n"+err.Error(), true)
		return 1
	}
	template := guiDialogTemplate("NestedUnpack", 318, 136, []guiDialogControl{
		{x: 15, y: 14, width: 288, height: 13, id: 0xffff, class: 0x0082, text: "通过资源管理器右键菜单启动批量解压。"},
		{style: guiWSTabStop, x: 15, y: 33, width: 92, height: 20, id: launcherPasswordBook, class: 0x0080, text: "管理密码簿"},
		{style: 7, x: 15, y: 59, width: 288, height: 44, id: 0xffff, class: 0x0080, text: "资源管理器右键菜单"}, // BS_GROUPBOX
		{x: 25, y: 77, width: 118, height: 14, id: launcherMenuStatus, class: 0x0082},
		{style: guiWSTabStop, x: 148, y: 73, width: 80, height: 20, id: launcherRegisterMenu, class: 0x0080, text: "注册到当前位置"},
		{style: guiWSTabStop, x: 235, y: 73, width: 58, height: 20, id: launcherRemoveMenu, class: 0x0080, text: "移除"},
		{style: guiWSTabStop, x: 15, y: 110, width: 92, height: 20, id: launcherNotices, class: 0x0080, text: "第三方许可"},
		{style: guiWSTabStop | 1, x: 235, y: 110, width: 68, height: 20, id: guiOK, class: 0x0080, text: "关闭"},
	})
	result, _, err := guiDialogBox.Call(instance, uintptr(unsafe.Pointer(&template[0])), 0, launcherDialogCallback, 0)
	runtime.KeepAlive(template)
	if int32(result) == -1 {
		guiNotify("无法打开管理窗口。\n"+err.Error(), true)
		return 1
	}
	return 0
}

func launcherDialogProc(window, message, wParam, _ uintptr) uintptr {
	switch message {
	case guiWMInitDlg:
		refreshLauncherMenu(window)
		return 1
	case guiWMClose:
		guiEndDialog.Call(window, 0)
		return 1
	case guiWMCommand:
		var err error
		switch uint16(wParam) {
		case guiOK, guiCancel:
			guiEndDialog.Call(window, 0)
		case launcherPasswordBook:
			showPasswordBook(window)
		case launcherNotices:
			showLauncherNotices(window)
		case launcherRegisterMenu:
			err = registerExplorerMenu()
		case launcherRemoveMenu:
			err = removeExplorerMenu()
		default:
			return 0
		}
		if err != nil {
			guiMessageBox.Call(window, uintptr(unsafe.Pointer(guiString(err.Error()))),
				uintptr(unsafe.Pointer(guiString("无法更新右键菜单"))), 0x30)
		}
		if id := uint16(wParam); id == launcherRegisterMenu || id == launcherRemoveMenu {
			refreshLauncherMenu(window)
		}
		return 1
	}
	return 0
}

func showLauncherNotices(parent uintptr) {
	template := guiDialogTemplate("第三方许可", 418, 270, []guiDialogControl{
		{style: guiWSTabStop | guiWSVScroll | 0x0844, extended: guiWSClientEdge, x: 12, y: 12, width: 394, height: 216,
			id: launcherNoticesText, class: 0x0081}, // ES_READONLY | ES_AUTOVSCROLL | ES_MULTILINE
		{style: guiWSTabStop | 1, x: 338, y: 239, width: 68, height: 20, id: guiOK, class: 0x0080, text: "关闭"},
	})
	instance, _, _ := guiGetModuleHandle.Call(0)
	result, _, err := guiDialogBox.Call(instance, uintptr(unsafe.Pointer(&template[0])), parent, launcherNoticesCallback, 0)
	runtime.KeepAlive(template)
	if int32(result) == -1 {
		guiNotify("无法打开第三方许可窗口。\n"+err.Error(), true)
	}
}

func launcherNoticesProc(window, message, wParam, _ uintptr) uintptr {
	switch message {
	case guiWMInitDlg:
		edit, _, _ := guiGetDlgItem.Call(window, launcherNoticesText)
		text := strings.ReplaceAll(archiveNotices, "\r\n", "\n")
		guiText(edit, strings.ReplaceAll(text, "\n", "\r\n"))
		closeButton, _, _ := guiGetDlgItem.Call(window, guiOK)
		guiSetFocus.Call(closeButton)
		return 0
	case guiWMClose:
		guiEndDialog.Call(window, 0)
		return 1
	case guiWMCommand:
		if id := uint16(wParam); id == guiOK || id == guiCancel {
			guiEndDialog.Call(window, 0)
			return 1
		}
	}
	return 0
}

func refreshLauncherMenu(window uintptr) {
	registered, err := explorerMenuRegistered()
	label := "未注册到当前位置"
	if err != nil {
		label = "无法读取注册状态"
	} else if registered {
		label = "已注册到当前位置"
	}
	status, _, _ := guiGetDlgItem.Call(window, launcherMenuStatus)
	guiText(status, label)
}

func guiEnablePerMonitorDPI() {
	if guiSetProcessDPI.Find() == nil {
		guiSetProcessDPI.Call(^uintptr(3)) // DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2
	} else {
		guiSetLegacyDPI.Call()
	}
}
