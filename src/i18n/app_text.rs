catalog! {
    AppKey {
        Pending => ("等待", "Waiting"),
        Running => ("解压中", "Extracting"),
        Success => ("成功", "Done"),
        Skipped => ("已跳过", "Skipped"),
        Failed => ("失败", "Failed"),
        Canceled => ("已停止", "Stopped"),
        Expand => ("展开 {0}", "Expand {0}"),
        Collapse => ("收起 {0}", "Collapse {0}"),
        ChooseArchives => ("选择要解压的文件", "Choose files to extract"),
        ArchivesAndVideos => ("压缩包和视频", "Archives and videos"),
        AllFiles => ("所有文件", "All files"),
        WorkerStartFailed => ("无法启动解压任务。{0}", "Couldn't start extraction. {0}"),
        WorkerEnded => ("解压任务意外结束。", "Extraction stopped unexpectedly."),
        OpenOutputFailed => ("无法打开解压目录。{0}", "Couldn't open the output folder. {0}"),
        OutputNotDirectory => ("输出路径不是文件夹", "The output path is not a folder."),
        NoConfigDirectory => ("无法定位当前用户的配置目录", "Couldn't find the current user's configuration directory."),
        SaveLanguageFailed => ("无法保存语言设置。{0}", "Couldn't save the language setting. {0}"),
        UpdateMenuLanguageFailed => ("无法更新右键菜单语言。{0}", "Couldn't update the context menu language. {0}"),
        ExplorerMenuLabel => ("解开嵌套压缩包", "Extract Nested Archives"),
        ReadSelectionFailed => ("无法读取资源管理器选区。", "Couldn't read the Explorer selection."),
        NoUsableSelection => ("选区内没有可处理的文件。", "The selection contains no files to extract."),
        OpenWindowFailed => ("无法打开解压窗口。\n{0}", "Couldn't open the extraction window.\n{0}"),
        InvalidPasswordBook => ("密码簿格式无效", "Invalid password book format."),
        #[cfg(windows)]
        EncryptPasswordBookFailed => ("无法加密密码簿。{0}", "Couldn't encrypt the password book. {0}"),
        #[cfg(windows)]
        DecryptPasswordBookFailed => ("无法解密密码簿。{0}", "Couldn't decrypt the password book. {0}"),
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        SystemPasswordStoreFailed => ("无法访问系统密码库：{0}", "Couldn't access the system password store: {0}"),
        #[cfg(windows)]
        OpenOutputShellFailed => ("无法打开输出文件夹（ShellExecute {0}）", "Couldn't open the output folder (ShellExecute {0})."),
        #[cfg(windows)]
        InvalidSelectionFile => ("选区清单路径无效", "Invalid path to the Explorer selection list."),
        #[cfg(windows)]
        InvalidSelectionEntry => ("选区清单含无效路径", "The selection list contains an invalid file path."),
        #[cfg(windows)]
        NullInText => ("文本中含空字符", "Text contains a null character."),
        #[cfg(windows)]
        ExplorerLaunchFailed => ("无法启动批量解压。\n{0}", "Couldn't start batch extraction.\n{0}"),
        #[cfg(windows)]
        InvalidExplorerFilePath => ("选区含无效的文件路径", "The Explorer selection contains an invalid file path."),
        #[cfg(windows)]
        InvalidRegistryString => ("注册表字符串无效", "Invalid registry string."),
        #[cfg(not(windows))]
        PlatformUnsupported => ("此平台不支持该操作", "This operation is not supported on this platform."),
        #[cfg(not(windows))]
        ExplorerSelectionUnsupported => ("此平台不支持资源管理器选区", "Explorer selections are not supported on this platform."),
        Usage => (
            "NestedUnpack [文件...]\nNestedUnpack --nested <文件> [更多文件...]\nNestedUnpack --outer <文件> [更多文件...]",
            "NestedUnpack [files...]\nNestedUnpack --nested <file> [more files...]\nNestedUnpack --outer <file> [more files...]"
        ),
        ConsoleFile => ("文件 {0}/{1} · {2}", "File {0}/{1} · {2}"),
        ConsoleLayer => ("第 {0} 层 · {1} · {2}", "Layer {0} · {1} · {2}"),
        ConsoleSuccess => ("解压完成 · {0} 个压缩包 · 最深 {1} 层 · {2} 个文件", "Extraction complete · Archives: {0} · Max depth: {1} · Files: {2}"),
        OutputAt => ("已解压到 {0}", "Output: {0}"),
        ConsoleFinished => (
            "处理结束 · 共 {0} 个文件 · 成功 {1} · 跳过 {2} · 失败 {3}",
            "Finished · Total: {0} · Succeeded: {1} · Skipped: {2} · Failed: {3}"
        ),
        ConsolePasswordRetry => ("密码校验失败，请重试。", "Incorrect password. Try again."),
        ConsolePasswordPrompt => ("请输入压缩包密码（留空则跳过）", "Archive password (leave blank to skip)"),
    }
}
