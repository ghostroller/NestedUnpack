package main

import (
	"errors"
	"fmt"
	"os"
	"strings"
	"syscall"
	"unsafe"
)

const (
	explorerMenuCLSID   = "{9E941531-6A19-4358-A773-30CD664BE034}"
	explorerVerbKey     = `Software\Classes\*\shell\NestedUnpack.Extract`
	explorerClassKey    = `Software\Classes\CLSID\` + explorerMenuCLSID
	explorerServerKey   = explorerClassKey + `\LocalServer32`
	explorerCommandKey  = explorerVerbKey + `\command`
	explorerMenuTitle   = "解开嵌套压缩包"
	explorerMenuApplies = `System.FileExtension:=".zip" OR System.FileExtension:=".zipx" OR System.FileExtension:=".mp4" OR ` +
		`System.FileExtension:=".rar" OR System.FileExtension:=".7z" OR System.FileName:~>".7z.001"`
)

var (
	explorerAdvapi         = syscall.NewLazyDLL("advapi32.dll")
	explorerRegCreateKey   = explorerAdvapi.NewProc("RegCreateKeyExW")
	explorerRegSetValue    = explorerAdvapi.NewProc("RegSetValueExW")
	explorerRegDeleteTree  = explorerAdvapi.NewProc("RegDeleteTreeW")
	explorerSHChangeNotify = guiShell32.NewProc("SHChangeNotify")

	explorerOldKeys = []string{
		`Software\Classes\*\shell\OuterRar.ExtractNested`,
		`Software\Classes\*\shell\OuterRar.Extract`,
		`Software\Classes\CLSID\{DF5835F3-405C-4950-94CE-121C0EA39879}`,
	}
)

type explorerRegistryValue struct{ name, data string }

func explorerServerCommand(exe string) string {
	return `"` + exe + `" --explorer-server`
}

func explorerMenuRegistered() (bool, error) {
	exe, err := os.Executable()
	if err != nil {
		return false, err
	}
	values := []struct{ key, name, want string }{
		{explorerServerKey, "", explorerServerCommand(exe)},
		{explorerServerKey, "ServerExecutable", exe},
		{explorerCommandKey, "DelegateExecute", explorerMenuCLSID},
	}
	for _, value := range values {
		got, err := readExplorerRegistryString(value.key, value.name)
		if errors.Is(err, syscall.ERROR_FILE_NOT_FOUND) {
			return false, nil
		}
		if err != nil {
			return false, err
		}
		if !strings.EqualFold(got, value.want) {
			return false, nil
		}
	}
	return true, nil
}

func registerExplorerMenu() error {
	exe, err := os.Executable()
	if err != nil {
		return err
	}
	defer explorerSHChangeNotify.Call(0x08000000, 0, 0, 0) // SHCNE_ASSOCCHANGED
	if err := setExplorerRegistryStrings(explorerServerKey,
		explorerRegistryValue{"", explorerServerCommand(exe)},
		explorerRegistryValue{"ServerExecutable", exe}); err != nil {
		return err
	}
	if err := setExplorerRegistryStrings(explorerCommandKey, explorerRegistryValue{"DelegateExecute", explorerMenuCLSID}); err != nil {
		return err
	}
	if err := setExplorerRegistryStrings(explorerVerbKey,
		explorerRegistryValue{"MUIVerb", explorerMenuTitle},
		explorerRegistryValue{"NeverDefault", ""},
		explorerRegistryValue{"MultiSelectModel", "Player"},
		explorerRegistryValue{"AppliesTo", explorerMenuApplies}); err != nil {
		return err
	}
	return deleteExplorerRegistryTrees(explorerOldKeys...)
}

func removeExplorerMenu() error {
	defer explorerSHChangeNotify.Call(0x08000000, 0, 0, 0) // SHCNE_ASSOCCHANGED
	return errors.Join(deleteExplorerRegistryTrees(explorerVerbKey, explorerClassKey), deleteExplorerRegistryTrees(explorerOldKeys...))
}

func setExplorerRegistryStrings(path string, values ...explorerRegistryValue) error {
	var key syscall.Handle
	status, _, _ := explorerRegCreateKey.Call(syscall.HKEY_CURRENT_USER, uintptr(unsafe.Pointer(guiString(path))),
		0, 0, 0, syscall.KEY_SET_VALUE|syscall.KEY_WOW64_64KEY, 0, uintptr(unsafe.Pointer(&key)), 0)
	if status != 0 {
		return fmt.Errorf("打开注册表项 %s: %w", path, syscall.Errno(status))
	}
	defer syscall.RegCloseKey(key)
	for _, value := range values {
		data, err := syscall.UTF16FromString(value.data)
		if err != nil {
			return err
		}
		status, _, _ := explorerRegSetValue.Call(uintptr(key), uintptr(unsafe.Pointer(guiString(value.name))),
			0, syscall.REG_SZ, uintptr(unsafe.Pointer(&data[0])), uintptr(len(data)*2))
		if status != 0 {
			return fmt.Errorf("写入注册表项 %s: %w", path, syscall.Errno(status))
		}
	}
	return nil
}

func readExplorerRegistryString(path, name string) (string, error) {
	var key syscall.Handle
	if err := syscall.RegOpenKeyEx(syscall.HKEY_CURRENT_USER, guiString(path), 0, syscall.KEY_QUERY_VALUE|syscall.KEY_WOW64_64KEY, &key); err != nil {
		return "", err
	}
	defer syscall.RegCloseKey(key)
	var kind, size uint32
	if err := syscall.RegQueryValueEx(key, guiString(name), nil, &kind, nil, &size); err != nil {
		return "", err
	}
	if kind != syscall.REG_SZ || size == 0 || size%2 != 0 || size > 128<<10 {
		return "", nil
	}
	data := make([]uint16, size/2)
	if err := syscall.RegQueryValueEx(key, guiString(name), nil, &kind, (*byte)(unsafe.Pointer(&data[0])), &size); err != nil {
		return "", err
	}
	return syscall.UTF16ToString(data[:size/2]), nil
}

func deleteExplorerRegistryTrees(paths ...string) error {
	var result error
	for _, path := range paths {
		separator := strings.LastIndexByte(path, '\\')
		parent, name := path[:separator], path[separator+1:]
		var key syscall.Handle
		err := syscall.RegOpenKeyEx(syscall.HKEY_CURRENT_USER, guiString(parent), 0,
			syscall.KEY_READ|syscall.KEY_WRITE|0x00010000|syscall.KEY_WOW64_64KEY, &key) // DELETE
		if err == nil {
			status, _, _ := explorerRegDeleteTree.Call(uintptr(key), uintptr(unsafe.Pointer(guiString(name))))
			syscall.RegCloseKey(key)
			if status != 0 {
				err = syscall.Errno(status)
			}
		}
		if err != nil && !errors.Is(err, syscall.ERROR_FILE_NOT_FOUND) {
			result = errors.Join(result, fmt.Errorf("删除注册表项 %s: %w", path, err))
		}
	}
	return result
}
