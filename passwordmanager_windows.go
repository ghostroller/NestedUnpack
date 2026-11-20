package main

import (
	"fmt"
	"runtime"
	"slices"
	"syscall"
	"unsafe"
)

const (
	guiBookList = 201 + iota
	guiBookPreview
	guiBookReveal
	guiBookRemove
	guiBookInput
	guiBookCount
)

type passwordBookDialog struct {
	book                                  *passwordBook
	passwords                             []string
	window, list, preview, reveal, remove uintptr
	input, add, count                     uintptr
}

var (
	guiBookCallback = syscall.NewCallback(guiBookProc)
	guiBookWindow   *passwordBookDialog
)

func showPasswordBook(parent uintptr) {
	book, err := loadPasswordBook()
	if err != nil {
		guiBookError(parent, "无法读取密码簿", err)
		return
	}
	template := guiBookTemplate()
	instance, _, _ := guiGetModuleHandle.Call(0)
	guiBookWindow = &passwordBookDialog{book: book}
	defer func() { guiBookWindow = nil }()
	result, _, callErr := guiDialogBox.Call(instance, uintptr(unsafe.Pointer(&template[0])), parent, guiBookCallback, 0)
	runtime.KeepAlive(template)
	if int32(result) == -1 {
		guiBookError(parent, "无法打开密码簿", callErr)
	}
}

func guiBookProc(window, message, wParam, lParam uintptr) uintptr {
	dialog := guiBookWindow
	if dialog == nil {
		return 0
	}
	switch message {
	case guiWMInitDlg:
		dialog.window = window
		control := func(id uintptr) uintptr {
			handle, _, _ := guiGetDlgItem.Call(window, id)
			return handle
		}
		dialog.list = control(guiBookList)
		dialog.preview = control(guiBookPreview)
		dialog.reveal = control(guiBookReveal)
		dialog.remove = control(guiBookRemove)
		dialog.input = control(guiBookInput)
		dialog.add = control(guiOK)
		dialog.count = control(guiBookCount)
		guiSendMessage.Call(dialog.input, 0x00c5, 32767, 0) // EM_SETLIMITTEXT
		guiEnableWindow.Call(dialog.add, 0)
		dialog.refresh(dialog.book.Passwords(), 0)
		guiSetFocus.Call(dialog.input)
		return 0
	case guiWMCommand:
		switch uint16(wParam) {
		case guiBookList:
			if uint16(wParam>>16) == guiLBSelChange {
				dialog.hidePreview()
				return 1
			}
		case guiBookReveal:
			dialog.updatePreview()
			return 1
		case guiBookRemove:
			dialog.removeSelected()
			return 1
		case guiBookInput:
			if uint16(wParam>>16) == 0x0300 { // EN_CHANGE
				length, _, _ := guiGetWindowTextLen.Call(dialog.input)
				guiEnableWindow.Call(dialog.add, guiBookEnabled(length != 0))
				return 1
			}
		case guiOK:
			dialog.addPassword()
			return 1
		case guiCancel:
			dialog.close()
			return 1
		}
	case guiWMClose:
		dialog.close()
		return 1
	}
	return 0
}

func (dialog *passwordBookDialog) refresh(passwords []string, row int) {
	dialog.passwords = passwords
	guiSendMessage.Call(dialog.list, 0x0184, 0, 0) // LB_RESETCONTENT
	for index := range passwords {
		label := guiString(fmt.Sprintf("密码 %d", index+1))
		guiSendMessage.Call(dialog.list, guiLBAddString, 0, uintptr(unsafe.Pointer(label)))
	}
	if len(passwords) != 0 {
		row = min(max(row, 0), len(passwords)-1)
		guiSendMessage.Call(dialog.list, guiLBSetCurSel, uintptr(row), 0)
	}
	guiText(dialog.count, fmt.Sprintf("已保存的密码（%d）", len(passwords)))
	dialog.hidePreview()
}

func (dialog *passwordBookDialog) selected() int {
	row, _, _ := guiSendMessage.Call(dialog.list, guiLBGetCurSel, 0, 0)
	return int(int32(row))
}

func (dialog *passwordBookDialog) hidePreview() {
	guiSendMessage.Call(dialog.reveal, 0x00f1, 0, 0) // BM_SETCHECK
	dialog.updatePreview()
}

func (dialog *passwordBookDialog) updatePreview() {
	row := dialog.selected()
	selected := row >= 0 && row < len(dialog.passwords)
	guiEnableWindow.Call(dialog.reveal, guiBookEnabled(selected))
	guiEnableWindow.Call(dialog.remove, guiBookEnabled(selected))
	revealed, _, _ := guiSendMessage.Call(dialog.reveal, 0x00f0, 0, 0) // BM_GETCHECK
	text := ""
	if selected && revealed == 1 {
		text = dialog.passwords[row]
	} else if selected {
		text = "••••••••"
	}
	guiText(dialog.preview, text)
	guiEnableWindow.Call(dialog.preview, guiBookEnabled(selected && revealed == 1))
}

func (dialog *passwordBookDialog) addPassword() {
	length, _, _ := guiGetWindowTextLen.Call(dialog.input)
	if length == 0 {
		return
	}
	buffer := make([]uint16, int(length)+1)
	guiGetWindowText.Call(dialog.input, uintptr(unsafe.Pointer(&buffer[0])), uintptr(len(buffer)))
	password := syscall.UTF16ToString(buffer)
	clear(buffer)
	if password == "" {
		return
	}
	if err := dialog.book.Remember(password); err != nil {
		guiBookError(dialog.window, "无法保存密码", err)
		return
	}
	guiText(dialog.input, "")
	passwords := dialog.book.Passwords()
	dialog.refresh(passwords, slices.Index(passwords, password))
	guiSetFocus.Call(dialog.input)
}

func (dialog *passwordBookDialog) removeSelected() {
	row := dialog.selected()
	if row < 0 || row >= len(dialog.passwords) {
		return
	}
	message := guiString("要从密码簿删除所选密码吗？")
	title := guiString("密码簿")
	answer, _, _ := guiMessageBox.Call(dialog.window, uintptr(unsafe.Pointer(message)), uintptr(unsafe.Pointer(title)), 0x124) // MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2
	if answer != 6 {                                                                                                           // IDYES
		return
	}
	if err := dialog.book.Forget(dialog.passwords[row]); err != nil {
		guiBookError(dialog.window, "无法删除密码", err)
		return
	}
	dialog.refresh(dialog.book.Passwords(), row)
	if len(dialog.passwords) == 0 {
		guiSetFocus.Call(dialog.input)
	} else {
		guiSetFocus.Call(dialog.list)
	}
}

func (dialog *passwordBookDialog) close() {
	guiText(dialog.preview, "")
	guiText(dialog.input, "")
	guiEndDialog.Call(dialog.window, guiCancel)
}

func guiBookEnabled(enabled bool) uintptr {
	if enabled {
		return 1
	}
	return 0
}

func guiBookError(parent uintptr, message string, err error) {
	text := guiString(message + "。\n" + err.Error())
	title := guiString("密码簿")
	guiMessageBox.Call(parent, uintptr(unsafe.Pointer(text)), uintptr(unsafe.Pointer(title)), 0x30) // MB_ICONWARNING
}

func guiBookTemplate() []uint16 {
	const (
		button  = 0x0080
		edit    = 0x0081
		label   = 0x0082
		listBox = 0x0083
	)
	return guiDialogTemplate("密码簿", 338, 203, []guiDialogControl{
		{x: 14, y: 12, width: 310, height: 14, id: guiBookCount, class: label},
		{style: guiWSTabStop | guiWSVScroll | 0x0101, extended: guiWSClientEdge, x: 14, y: 28, width: 310, height: 77, id: guiBookList, class: listBox}, // LBS_NOTIFY | LBS_NOINTEGRALHEIGHT
		{x: 14, y: 116, width: 58, height: 14, id: 0xffff, class: label, text: "所选密码"},
		{style: guiWSTabStop | 0x0880, extended: guiWSClientEdge, x: 72, y: 113, width: 142, height: 18, id: guiBookPreview, class: edit}, // ES_READONLY | ES_AUTOHSCROLL
		{style: guiWSTabStop | 3, x: 222, y: 114, width: 43, height: 17, id: guiBookReveal, class: button, text: "显示"},                    // BS_AUTOCHECKBOX
		{style: guiWSTabStop, x: 270, y: 112, width: 54, height: 20, id: guiBookRemove, class: button, text: "删除"},
		{x: 14, y: 143, width: 58, height: 14, id: 0xffff, class: label, text: "添加密码"},
		{style: guiWSTabStop | 0x00a0, extended: guiWSClientEdge, x: 72, y: 140, width: 190, height: 18, id: guiBookInput, class: edit}, // ES_PASSWORD | ES_AUTOHSCROLL
		{style: guiWSTabStop | 1, x: 270, y: 139, width: 54, height: 20, id: guiOK, class: button, text: "添加"},                          // BS_DEFPUSHBUTTON
		{style: guiWSTabStop, x: 254, y: 171, width: 70, height: 20, id: guiCancel, class: button, text: "关闭"},
	})
}

type guiDialogControl struct {
	style, extended                uint32
	x, y, width, height, id, class uint16
	text                           string
}

// The system dialog manager uses this standard DLGTEMPLATE for layout and keyboard navigation.
func guiDialogTemplate(title string, width, height uint16, controls []guiDialogControl) []uint16 {
	const style = 0x80000000 | guiWSCaption | guiWSSysMenu | 0x08c0 // WS_POPUP | DS_CENTER | DS_SETFONT | DS_MODALFRAME
	var template []uint16
	appendDword := func(value uint32) { template = append(template, uint16(value), uint16(value>>16)) }
	appendText := func(value string) {
		encoded, _ := syscall.UTF16FromString(value)
		template = append(template, encoded...)
	}
	appendDword(style)
	appendDword(0)
	template = append(template, uint16(len(controls)), 0, 0, width, height, 0, 0)
	appendText(title)
	template = append(template, 10) // DS_SETFONT point size
	appendText("Segoe UI")
	for _, control := range controls {
		if len(template)%2 != 0 {
			template = append(template, 0)
		}
		appendDword(guiWSChild | guiWSVisible | control.style)
		appendDword(control.extended)
		template = append(template, control.x, control.y, control.width, control.height, control.id, 0xffff, control.class)
		appendText(control.text)
		template = append(template, 0) // no control creation data
	}
	return template
}
