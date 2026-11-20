# NestedUnpack

NestedUnpack 是一个批量解压工具，支持 ZIP、ZIP64、RAR、7z、7z 分卷，以及拼接在 MP4 后面的 ZIP 或 ZIP64。它会逐层解开识别出的压缩包。单个内层包会直接展开。若还有其他文件，内层包会解压到同级子目录，原有文件会保留。结果写到源文件旁的 `文件名-解压` 目录，遇到同名目录会递增编号。

## Windows 用法

- 双击 `NestedUnpack.exe` 打开工作台。点击 **选择文件** 或把文件拖入窗口即可解压。
- 把一个或多个文件拖到 EXE 图标上，可直接打开工作台并处理这些文件。拖入任意一个 7z 分卷即可自动查找同目录的其余分卷，同组只处理一次。
- 打开 **右键菜单**，为当前用户注册 **解开嵌套压缩包**。Windows 11 可从 **显示更多选项** 找到该命令。移动 EXE 后，在新位置重新注册即可更新菜单指向。
- 打开 **密码簿** 可添加、查看或删除条目。程序会优先尝试本批次验证过的密码和密码簿中的密码。手动输入时，可以选择在密码验证成功后保存。原有 Windows 密码簿继续使用同一个 DPAPI 文件 `%LOCALAPPDATA%\NestedUnpack\passwords.dpapi`。
- 任务列表显示每个源文件的状态。选中任务可查看嵌套包、分卷、各层状态和输出目录。界面默认使用系统主题，可手动切换浅色和深色。

### 命令行

直接传文件路径会打开工作台。使用 `--nested` 可在终端递归解压，使用 `--outer` 可只提取外层 ZIP 内的 RAR。

```powershell
Start-Process -FilePath .\bin\NestedUnpack.exe -NoNewWindow -Wait -ArgumentList '--nested', '"D:\Downloads\one.mp4"', '"D:\Downloads\two.zip"'
```

## 构建和分发

Windows 构建需要 Rust 1.97.1 或更新版本、`x86_64-pc-windows-msvc` 目标，以及 Visual Studio C++ Build Tools 与 Windows SDK。在 PowerShell 中运行以下命令。

```powershell
.\build.ps1
cargo test --locked --target x86_64-pc-windows-msvc
```

分发 `bin\NestedUnpack.exe` 即可。ZIP、7z 和 UnRAR 解码器随程序构建，目标机器无需安装 7-Zip。第三方许可可在应用内、通过 `--licenses` 或在源码清单中查看。 [第三方许可清单](THIRD-PARTY-NOTICES.txt)

NestedUnpack 使用 Rust 和 egui/eframe 的 glow 后端，任务表使用 egui Extras 的 TableBuilder，图标来自 Phosphor。解压流程与窗口、文件关联和密码存储互相独立。macOS 和 Linux 的密码存储接入系统钥匙串或 Secret Service。当前发布与完整验证以 Windows x64 为准，资源管理器右键菜单仅在 Windows 提供。 [egui Extras](https://docs.rs/egui_extras/0.36.2/egui_extras/struct.TableBuilder.html) · [Phosphor Icons](https://phosphoricons.com/)
