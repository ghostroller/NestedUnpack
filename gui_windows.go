package main

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"syscall"
	"unsafe"
)

const (
	guiTitle     = "解开嵌套压缩包"
	guiClassName = "NestedUnpackProgressWindow"

	guiWMDestroy = 0x0002
	guiWMRedraw  = 0x000b
	guiWMClose   = 0x0010
	guiWMCommand = 0x0111
	guiWMInitDlg = 0x0110
	guiWMAction  = 0x8001
	guiWMSize    = 0x0005
	guiWMMinMax  = 0x0024
	guiWMColor   = 0x0138
	guiWMDPI     = 0x02e0

	guiWSChild       = 0x40000000
	guiWSVisible     = 0x10000000
	guiWSBorder      = 0x00800000
	guiWSCaption     = 0x00c00000
	guiWSSysMenu     = 0x00080000
	guiWSTabStop     = 0x00010000
	guiWSVScroll     = 0x00200000
	guiWSClientEdge  = 0x00000200
	guiMainStyle     = 0x00cf0000 // WS_OVERLAPPEDWINDOW
	guiKeepZOrder    = 0x0014     // SWP_NOZORDER | SWP_NOACTIVATE
	guiMinimumWidth  = 560
	guiMinimumHeight = 476

	guiOK       = 1
	guiCancel   = 2
	guiPassword = 101
	guiRemember = 102
	guiClose    = 103
	guiResults  = 104
	guiOpen     = 105

	guiLBAddString    = 0x0180
	guiLBInsertString = 0x0181
	guiLBDeleteString = 0x0182
	guiLBSetCurSel    = 0x0186
	guiLBGetCurSel    = 0x0188
	guiLBGetTopIndex  = 0x018e
	guiLBSetTopIndex  = 0x0197
	guiLBSelChange    = 1
	guiLBDoubleClick  = 2

	guiPBSetPos     = 0x0402
	guiPBSetMarquee = 0x040a
	guiPBSMarquee   = 0x0008

	guiTVDeleteItem = 0x1101
	guiTVExpand     = 0x1102
	guiTVSetIndent  = 0x1107
	guiTVInsertItem = 0x1132 // TVM_INSERTITEMW
	guiTVSetItem    = 0x113f // TVM_SETITEMW
	guiTVIFText     = 0x0001
	guiTVEExpand    = 0x0002
	guiTVIFirst     = ^uintptr(0xfffe)
	guiTVSStyle     = 0x17 // TVS_HASBUTTONS | TVS_HASLINES | TVS_LINESATROOT | TVS_DISABLEDRAGDROP
)

type guiProgressMode int

const (
	guiProgressHidden guiProgressMode = iota
	guiProgressBusy
	guiProgressMeasured
)

var (
	guiUser32   = syscall.NewLazyDLL("user32.dll")
	guiKernel32 = syscall.NewLazyDLL("kernel32.dll")
	guiGDI32    = syscall.NewLazyDLL("gdi32.dll")
	guiComCtl32 = syscall.NewLazyDLL("comctl32.dll")
	guiShell32  = syscall.NewLazyDLL("shell32.dll")
	guiOle32    = syscall.NewLazyDLL("ole32.dll")

	guiAdjustWindowRect  = guiUser32.NewProc("AdjustWindowRectEx")
	guiAdjustRectForDPI  = guiUser32.NewProc("AdjustWindowRectExForDpi")
	guiCreateWindow      = guiUser32.NewProc("CreateWindowExW")
	guiDefaultWndProc    = guiUser32.NewProc("DefWindowProcW")
	guiDestroyWindow     = guiUser32.NewProc("DestroyWindow")
	guiDialogBox         = guiUser32.NewProc("DialogBoxIndirectParamW")
	guiDispatchMessage   = guiUser32.NewProc("DispatchMessageW")
	guiEnableWindow      = guiUser32.NewProc("EnableWindow")
	guiEndDialog         = guiUser32.NewProc("EndDialog")
	guiGetClientRect     = guiUser32.NewProc("GetClientRect")
	guiGetCursorPos      = guiUser32.NewProc("GetCursorPos")
	guiGetDC             = guiUser32.NewProc("GetDC")
	guiGetDPIForWindow   = guiUser32.NewProc("GetDpiForWindow")
	guiGetDlgItem        = guiUser32.NewProc("GetDlgItem")
	guiGetMessage        = guiUser32.NewProc("GetMessageW")
	guiGetMonitorInfo    = guiUser32.NewProc("GetMonitorInfoW")
	guiGetSysColor       = guiUser32.NewProc("GetSysColor")
	guiGetSysColorBrush  = guiUser32.NewProc("GetSysColorBrush")
	guiGetWindowText     = guiUser32.NewProc("GetWindowTextW")
	guiGetWindowTextLen  = guiUser32.NewProc("GetWindowTextLengthW")
	guiIsDialogMessage   = guiUser32.NewProc("IsDialogMessageW")
	guiIsZoomed          = guiUser32.NewProc("IsZoomed")
	guiInvalidateRect    = guiUser32.NewProc("InvalidateRect")
	guiLoadCursor        = guiUser32.NewProc("LoadCursorW")
	guiMessageBox        = guiUser32.NewProc("MessageBoxW")
	guiMonitorFromRect   = guiUser32.NewProc("MonitorFromRect")
	guiMonitorFromWindow = guiUser32.NewProc("MonitorFromWindow")
	guiPostMessage       = guiUser32.NewProc("PostMessageW")
	guiPostQuitMessage   = guiUser32.NewProc("PostQuitMessage")
	guiRegisterClass     = guiUser32.NewProc("RegisterClassW")
	guiReleaseDC         = guiUser32.NewProc("ReleaseDC")
	guiSendMessage       = guiUser32.NewProc("SendMessageW")
	guiSetFocus          = guiUser32.NewProc("SetFocus")
	guiSetProcessDPI     = guiUser32.NewProc("SetProcessDpiAwarenessContext")
	guiSetLegacyDPI      = guiUser32.NewProc("SetProcessDPIAware")
	guiSetWindowPos      = guiUser32.NewProc("SetWindowPos")
	guiSetWindowText     = guiUser32.NewProc("SetWindowTextW")
	guiShowWindow        = guiUser32.NewProc("ShowWindow")
	guiSystemParameters  = guiUser32.NewProc("SystemParametersInfoW")
	guiTranslateMessage  = guiUser32.NewProc("TranslateMessage")
	guiUnregisterClass   = guiUser32.NewProc("UnregisterClassW")
	guiGetModuleHandle   = guiKernel32.NewProc("GetModuleHandleW")
	guiCreateFont        = guiGDI32.NewProc("CreateFontW")
	guiDeleteObject      = guiGDI32.NewProc("DeleteObject")
	guiGetDeviceCaps     = guiGDI32.NewProc("GetDeviceCaps")
	guiGetStockObject    = guiGDI32.NewProc("GetStockObject")
	guiSetBkColor        = guiGDI32.NewProc("SetBkColor")
	guiSetTextColor      = guiGDI32.NewProc("SetTextColor")
	guiInitControls      = guiComCtl32.NewProc("InitCommonControlsEx")
	guiShellExecute      = guiShell32.NewProc("ShellExecuteW")
	guiCOMInitialize     = guiOle32.NewProc("CoInitializeEx")
	guiCOMUninitialize   = guiOle32.NewProc("CoUninitialize")

	guiWndCallback = syscall.NewCallback(guiWndProc)
	guiDlgCallback = syscall.NewCallback(guiPasswordProc)

	// One Explorer launch owns one window and one locked UI thread.
	guiWindow *nativeNestedUI
)

type guiPoint struct{ X, Y int32 }

type guiRect struct{ Left, Top, Right, Bottom int32 }

type guiMonitorInfo struct {
	Size          uint32
	Monitor, Work guiRect
	Flags         uint32
}

type guiMinMaxInfo struct {
	Reserved, MaxSize, MaxPosition, MinTrackSize, MaxTrackSize guiPoint
}

type guiMessage struct {
	Window         uintptr
	Message        uint32
	WParam, LParam uintptr
	Time           uint32
	Point          guiPoint
	Private        uint32
}

type guiClass struct {
	Style                   uint32
	WindowProc              uintptr
	ClassExtra, WindowExtra int32
	Instance, Icon, Cursor  uintptr
	Background              uintptr
	MenuName, ClassName     *uint16
}

// TVITEMEXW is the largest member of the TVINSERTSTRUCTW union.
type guiTreeItem struct {
	Mask                               uint32
	Item                               uintptr
	State, StateMask                   uint32
	Text                               *uint16
	TextCapacity, Image, SelectedImage int32
	Children                           int32
	Param                              uintptr
	Integral                           int32
	StateEx                            uint32
	Window                             uintptr
	ExpandedImage, Reserved            int32
}

type guiTreeInsert struct {
	Parent, After uintptr
	Item          guiTreeItem
}

type guiResultEntry struct {
	event nestedEvent
	live  []*archiveStructure
	nodes map[int]int
}

type nativeNestedUI struct {
	jobCtx                        context.Context
	windowCtx                     context.Context
	cancelJob                     context.CancelFunc
	actions                       chan func()
	window, instance              uintptr
	batch, source, layer, archive uintptr
	stage, progress, marquee      uintptr
	section, list, details        uintptr
	treeTitle, tree, treeEmpty    uintptr
	open, close                   uintptr
	dpi                           uint32
	fonts                         [3]uintptr
	processed, total              int
	progressMode                  guiProgressMode
	layerText, archiveText        string
	results                       []guiResultEntry
	activeRow, treeRow            int
	followResults                 bool
	treeItems                     map[int]uintptr
	treeTail                      map[uintptr]uintptr
	treeExpanded                  map[uintptr]bool
	done, closing                 bool
	exitCode                      int
	password                      passwordReply
}

func runNestedGUI(paths []string) int {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	if result, _, _ := guiCOMInitialize.Call(0, 0x6); int32(result) >= 0 {
		defer guiCOMUninitialize.Call() // COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE
	}
	if guiSetProcessDPI.Find() == nil {
		guiSetProcessDPI.Call(^uintptr(3)) // DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2
	} else {
		guiSetLegacyDPI.Call()
	}
	windowCtx, cancelWindow := context.WithCancel(context.Background())
	defer cancelWindow()
	jobCtx, cancelJob := context.WithCancel(windowCtx)
	defer cancelJob()
	ui := &nativeNestedUI{
		jobCtx: jobCtx, windowCtx: windowCtx, cancelJob: cancelJob,
		actions: make(chan func(), 32), exitCode: 1,
	}
	guiWindow = ui
	defer func() { guiWindow = nil }()
	if err := ui.create(len(paths)); err != nil {
		guiNotify("无法打开解压窗口。\n"+err.Error(), true)
		return 1
	}
	defer func() {
		for _, font := range ui.fonts {
			if font != 0 {
				guiDeleteObject.Call(font)
			}
		}
		guiUnregisterClass.Call(uintptr(unsafe.Pointer(guiString(guiClassName))), ui.instance)
	}()
	guiShowWindow.Call(ui.window, 5) // SW_SHOW
	workerDone := make(chan struct{})
	go func() {
		defer close(workerDone)
		summary := executeNested(jobCtx, paths, ui)
		_ = ui.dispatch(windowCtx, func() { ui.finish(summary) })
	}()
	var message guiMessage
	for {
		result, _, messageErr := guiGetMessage.Call(uintptr(unsafe.Pointer(&message)), 0, 0, 0)
		if int32(result) <= 0 {
			if int32(result) < 0 {
				guiDestroyWindow.Call(ui.window)
				guiNotify("解压窗口发生错误。\n"+messageErr.Error(), true)
				ui.exitCode = 1
			}
			break
		}
		if handled, _, _ := guiIsDialogMessage.Call(ui.window, uintptr(unsafe.Pointer(&message))); handled == 0 {
			guiTranslateMessage.Call(uintptr(unsafe.Pointer(&message)))
			guiDispatchMessage.Call(uintptr(unsafe.Pointer(&message)))
		}
	}
	cancelWindow()
	<-workerDone
	return ui.exitCode
}

func (ui *nativeNestedUI) create(total int) error {
	ui.total = total
	ui.activeRow, ui.treeRow = -1, -1
	ui.followResults = true
	ui.treeItems = make(map[int]uintptr)
	ui.treeTail = make(map[uintptr]uintptr)
	ui.treeExpanded = make(map[uintptr]bool)
	instance, _, err := guiGetModuleHandle.Call(0)
	if instance == 0 {
		return err
	}
	ui.instance = instance
	controls := [2]uint32{8, 0x22} // INITCOMMONCONTROLSEX, ICC_PROGRESS_CLASS | ICC_TREEVIEW_CLASSES
	if initialized, _, err := guiInitControls.Call(uintptr(unsafe.Pointer(&controls))); initialized == 0 {
		return fmt.Errorf("初始化窗口控件：%w", err)
	}
	cursor, _, _ := guiLoadCursor.Call(0, 32512) // IDC_ARROW
	className := guiString(guiClassName)
	class := guiClass{
		WindowProc: guiWndCallback, Instance: instance, Cursor: cursor,
		Background: 6, ClassName: className, // COLOR_WINDOW + 1
	}
	if atom, _, err := guiRegisterClass.Call(uintptr(unsafe.Pointer(&class))); atom == 0 {
		return fmt.Errorf("注册解压窗口：%w", err)
	}
	work := guiActiveWorkArea()
	window, _, windowErr := guiCreateWindow.Call(0, uintptr(unsafe.Pointer(className)), uintptr(unsafe.Pointer(guiString(guiTitle))), guiMainStyle,
		uintptr(work.Left+32), uintptr(work.Top+32), 1, 1, 0, 0, instance, 0)
	if window == 0 {
		guiUnregisterClass.Call(uintptr(unsafe.Pointer(className)), instance)
		return fmt.Errorf("创建解压窗口：%w", windowErr)
	}
	ui.window = window
	ui.dpi = guiWindowDPI(window)
	controlsToCreate := []struct {
		handle              *uintptr
		class, text         string
		style, extended, id uintptr
		hidden              bool
	}{
		{&ui.batch, "STATIC", fmt.Sprintf("已处理 0 / %d 个文件", total), 0x4000, 0, 0, false},
		{&ui.source, "STATIC", "正在准备文件…", 0x4000 | 0x0080, 0, 0, false}, // SS_ENDELLIPSIS | SS_NOPREFIX
		{&ui.layer, "STATIC", "", 0x4000, 0, 0, false},
		{&ui.archive, "STATIC", "", 0x4000 | 0x0080, 0, 0, false},
		{&ui.stage, "STATIC", "", 0x4000, 0, 0, false},
		{&ui.progress, "msctls_progress32", "", 0, 0, 0, true},
		{&ui.marquee, "msctls_progress32", "", guiPBSMarquee, 0, 0, true},
		{&ui.section, "STATIC", "处理结果", 0, 0, 0, false},
		{&ui.list, "LISTBOX", "", guiWSBorder | guiWSVScroll | guiWSTabStop | 0x0101, 0, guiResults, false}, // LBS_NOTIFY | LBS_NOINTEGRALHEIGHT
		{&ui.treeTitle, "STATIC", "压缩包结构", 0, 0, 0, false},
		{&ui.tree, "SysTreeView32", "", guiWSBorder | guiWSTabStop | guiTVSStyle, 0, 0, true},
		{&ui.treeEmpty, "STATIC", "等待查看压缩包结构", guiWSBorder | 0x0201, 0, 0, false},                 // SS_CENTER | SS_CENTERIMAGE
		{&ui.details, "EDIT", "", guiWSBorder | guiWSVScroll | guiWSTabStop | 0x844, 0, 0, false}, // ES_READONLY | ES_AUTOVSCROLL | ES_MULTILINE
		{&ui.open, "BUTTON", "打开解压目录", guiWSTabStop, 0, guiOpen, false},
		{&ui.close, "BUTTON", "取消解压", guiWSTabStop, 0, guiClose, false},
	}
	for _, control := range controlsToCreate {
		style := guiWSChild | control.style
		if !control.hidden {
			style |= guiWSVisible
		}
		handle, _, err := guiCreateWindow.Call(control.extended, uintptr(unsafe.Pointer(guiString(control.class))), uintptr(unsafe.Pointer(guiString(control.text))),
			style, 0, 0, 0, 0, window, control.id, instance, 0)
		if handle == 0 {
			guiDestroyWindow.Call(window)
			guiUnregisterClass.Call(uintptr(unsafe.Pointer(className)), instance)
			return fmt.Errorf("创建 %s 控件：%w", control.class, err)
		}
		*control.handle = handle
	}
	guiEnableWindow.Call(ui.open, 0)
	guiSendMessage.Call(ui.details, 0x00c5, 0x7ffffffe, 0) // EM_SETLIMITTEXT
	ui.applyFonts()
	w, h := guiFrameSize(850, 620, ui.dpi)
	w = min(w, work.Right-work.Left)
	h = min(h, work.Bottom-work.Top)
	x, y := work.Left+(work.Right-work.Left-w)/2, work.Top+(work.Bottom-work.Top-h)/2
	guiSetWindowPos.Call(window, 0, uintptr(x), uintptr(y), uintptr(w), uintptr(h), guiKeepZOrder)
	ui.layout()
	return nil
}

func guiScale(value int32, dpi uint32) int32 {
	return (value*int32(dpi) + 48) / 96
}

func guiFrameSize(width, height int32, dpi uint32) (int32, int32) {
	rect := guiRect{Right: guiScale(width, dpi), Bottom: guiScale(height, dpi)}
	if guiAdjustRectForDPI.Find() == nil {
		guiAdjustRectForDPI.Call(uintptr(unsafe.Pointer(&rect)), guiMainStyle, 0, 0, uintptr(dpi))
	} else {
		guiAdjustWindowRect.Call(uintptr(unsafe.Pointer(&rect)), guiMainStyle, 0, 0)
	}
	return rect.Right - rect.Left, rect.Bottom - rect.Top
}

func guiActiveWorkArea() guiRect {
	var cursor guiPoint
	if ok, _, _ := guiGetCursorPos.Call(uintptr(unsafe.Pointer(&cursor))); ok != 0 {
		point := guiRect{cursor.X, cursor.Y, cursor.X + 1, cursor.Y + 1}
		monitor, _, _ := guiMonitorFromRect.Call(uintptr(unsafe.Pointer(&point)), 2) // MONITOR_DEFAULTTONEAREST
		if work, available := guiWorkArea(monitor); available {
			return work
		}
	}
	work := guiRect{Right: 1024, Bottom: 768}
	guiSystemParameters.Call(0x0030, 0, uintptr(unsafe.Pointer(&work)), 0) // SPI_GETWORKAREA
	return work
}

func guiWorkArea(monitor uintptr) (guiRect, bool) {
	info := guiMonitorInfo{Size: uint32(unsafe.Sizeof(guiMonitorInfo{}))}
	available, _, _ := guiGetMonitorInfo.Call(monitor, uintptr(unsafe.Pointer(&info)))
	return info.Work, available != 0
}

func guiWindowDPI(window uintptr) uint32 {
	if guiGetDPIForWindow.Find() == nil {
		if dpi, _, _ := guiGetDPIForWindow.Call(window); dpi != 0 {
			return uint32(dpi)
		}
	}
	if dc, _, _ := guiGetDC.Call(window); dc != 0 {
		dpi, _, _ := guiGetDeviceCaps.Call(dc, 88) // LOGPIXELSX
		guiReleaseDC.Call(window, dc)
		if dpi != 0 {
			return uint32(dpi)
		}
	}
	return 96
}

func (ui *nativeNestedUI) applyFonts() {
	previous := ui.fonts
	create := func(size int32, weight uintptr) uintptr {
		font, _, _ := guiCreateFont.Call(uintptr(-guiScale(size, ui.dpi)), 0, 0, 0, weight,
			0, 0, 0, 1, 0, 0, 5, 0, uintptr(unsafe.Pointer(guiString("Segoe UI"))))
		return font
	}
	ui.fonts = [3]uintptr{create(26, 600), create(16, 400), create(15, 600)}
	stock, _, _ := guiGetStockObject.Call(17) // DEFAULT_GUI_FONT
	for index, controls := range [3][]uintptr{
		{ui.batch},
		{ui.source, ui.layer, ui.stage, ui.list, ui.tree, ui.treeEmpty, ui.details, ui.open, ui.close},
		{ui.archive, ui.section, ui.treeTitle},
	} {
		font := ui.fonts[index]
		if font == 0 {
			font = stock
		}
		for _, control := range controls {
			guiSendMessage.Call(control, 0x0030, font, 1) // WM_SETFONT
		}
	}
	guiSendMessage.Call(ui.tree, guiTVSetIndent, uintptr(guiScale(18, ui.dpi)), 0)
	for _, font := range previous {
		if font != 0 {
			guiDeleteObject.Call(font)
		}
	}
}

func (ui *nativeNestedUI) layout() {
	var client guiRect
	if ok, _, _ := guiGetClientRect.Call(ui.window, uintptr(unsafe.Pointer(&client))); ok == 0 {
		return
	}
	scale := func(value int32) int32 { return guiScale(value, ui.dpi) }
	margin := scale(28)
	width := max(0, client.Right-2*margin)
	buttonWidth, buttonHeight := scale(112), scale(36)
	buttonTop := max(0, client.Bottom-scale(22)-buttonHeight)
	resultsHeading := scale(220)
	if ui.done {
		resultsHeading = scale(105)
	}
	resultsTop := resultsHeading + scale(25)
	resultsHeight := max(0, buttonTop-resultsTop-scale(16))
	listWidth := width * 34 / 100
	gap := scale(12)
	treeLeft := margin + listWidth + gap
	treeWidth := max(0, width-listWidth-gap)
	panelGap := min(gap, resultsHeight)
	treeHeight := (resultsHeight - panelGap) * 62 / 100
	position := func(handle uintptr, x, y, width, height int32) {
		guiSetWindowPos.Call(handle, 0, uintptr(x), uintptr(y), uintptr(width), uintptr(height), guiKeepZOrder)
	}
	position(ui.batch, margin, scale(22), width, scale(35))
	position(ui.source, margin, scale(65), width, scale(24))
	position(ui.layer, margin, scale(104), width, scale(22))
	position(ui.archive, margin, scale(130), width, scale(22))
	position(ui.stage, margin, scale(161), width, scale(23))
	position(ui.progress, margin, scale(188), width, scale(13))
	position(ui.marquee, margin, scale(188), width, scale(13))
	position(ui.section, margin, resultsHeading, listWidth, scale(21))
	position(ui.treeTitle, treeLeft, resultsHeading, treeWidth, scale(21))
	position(ui.list, margin, resultsTop, listWidth, resultsHeight)
	position(ui.tree, treeLeft, resultsTop, treeWidth, treeHeight)
	position(ui.treeEmpty, treeLeft, resultsTop, treeWidth, treeHeight)
	position(ui.details, treeLeft, resultsTop+treeHeight+panelGap, treeWidth, resultsHeight-treeHeight-panelGap)
	closeLeft := max(margin, client.Right-margin-buttonWidth)
	openWidth := scale(140)
	position(ui.open, max(margin, closeLeft-gap-openWidth), buttonTop, openWidth, buttonHeight)
	position(ui.close, closeLeft, buttonTop, buttonWidth, buttonHeight)
}

func (ui *nativeNestedUI) dispatch(ctx context.Context, action func()) error {
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-ui.windowCtx.Done():
		return ui.windowCtx.Err()
	case ui.actions <- action:
	}
	if sent, _, err := guiPostMessage.Call(ui.window, guiWMAction, 0, 0); sent == 0 {
		return fmt.Errorf("无法更新解压窗口：%w", err)
	}
	return nil
}

func (ui *nativeNestedUI) Update(event nestedEvent) {
	ctx := ui.jobCtx
	if event.Kind == "structure" || event.Kind == "result" || event.Kind == "warning" {
		ctx = ui.windowCtx
	}
	_ = ui.dispatch(ctx, func() { ui.render(event) })
}

func (ui *nativeNestedUI) render(event nestedEvent) {
	if ui.closing && event.Kind != "structure" && event.Kind != "result" && event.Kind != "warning" {
		return
	}
	switch event.Kind {
	case "source":
		ui.total = event.Total
		ui.updateBatch()
		guiText(ui.source, fmt.Sprintf("当前文件 %d / %d  ·  %s", event.Index, event.Total, filepath.Base(event.Source)))
		ui.showLayer("", "")
		ui.showStage("正在识别压缩格式", guiProgressBusy, 0)
		ui.activeRow = ui.addResult(event)
	case "stage", "progress":
		layer, archive := guiLayerDescription(event)
		ui.showLayer(layer, archive)
		mode := guiProgressBusy
		if event.Stage == "等待输入密码" {
			mode = guiProgressHidden
		} else if event.Percent >= 0 {
			mode = guiProgressMeasured
		}
		ui.showStage(event.Stage, mode, event.Percent)
	case "structure":
		ui.updateStructure(event)
	case "result":
		if event.Status != "canceled" {
			ui.processed++
		}
		ui.updateBatch()
		ui.completeResult(event)
		if !ui.closing {
			ui.showLayer("", "")
			ui.showStage("", guiProgressHidden, 0)
		}
	case "warning":
		ui.addResult(event)
	}
}

func guiLayerDescription(event nestedEvent) (description, archive string) {
	if event.ArchiveIndex != 0 {
		description = fmt.Sprintf("第 %d 层 · 第 %d 个压缩包", event.Depth, event.ArchiveIndex)
		if event.Archive != "" {
			if event.Archive == event.Source {
				archive = "外层压缩包"
			} else {
				archive = filepath.Base(event.Archive)
			}
		}
		if event.Format != "" {
			archive = event.Format + "  ·  " + archive
			if event.Parts > 1 {
				archive += fmt.Sprintf("（%d 个分卷）", event.Parts)
			}
		}
	}
	if event.ArchivesDiscovered > 0 {
		if description != "" {
			description += "    "
		}
		description += fmt.Sprintf("已解开 %d 个，已发现 %d 个", event.ArchivesCompleted, event.ArchivesDiscovered)
	}
	return description, archive
}

func (ui *nativeNestedUI) showLayer(description, archive string) {
	if ui.layerText != description {
		guiText(ui.layer, description)
		ui.layerText = description
	}
	if ui.archiveText != archive {
		guiText(ui.archive, archive)
		ui.archiveText = archive
	}
}

func (ui *nativeNestedUI) showStage(stage string, mode guiProgressMode, percent int) {
	if mode == guiProgressMeasured {
		percent = min(100, max(0, percent))
		stage += fmt.Sprintf("  ·  本步骤 %d%%", percent)
		guiSendMessage.Call(ui.progress, guiPBSetPos, uintptr(percent), 0)
	}
	guiText(ui.stage, stage)
	if mode == ui.progressMode {
		return
	}
	if ui.progressMode == guiProgressBusy {
		guiSendMessage.Call(ui.marquee, guiPBSetMarquee, 0, 0)
		guiShowWindow.Call(ui.marquee, 0) // SW_HIDE
	} else if ui.progressMode == guiProgressMeasured {
		guiShowWindow.Call(ui.progress, 0)
	}
	switch mode {
	case guiProgressBusy:
		guiShowWindow.Call(ui.marquee, 4) // SW_SHOWNOACTIVATE
		guiSendMessage.Call(ui.marquee, guiPBSetMarquee, 1, 0)
	case guiProgressMeasured:
		guiShowWindow.Call(ui.progress, 4)
	}
	ui.progressMode = mode
}

func (ui *nativeNestedUI) updateBatch() {
	guiText(ui.batch, fmt.Sprintf("已处理 %d / %d 个文件", ui.processed, ui.total))
}

func guiResultStatus(event nestedEvent) string {
	if event.Kind == "source" {
		return "处理中"
	}
	if event.Kind == "warning" {
		return "提醒"
	}
	switch event.Status {
	case "success":
		return "成功"
	case "skipped":
		return "跳过"
	case "canceled":
		return "取消"
	default:
		return "失败"
	}
}

func guiResultDetails(event nestedEvent) string {
	var lines []string
	if event.Source != "" {
		lines = append(lines, event.Source, "")
	}
	if event.Kind == "source" {
		lines = append(lines, "正在处理，压缩包结构会随解压更新。")
	} else if event.Output != "" {
		lines = append(lines, "解压目录", event.Output, "",
			fmt.Sprintf("写出 %d 个文件 · 解开 %d 个压缩包 · 最深 %d 层",
				event.FilesWritten, event.ArchivesCompleted, event.MaxDepth))
	} else {
		lines = append(lines, event.Message)
		if event.Kind == "result" && event.ArchivesCompleted > 0 {
			lines = append(lines, "", fmt.Sprintf("中止前已解开 %d 个压缩包 · 最深到达第 %d 层",
				event.ArchivesCompleted, event.MaxDepth))
		}
	}
	text := strings.ReplaceAll(strings.Join(lines, "\n"), "\r\n", "\n")
	return strings.ReplaceAll(text, "\n", "\r\n")
}

func guiResultLabel(event nestedEvent) string {
	label := guiResultStatus(event)
	if event.Source != "" {
		label += "  ·  " + filepath.Base(event.Source)
	} else if event.Message != "" {
		firstLine, _, _ := strings.Cut(event.Message, "\n")
		label += "  ·  " + strings.TrimSpace(firstLine)
	}
	return label
}

func (ui *nativeNestedUI) addResult(event nestedEvent) int {
	current := ui.selectedRow()
	index, _, _ := guiSendMessage.Call(ui.list, guiLBAddString, 0, uintptr(unsafe.Pointer(guiString(guiResultLabel(event)))))
	if int(index) < 0 {
		return -1
	}
	ui.results = append(ui.results, guiResultEntry{event: event})
	if current < 0 || (event.Kind == "source" && ui.followResults) {
		guiSendMessage.Call(ui.list, guiLBSetCurSel, index, 0)
		ui.showSelectedResult()
	}
	return int(index)
}

func (ui *nativeNestedUI) activeResult(event nestedEvent) int {
	if ui.activeRow >= 0 && ui.results[ui.activeRow].event.Index == event.Index {
		return ui.activeRow
	}
	return -1
}

func (ui *nativeNestedUI) updateStructure(event nestedEvent) {
	row, node := ui.activeResult(event), event.Structure
	if row < 0 || node == nil {
		return
	}
	entry := &ui.results[row]
	if entry.nodes == nil {
		entry.nodes = make(map[int]int)
	}
	if index, ok := entry.nodes[node.ID]; ok {
		entry.live[index] = node
	} else {
		entry.nodes[node.ID] = len(entry.live)
		entry.live = append(entry.live, node)
	}
	if ui.treeRow == row {
		ui.updateArchiveTree(node, true, true)
	}
}

func (ui *nativeNestedUI) completeResult(event nestedEvent) {
	row := ui.activeResult(event)
	if row < 0 {
		ui.addResult(event)
		return
	}
	ui.activeRow = -1
	entry := &ui.results[row]
	entry.event = event
	if ui.treeRow == row {
		if len(ui.treeItems) == 0 || event.Structure == nil {
			ui.treeRow = -1
		} else {
			// The final tree reconciles every node without replacing native handles.
			ui.insertArchiveBranch(event.Structure, entry)
		}
	}
	entry.live, entry.nodes = nil, nil

	selected := ui.selectedRow()
	topIndex, _, _ := guiSendMessage.Call(ui.list, guiLBGetTopIndex, 0, 0)
	guiSendMessage.Call(ui.list, guiWMRedraw, 0, 0)
	guiSendMessage.Call(ui.list, guiLBDeleteString, uintptr(row), 0)
	guiSendMessage.Call(ui.list, guiLBInsertString, uintptr(row), uintptr(unsafe.Pointer(guiString(guiResultLabel(event)))))
	guiSendMessage.Call(ui.list, guiLBSetCurSel, uintptr(selected), 0)
	guiSendMessage.Call(ui.list, guiLBSetTopIndex, topIndex, 0)
	guiSendMessage.Call(ui.list, guiWMRedraw, 1, 0)
	guiInvalidateRect.Call(ui.list, 0, 1)
	if selected == row {
		ui.showSelectedResult()
	}
}

func (ui *nativeNestedUI) selectedRow() int {
	current, _, _ := guiSendMessage.Call(ui.list, guiLBGetCurSel, 0, 0)
	if current >= uintptr(len(ui.results)) {
		return -1
	}
	return int(current)
}

func (ui *nativeNestedUI) selectedResult() (nestedEvent, bool) {
	row := ui.selectedRow()
	if row < 0 {
		return nestedEvent{}, false
	}
	return ui.results[row].event, true
}

func (ui *nativeNestedUI) showSelectedResult() {
	ui.showResultTree(ui.selectedRow())
	event, selected := ui.selectedResult()
	if !selected {
		guiText(ui.details, "")
		guiEnableWindow.Call(ui.open, 0)
		return
	}
	guiText(ui.details, guiResultDetails(event))
	guiSendMessage.Call(ui.details, 0x00b1, 0, 0) // EM_SETSEL
	guiSendMessage.Call(ui.details, 0x00b7, 0, 0) // EM_SCROLLCARET
	enabled := uintptr(0)
	if event.Status == "success" && event.Output != "" {
		enabled = 1
	}
	guiEnableWindow.Call(ui.open, enabled)
}

func (ui *nativeNestedUI) showResultTree(row int) {
	if ui.treeRow == row {
		return
	}
	ui.treeRow = row
	guiSendMessage.Call(ui.tree, guiTVDeleteItem, 0, 0)
	clear(ui.treeItems)
	clear(ui.treeTail)
	clear(ui.treeExpanded)
	empty := "暂无压缩包结构"
	if row >= 0 {
		entry := &ui.results[row]
		if entry.event.Kind == "source" {
			for _, node := range entry.live {
				ui.updateArchiveTree(node, true, true)
			}
			empty = "等待识别压缩包"
		} else if root := entry.event.Structure; root != nil {
			ui.insertArchiveBranch(root, nil)
			for node, depth := root, 0; node != nil && depth < 3; depth++ {
				ui.expandArchiveTree(ui.treeItems[node.ID])
				if len(node.Children) != 1 {
					break
				}
				node = node.Children[0]
			}
		}
	}
	if len(ui.treeItems) == 0 {
		guiShowWindow.Call(ui.tree, 0) // SW_HIDE
		guiText(ui.treeEmpty, empty)
		guiShowWindow.Call(ui.treeEmpty, 4) // SW_SHOWNOACTIVATE
	}
}

func (ui *nativeNestedUI) insertArchiveBranch(node *archiveStructure, previous *guiResultEntry) {
	changed := true
	if previous != nil && ui.treeItems[node.ID] != 0 {
		if index, ok := previous.nodes[node.ID]; ok {
			changed = guiArchiveTreeLabel(previous.live[index], true) != guiArchiveTreeLabel(node, false)
		}
	}
	if changed {
		ui.updateArchiveTree(node, false, false)
	}
	for _, child := range node.Children {
		ui.insertArchiveBranch(child, previous)
	}
}

func guiArchiveTreeLabel(node *archiveStructure, running bool) string {
	label := node.Name + " · " + node.Format
	if len(node.Volumes) > 0 {
		label += fmt.Sprintf(" · %d 个分卷", len(node.Volumes))
	}
	switch node.Status {
	case "active":
		label += " · 正在处理"
	case "opened":
		label += " · 已解开"
		if node.Files > 0 {
			label += fmt.Sprintf(" · 本层 %d 个普通文件", node.Files)
		}
	case "failed":
		label += " · 失败"
	case "skipped":
		label += " · 跳过"
	case "canceled":
		label += " · 取消"
	case "pending":
		if running {
			label += " · 待处理"
		} else {
			label += " · 未处理"
		}
	}
	return label
}

func (ui *nativeNestedUI) updateArchiveTree(node *archiveStructure, running, expandParent bool) {
	label := guiArchiveTreeLabel(node, running)
	if item := ui.treeItems[node.ID]; item != 0 {
		ui.setArchiveTreeText(item, label)
		return
	}
	parent := ui.treeItems[node.ParentID]
	if node.ParentID != 0 && parent == 0 {
		return
	}
	item := ui.insertTreeItem(parent, label)
	if item == 0 {
		return
	}
	ui.treeItems[node.ID] = item
	for _, volume := range node.Volumes {
		ui.insertTreeItem(item, filepath.Base(volume))
	}
	if parent == 0 {
		guiShowWindow.Call(ui.treeEmpty, 0)
		guiShowWindow.Call(ui.tree, 4)
	} else if expandParent && !ui.treeExpanded[parent] {
		ui.expandArchiveTree(parent)
	}
}

func (ui *nativeNestedUI) insertTreeItem(parent uintptr, label string) uintptr {
	after := ui.treeTail[parent]
	if after == 0 {
		after = guiTVIFirst
	}
	insert := guiTreeInsert{
		Parent: parent,
		After:  after,
		Item:   guiTreeItem{Mask: guiTVIFText, Text: guiString(label)},
	}
	item, _, _ := guiSendMessage.Call(ui.tree, guiTVInsertItem, 0, uintptr(unsafe.Pointer(&insert)))
	runtime.KeepAlive(insert.Item.Text)
	if item != 0 {
		ui.treeTail[parent] = item
	}
	return item
}

func (ui *nativeNestedUI) setArchiveTreeText(item uintptr, label string) {
	if item != 0 {
		value := guiTreeItem{Mask: guiTVIFText, Item: item, Text: guiString(label)}
		guiSendMessage.Call(ui.tree, guiTVSetItem, 0, uintptr(unsafe.Pointer(&value)))
		runtime.KeepAlive(value.Text)
	}
}

func (ui *nativeNestedUI) expandArchiveTree(item uintptr) {
	if item != 0 {
		guiSendMessage.Call(ui.tree, guiTVExpand, guiTVEExpand, item)
		ui.treeExpanded[item] = true
	}
}

func (ui *nativeNestedUI) openSelectedResult() {
	event, selected := ui.selectedResult()
	if !selected || event.Status != "success" || event.Output == "" {
		return
	}
	info, err := os.Stat(event.Output)
	if err != nil || !info.IsDir() {
		guiNotify("解压目录已无法访问。\n"+event.Output, true)
		return
	}
	result, _, _ := guiShellExecute.Call(ui.window, uintptr(unsafe.Pointer(guiString("explore"))),
		uintptr(unsafe.Pointer(guiString(event.Output))), 0, 0, 1) // SW_SHOWNORMAL
	if result <= 32 {
		guiNotify("无法打开解压目录。\n"+event.Output, true)
	}
}

func (ui *nativeNestedUI) finish(summary batchSummary) {
	ui.done = true
	ui.exitCode = summary.exitCode()
	status := "解压结束"
	if summary.Canceled {
		status = "已取消解压"
	}
	ui.processed, ui.total = summary.Success+summary.Skipped+summary.Failed, summary.Total
	ui.updateBatch()
	guiText(ui.source, fmt.Sprintf("%s  ·  成功 %d  ·  跳过 %d  ·  失败 %d", status, summary.Success, summary.Skipped, summary.Failed))
	ui.showLayer("", "")
	ui.showStage("", guiProgressHidden, 0)
	for _, control := range []uintptr{ui.layer, ui.archive, ui.stage} {
		guiShowWindow.Call(control, 0) // SW_HIDE
	}
	ui.layout()
	guiText(ui.close, "关闭")
	guiSendMessage.Call(ui.close, 0x00f4, 1, 1) // BM_SETSTYLE, BS_DEFPUSHBUTTON
	guiEnableWindow.Call(ui.close, 1)
	guiSetFocus.Call(ui.close)
}

func guiWndProc(window, message, wParam, lParam uintptr) uintptr {
	ui := guiWindow
	if ui != nil && window == ui.window {
		switch message {
		case guiWMSize:
			if wParam != 1 { // SIZE_MINIMIZED
				ui.layout()
			}
			return 0
		case guiWMDPI:
			ui.dpi = uint32(uint16(wParam))
			ui.applyFonts()
			bounds := *guiMessagePointer[guiRect](lParam)
			width, height := bounds.Right-bounds.Left, bounds.Bottom-bounds.Top
			if maximized, _, _ := guiIsZoomed.Call(window); maximized == 0 {
				monitor, _, _ := guiMonitorFromRect.Call(uintptr(unsafe.Pointer(&bounds)), 2)
				if work, ok := guiWorkArea(monitor); ok {
					width = min(width, work.Right-work.Left)
					height = min(height, work.Bottom-work.Top)
					bounds.Left = max(work.Left, min(bounds.Left, work.Right-width))
					bounds.Top = max(work.Top, min(bounds.Top, work.Bottom-height))
				}
			}
			guiSetWindowPos.Call(window, 0, uintptr(bounds.Left), uintptr(bounds.Top), uintptr(width), uintptr(height), guiKeepZOrder)
			ui.layout()
			return 0
		case guiWMMinMax:
			info := guiMessagePointer[guiMinMaxInfo](lParam)
			width, height := guiFrameSize(guiMinimumWidth, guiMinimumHeight, ui.dpi)
			monitor, _, _ := guiMonitorFromWindow.Call(window, 2) // MONITOR_DEFAULTTONEAREST
			if work, ok := guiWorkArea(monitor); ok {
				width = min(width, work.Right-work.Left)
				height = min(height, work.Bottom-work.Top)
			}
			info.MinTrackSize = guiPoint{width, height}
			return 0
		case guiWMColor:
			if lParam == ui.batch || lParam == ui.source || lParam == ui.layer || lParam == ui.archive ||
				lParam == ui.stage || lParam == ui.section || lParam == ui.treeTitle || lParam == ui.treeEmpty || lParam == ui.details {
				color := uintptr(8) // COLOR_WINDOWTEXT
				if lParam == ui.stage || lParam == ui.treeEmpty {
					color = 17 // COLOR_GRAYTEXT
				}
				text, _, _ := guiGetSysColor.Call(color)
				background, _, _ := guiGetSysColor.Call(5) // COLOR_WINDOW
				guiSetTextColor.Call(wParam, text)
				guiSetBkColor.Call(wParam, background)
				brush, _, _ := guiGetSysColorBrush.Call(5)
				return brush
			}
		case guiWMAction:
			select {
			case action := <-ui.actions:
				action()
			default:
			}
			return 0
		case guiWMCommand:
			id := uint16(wParam)
			code := uint16(wParam >> 16)
			if id == guiResults && code == guiLBSelChange {
				ui.followResults = ui.activeRow >= 0 && ui.selectedRow() == ui.activeRow
				ui.showSelectedResult()
				return 0
			}
			if id == guiOpen || (id == guiResults && code == guiLBDoubleClick) {
				ui.openSelectedResult()
				return 0
			}
			if id == guiClose || id == guiCancel || (id == guiOK && ui.done) {
				ui.closeOrCancel()
				return 0
			}
		case guiWMClose:
			ui.closeOrCancel()
			return 0
		case guiWMDestroy:
			ui.cancelJob()
			guiPostQuitMessage.Call(0)
			return 0
		case 0x0400: // DM_GETDEFID, used by IsDialogMessage for Enter after completion
			if ui.done {
				return 0x534b0000 | guiClose // DC_HASDEFID
			}
			return 0
		}
	}
	result, _, _ := guiDefaultWndProc.Call(window, message, wParam, lParam)
	return result
}

func guiMessagePointer[T any](lParam uintptr) *T {
	// Windows owns the memory and lends it for the duration of the message.
	return *(**T)(unsafe.Pointer(&lParam))
}

func (ui *nativeNestedUI) closeOrCancel() {
	if ui.done {
		guiDestroyWindow.Call(ui.window)
		return
	}
	if !ui.closing {
		ui.closing = true
		ui.cancelJob()
		ui.showStage("正在取消并清理暂存文件", guiProgressBusy, 0)
		guiEnableWindow.Call(ui.close, 0)
	}
}

func (ui *nativeNestedUI) RequestPassword(ctx context.Context, request passwordRequest) (passwordReply, error) {
	type answer struct {
		reply passwordReply
		err   error
	}
	reply := make(chan answer, 1)
	if err := ui.dispatch(ctx, func() {
		if err := ctx.Err(); err != nil {
			reply <- answer{err: err}
			return
		}
		value, err := ui.askPassword(request)
		reply <- answer{value, err}
	}); err != nil {
		return passwordReply{}, err
	}
	select {
	case result := <-reply:
		return result.reply, result.err
	case <-ctx.Done():
		return passwordReply{}, ctx.Err()
	}
}

func (ui *nativeNestedUI) askPassword(request passwordRequest) (passwordReply, error) {
	ui.showStage("等待输入密码", guiProgressHidden, 0)
	prompt := "请输入压缩包密码。"
	if request.Retry {
		prompt = "密码验证未通过，请重新输入。"
	}
	archive := filepath.Base(request.Archive)
	if request.Archive == "" {
		archive = filepath.Base(request.Source)
	}
	subtitle := fmt.Sprintf("第 %d 层 · %s · %s", request.Depth, request.Format, archive)
	template := guiPasswordTemplate(prompt, subtitle)
	result, _, err := guiDialogBox.Call(ui.instance, uintptr(unsafe.Pointer(&template[0])), ui.window, guiDlgCallback, 0)
	runtime.KeepAlive(template)
	reply := ui.password
	ui.password = passwordReply{}
	if int32(result) == -1 {
		return passwordReply{}, fmt.Errorf("无法打开密码输入窗口：%w", err)
	}
	if reply.Value != "" {
		ui.showStage("验证输入的密码", guiProgressBusy, 0)
	}
	return reply, nil
}

func guiPasswordProc(window, message, wParam, _ uintptr) uintptr {
	switch message {
	case guiWMInitDlg:
		edit, _, _ := guiGetDlgItem.Call(window, guiPassword)
		guiSendMessage.Call(edit, 0x00c5, 32767, 0) // EM_SETLIMITTEXT
		return 1
	case guiWMCommand:
		switch uint16(wParam) {
		case guiOK:
			edit, _, _ := guiGetDlgItem.Call(window, guiPassword)
			check, _, _ := guiGetDlgItem.Call(window, guiRemember)
			length, _, _ := guiGetWindowTextLen.Call(edit)
			buffer := make([]uint16, int(length)+1)
			guiGetWindowText.Call(edit, uintptr(unsafe.Pointer(&buffer[0])), uintptr(len(buffer)))
			remember, _, _ := guiSendMessage.Call(check, 0x00f0, 0, 0) // BM_GETCHECK
			guiWindow.password = passwordReply{Value: syscall.UTF16ToString(buffer), Remember: remember == 1}
			clear(buffer)
			guiText(edit, "")
			guiEndDialog.Call(window, guiOK)
			return 1
		case guiCancel:
			guiEndDialog.Call(window, guiCancel)
			return 1
		}
	case guiWMClose:
		guiEndDialog.Call(window, guiCancel)
		return 1
	}
	return 0
}

// The dialog manager handles focus, tab navigation, Enter and Escape for this
// native DLGTEMPLATE, so it behaves like a dialog compiled from a .rc file.
func guiPasswordTemplate(prompt, subtitle string) []uint16 {
	const style = 0x80000000 | guiWSCaption | guiWSSysMenu | 0x08c0 // WS_POPUP | DS_CENTER | DS_SETFONT | DS_MODALFRAME
	template := []uint16{}
	appendDword := func(value uint32) { template = append(template, uint16(value), uint16(value>>16)) }
	appendText := func(value string) {
		encoded, _ := syscall.UTF16FromString(value)
		template = append(template, encoded...)
	}
	appendDword(style)
	appendDword(0)
	template = append(template, 6, 0, 0, 344, 143, 0, 0)
	appendText("压缩包密码")
	template = append(template, 10) // DS_SETFONT point size
	appendText("Segoe UI")
	add := func(style, extended uint32, x, y, w, h, id, class uint16, text string) {
		if len(template)%2 != 0 {
			template = append(template, 0)
		}
		appendDword(guiWSChild | guiWSVisible | style)
		appendDword(extended)
		template = append(template, x, y, w, h, id, 0xffff, class)
		appendText(text)
		template = append(template, 0) // no control creation data
	}
	add(0, 0, 15, 15, 314, 15, 0xffff, 0x0082, prompt) // STATIC
	add(0x4000, 0, 15, 36, 314, 15, 0xffff, 0x0082, subtitle)
	add(guiWSTabStop|0x00a0, guiWSClientEdge, 15, 58, 314, 19, guiPassword, 0x0081, "") // EDIT | ES_PASSWORD | ES_AUTOHSCROLL
	add(guiWSTabStop|3, 0, 15, 86, 314, 14, guiRemember, 0x0080, "密码正确时保存到密码簿")         // BS_AUTOCHECKBOX
	add(guiWSTabStop, 0, 154, 111, 87, 21, guiCancel, 0x0080, "跳过当前文件")
	add(guiWSTabStop|1, 0, 249, 111, 80, 21, guiOK, 0x0080, "确定")
	return template
}

func guiText(window uintptr, text string) {
	guiSetWindowText.Call(window, uintptr(unsafe.Pointer(guiString(text))))
}

func guiString(value string) *uint16 {
	pointer, _ := syscall.UTF16PtrFromString(strings.ReplaceAll(value, "\x00", "�"))
	return pointer
}

func guiNotify(message string, warning bool) {
	style := uintptr(0x40 | 0x10000) // MB_ICONINFORMATION | MB_SETFOREGROUND
	if warning {
		style = 0x30 | 0x10000
	}
	guiMessageBox.Call(0, uintptr(unsafe.Pointer(guiString(message))), uintptr(unsafe.Pointer(guiString(guiTitle))), style)
}
