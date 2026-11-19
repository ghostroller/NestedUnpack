# NestedUnpack

NestedUnpack 是一个 Windows 批量解压工具，能识别普通 ZIP 和拼接在 MP4 后面的 ZIP 或 ZIP64。它会逐层解开只含一个压缩包的嵌套结构，并把同一组 7z 分卷当作一个包处理。最终文件保留包内目录结构，写入各自源文件旁的 `文件名-解压` 文件夹。同名目录会自动加编号。

## 使用

在资源管理器里选中一个或多个 `.zip`、`.mp4` 文件，通过右键菜单的 **显示更多选项 → 解开嵌套压缩包** 启动。进度窗口会显示当前层和百分比。遇到加密层时，在窗口输入密码并按 Enter，输入内容不会回显。

也可以从 PowerShell 启动同一流程。

```powershell
.\bin\NestedUnpack.exe --nested 'D:\Downloads\one.mp4' 'D:\Downloads\two.zip'
```

## 构建和注册

构建需要 Go 1.23 或更新版本，以及可从 `PATH` 调用的 LLVM MinGW `clang++`。RAR、7z 分卷及 AES ZIP 的解压使用本机的 7-Zip 命令行程序。

```powershell
.\build.ps1
.\menu.ps1
```

菜单注册作用于当前 Windows 用户，并使用这个目录下的 `bin`。移动工具后可在新目录重新运行 `menu.ps1` 更新路径。运行 `menu.ps1 -Remove` 可移除菜单注册。
