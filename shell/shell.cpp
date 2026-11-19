#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <objbase.h>
#include <shellapi.h>
#include <shobjidl.h>

#include <new>
#include <string>

namespace {

const CLSID commandClass = {0xdf5835f3, 0x405c, 0x4950, {0x94, 0xce, 0x12, 0x1c, 0x0e, 0xa3, 0x98, 0x79}};
const CLSID nestedCommandClass = {0x9e941531, 0x6a19, 0x4358, {0xa7, 0x73, 0x30, 0xcd, 0x66, 0x4b, 0xe0, 0x34}};
enum class Mode { outer, nested };
constexpr UINT runBatchMessage = WM_APP + 1;
DWORD serverThread = 0;
unsigned int liveCommands = 0;
ULONGLONG lastActivity = 0;
bool handledActivation = false;

HRESULT lastError() {
    const DWORD error = GetLastError();
    return error == ERROR_SUCCESS ? E_FAIL : HRESULT_FROM_WIN32(error);
}

HRESULT selectionBytes(IShellItemArray* selection, std::string& paths) {
    DWORD count = 0;
    HRESULT result = selection->GetCount(&count);
    if (FAILED(result)) return result;
    if (count == 0) return HRESULT_FROM_WIN32(ERROR_NO_DATA);

    for (DWORD index = 0; index < count; ++index) {
        IShellItem* item = nullptr;
        result = selection->GetItemAt(index, &item);
        if (FAILED(result)) return result;

        PWSTR path = nullptr;
        result = item->GetDisplayName(SIGDN_FILESYSPATH, &path);
        item->Release();
        if (FAILED(result)) return result;

        const int size = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, path, -1, nullptr, 0, nullptr, nullptr);
        if (size <= 1) {
            result = size == 0 ? lastError() : E_INVALIDARG;
            CoTaskMemFree(path);
            return result;
        }
        try {
            const size_t offset = paths.size();
            paths.resize(offset + static_cast<size_t>(size));
            if (WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, path, -1, paths.data() + offset, size, nullptr, nullptr) != size) {
                result = lastError();
            }
            paths.back() = '\n';
        } catch (...) {
            CoTaskMemFree(path);
            throw;
        }
        CoTaskMemFree(path);
        if (FAILED(result)) return result;
    }
    return S_OK;
}

HRESULT launchExtractor(IShellItemArray* selection, Mode mode) {
    std::string paths;
    HRESULT result = selectionBytes(selection, paths);
    if (FAILED(result)) return result;
    if (paths.size() > MAXDWORD) return HRESULT_FROM_WIN32(ERROR_BUFFER_OVERFLOW);

    std::wstring buffer(32768, L'\0');
    const DWORD exeSize = GetModuleFileNameW(nullptr, buffer.data(), static_cast<DWORD>(buffer.size()));
    if (exeSize == 0 || exeSize >= buffer.size()) return lastError();
    const std::wstring shellExe = buffer.substr(0, exeSize);
    const std::wstring extractor = shellExe.substr(0, shellExe.find_last_of(L"\\/") + 1) +
        (mode == Mode::nested ? L"NestedUnpackGUI.exe" : L"NestedUnpack.exe");

    const DWORD tempSize = GetTempPathW(static_cast<DWORD>(buffer.size()), buffer.data());
    if (tempSize == 0 || tempSize >= buffer.size()) return lastError();
    GUID batchId;
    result = CoCreateGuid(&batchId);
    if (FAILED(result)) return result;
    wchar_t guid[39];
    if (StringFromGUID2(batchId, guid, 39) == 0) return E_FAIL;
    const std::wstring manifest = buffer.substr(0, tempSize) + L"NestedUnpack-Selection-" + guid + L".utf8";
    const wchar_t* option = mode == Mode::nested ? L"--nested-selection-file" : L"--selection-file";
    std::wstring arguments = L"\"" + extractor + L"\" " + option + L" \"" + manifest + L"\"";

    HANDLE file = CreateFileW(manifest.c_str(), GENERIC_WRITE, 0, nullptr, CREATE_NEW, FILE_ATTRIBUTE_TEMPORARY, nullptr);
    if (file == INVALID_HANDLE_VALUE) return lastError();
    DWORD written = 0;
    if (!WriteFile(file, paths.data(), static_cast<DWORD>(paths.size()), &written, nullptr)) {
        result = lastError();
    } else if (written != paths.size()) {
        result = HRESULT_FROM_WIN32(ERROR_WRITE_FAULT);
    }
    CloseHandle(file);

    if (SUCCEEDED(result)) {
        STARTUPINFOW startup{};
        startup.cb = sizeof(startup);
        PROCESS_INFORMATION process{};
        const DWORD flags = mode == Mode::nested ? 0 : CREATE_NO_WINDOW;
        if (CreateProcessW(extractor.c_str(), arguments.data(), nullptr, nullptr, FALSE, flags, nullptr, nullptr, &startup, &process)) {
            CloseHandle(process.hThread);
            CloseHandle(process.hProcess);
            return S_OK;
        }
        result = lastError();
    }
    DeleteFileW(manifest.c_str());
    return result;
}

class Command final : public IExecuteCommand, public IObjectWithSelection {
public:
    explicit Command(Mode mode) : mode_(mode) { ++liveCommands; }

    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID iid, void** object) override {
        if (!object) return E_POINTER;
        *object = nullptr;
        if (IsEqualIID(iid, IID_IUnknown) || IsEqualIID(iid, IID_IExecuteCommand)) {
            *object = static_cast<IExecuteCommand*>(this);
        } else if (IsEqualIID(iid, IID_IObjectWithSelection)) {
            *object = static_cast<IObjectWithSelection*>(this);
        } else {
            return E_NOINTERFACE;
        }
        AddRef();
        return S_OK;
    }

    ULONG STDMETHODCALLTYPE AddRef() override { return ++references_; }

    ULONG STDMETHODCALLTYPE Release() override {
        const ULONG remaining = --references_;
        if (remaining == 0) delete this;
        return remaining;
    }

    HRESULT STDMETHODCALLTYPE SetKeyState(DWORD) override { return S_OK; }
    HRESULT STDMETHODCALLTYPE SetParameters(LPCWSTR) override { return S_OK; }
    HRESULT STDMETHODCALLTYPE SetPosition(POINT) override { return S_OK; }
    HRESULT STDMETHODCALLTYPE SetShowWindow(int) override { return S_OK; }
    HRESULT STDMETHODCALLTYPE SetNoShowUI(BOOL noUI) override { noUI_ = noUI != FALSE; return S_OK; }
    HRESULT STDMETHODCALLTYPE SetDirectory(LPCWSTR) override { return S_OK; }

    HRESULT STDMETHODCALLTYPE SetSelection(IShellItemArray* selection) override {
        if (selection) selection->AddRef();
        if (selection_) selection_->Release();
        selection_ = selection;
        return S_OK;
    }

    HRESULT STDMETHODCALLTYPE GetSelection(REFIID iid, void** object) override {
        if (!object) return E_POINTER;
        *object = nullptr;
        return selection_ ? selection_->QueryInterface(iid, object) : E_FAIL;
    }

    HRESULT STDMETHODCALLTYPE Execute() override {
        if (!selection_ || scheduled_) return E_UNEXPECTED;
        AddRef();
        if (!PostThreadMessageW(serverThread, runBatchMessage, reinterpret_cast<WPARAM>(this), 0)) {
            const HRESULT result = lastError();
            Release();
            return result;
        }
        scheduled_ = true;
        return S_OK;
    }

    void runBatch() {
        HRESULT result;
        try {
            result = launchExtractor(selection_, mode_);
        } catch (const std::bad_alloc&) {
            result = E_OUTOFMEMORY;
        } catch (...) {
            result = E_FAIL;
        }
        if (FAILED(result) && !noUI_) {
            wchar_t message[160];
            swprintf(message, 160, L"\u65e0\u6cd5\u542f\u52a8\u6279\u91cf\u89e3\u538b\u3002\n\nHRESULT 0x%08lX", static_cast<unsigned long>(result));
            MessageBoxW(nullptr, message, L"NestedUnpack", MB_OK | MB_ICONERROR | MB_SETFOREGROUND);
        }
    }

private:
    ~Command() {
        if (selection_) selection_->Release();
        --liveCommands;
        lastActivity = GetTickCount64();
    }

    const Mode mode_;
    ULONG references_ = 1;
    IShellItemArray* selection_ = nullptr;
    bool scheduled_ = false;
    bool noUI_ = false;
};

class Factory final : public IClassFactory {
public:
    explicit Factory(Mode mode) : mode_(mode) {}

    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID iid, void** object) override {
        if (!object) return E_POINTER;
        *object = nullptr;
        if (!IsEqualIID(iid, IID_IUnknown) && !IsEqualIID(iid, IID_IClassFactory)) return E_NOINTERFACE;
        *object = static_cast<IClassFactory*>(this);
        AddRef();
        return S_OK;
    }

    // This factory lives on the server stack until after CoRevokeClassObject.
    ULONG STDMETHODCALLTYPE AddRef() override { return 2; }
    ULONG STDMETHODCALLTYPE Release() override { return 1; }

    HRESULT STDMETHODCALLTYPE CreateInstance(IUnknown* outer, REFIID iid, void** object) override {
        if (!object) return E_POINTER;
        *object = nullptr;
        if (outer) return CLASS_E_NOAGGREGATION;
        handledActivation = true;
        Command* command = new (std::nothrow) Command(mode_);
        if (!command) return E_OUTOFMEMORY;
        const HRESULT result = command->QueryInterface(iid, object);
        command->Release();
        return result;
    }

    HRESULT STDMETHODCALLTYPE LockServer(BOOL) override { return S_OK; }

private:
    const Mode mode_;
};

int serve(Mode mode) {
    serverThread = GetCurrentThreadId();
    lastActivity = GetTickCount64();
    MSG message{};
    PeekMessageW(&message, nullptr, 0, 0, PM_NOREMOVE);
    const UINT_PTR timer = SetTimer(nullptr, 0, 1000, nullptr);
    if (!timer) return 1;

    Factory factory(mode);
    const CLSID& clsid = mode == Mode::nested ? nestedCommandClass : commandClass;
    DWORD registration = 0;
    const HRESULT result = CoRegisterClassObject(clsid, &factory, CLSCTX_LOCAL_SERVER, REGCLS_SINGLEUSE, &registration);
    if (FAILED(result)) {
        KillTimer(nullptr, timer);
        return 1;
    }
    BOOL status;
    while ((status = GetMessageW(&message, nullptr, 0, 0)) > 0) {
        if (message.message == runBatchMessage) {
            auto* command = reinterpret_cast<Command*>(message.wParam);
            command->runBatch();
            command->Release();
        } else if (message.message == WM_TIMER && message.wParam == timer) {
            const ULONGLONG idleLimit = handledActivation ? 5000 : 30000;
            if (liveCommands == 0 && GetTickCount64() - lastActivity >= idleLimit) break;
        } else {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    CoRevokeClassObject(registration);
    KillTimer(nullptr, timer);
    return status < 0 ? 1 : 0;
}

}  // namespace

int WINAPI wWinMain(HINSTANCE, HINSTANCE, PWSTR, int) {
    int argc = 0;
    wchar_t** argv = CommandLineToArgvW(GetCommandLineW(), &argc);
    if (!argv) return 1;
    const bool nested = argc == 3 && wcscmp(argv[1], L"--nested") == 0;
    const bool valid = (argc == 2 || nested) &&
        (_wcsicmp(argv[argc - 1], L"-Embedding") == 0 || _wcsicmp(argv[argc - 1], L"/Embedding") == 0);
    LocalFree(argv);
    if (!valid) return 1;

    const HRESULT result = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
    if (FAILED(result)) return 1;
    const int status = serve(nested ? Mode::nested : Mode::outer);
    CoUninitialize();
    return status;
}
