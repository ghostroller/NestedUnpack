use super::format;

catalog! {
    UiKey {
        Language => ("语言", "Language"),
        FollowSystem => ("跟随系统", "System language"),
        SimplifiedChinese => ("简体中文", "简体中文"),
        English => ("English", "English"),
        LightTheme => ("切换到浅色主题", "Switch to light theme"),
        DarkTheme => ("切换到深色主题", "Switch to dark theme"),
        ContextMenu => ("右键菜单", "Context menu"),
        Licenses => ("许可证", "Licenses"),
        PasswordBook => ("密码簿", "Password book"),
        CurrentBatch => ("正在处理当前批次", "Processing current batch"),
        WaitingFile => ("另有 {0} 个文件等待解压", "{0} more file waiting"),
        WaitingFiles => ("另有 {0} 个文件等待解压", "{0} more files waiting"),
        BatchStopped => ("已停止", "Stopped"),
        BatchCompleted => ("处理完毕", "Complete"),
        BatchSummary => ("{0} · 共 {1} · 成功 {2} · 跳过 {3} · 失败 {4}", "{0} · {1} total · {2} succeeded · {3} skipped · {4} failed"),
        TotalFiles => ("共 {0}", "{0} total"),
        OneFile => ("{0} 个文件", "{0} file"),
        ManyFiles => ("{0} 个文件", "{0} files"),
        OneArchive => ("{0} 个压缩包", "{0} archive"),
        ManyArchives => ("{0} 个压缩包", "{0} archives"),
        OneVolume => ("{0} 卷", "{0} volume"),
        ManyVolumes => ("{0} 卷", "{0} volumes"),
        DismissNotice => ("关闭提示", "Dismiss notice"),
        ReleaseToExtract => ("松开鼠标开始解压", "Release to extract"),
        DropToExtract => ("拖入压缩包开始解压", "Drop archives to extract"),
        SupportedFormats => ("支持 ZIP、RAR、7z、7z 分卷和带 ZIP 的 MP4 文件", "Supports ZIP, RAR, 7z, split 7z, and MP4 files with ZIP data"),
        ChooseFiles => ("选择文件", "Choose files"),
        ExtractionQueue => ("解压队列", "Extraction queue"),
        AddFiles => ("添加文件", "Add files"),
        Stopping => ("正在停止", "Stopping"),
        StopExtraction => ("停止解压", "Stop extraction"),
        CleaningUp => ("正在停止并清理暂存文件", "Stopping and cleaning up temporary files"),
        IdentifyingArchive => ("正在识别压缩包", "Identifying archive"),
        LayerProgress => ("第 {0} 层 · {1} · {2}", "Layer {0} · {1} · {2}"),
        WaitingForFiles => ("等待处理文件", "Waiting for files"),
        PreparingFiles => ("正在准备文件", "Preparing files"),
        File => ("文件", "File"),
        Status => ("状态", "Status"),
        Progress => ("进度", "Progress"),
        ArchiveSummary => ("{0} · {1} · 最深 {2} 层", "{0} · {1} · Depth {2}"),
        OpenOutputFolder => ("打开解压目录", "Open output folder"),
        AwaitingArchive => ("等待识别压缩包", "Waiting to identify archive"),
        NoArchiveContents => ("无压缩包结构", "No archive contents"),
        Contents => ("文件结构", "Contents"),
        ContentsCount => ("文件结构 · {0}", "Contents · {0}"),
        ReadPasswordBookFailed => ("无法读取密码簿。{0}", "Couldn't read the password book. {0}"),
        SavePasswordFailed => ("无法保存密码。{0}", "Couldn't save the password. {0}"),
        DeletePasswordFailed => ("无法删除密码。{0}", "Couldn't delete the password. {0}"),
        ImportPasswords => ("导入", "Import"),
        ExportPasswords => ("导出", "Export"),
        ImportPasswordsTitle => ("导入密码簿", "Import password book"),
        ExportPasswordsTitle => ("导出密码簿", "Export password book"),
        PasswordBackupJson => ("密码簿备份（JSON）", "Password book backup (JSON)"),
        PasswordBackupPlaintext => ("导出的 JSON 文件包含明文密码。", "The exported JSON contains plain text passwords."),
        PasswordsImported => ("已导入 {0} 个新密码", "Added {0} to the password book"),
        PasswordsExported => ("已导出密码簿", "Password book exported"),
        PasswordImportFailed => ("无法导入密码簿。{0}", "Couldn't import the password book. {0}"),
        PasswordExportFailed => ("无法导出密码簿。{0}", "Couldn't export the password book. {0}"),
        ArchivePassword => ("压缩包密码", "Archive password"),
        PasswordContext => ("{0} · 第 {1} 层 · {2}", "{0} · Layer {1} · {2}"),
        PasswordRetry => ("密码验证未通过，请重试。", "Incorrect password. Try again."),
        EnterPassword => ("输入密码", "Enter password"),
        HidePassword => ("隐藏密码", "Hide password"),
        ShowPassword => ("显示密码", "Show password"),
        RememberPassword => ("验证成功后保存到密码簿", "Save to the password book if correct"),
        ContinueExtraction => ("继续解压", "Continue"),
        SkipFile => ("跳过此文件", "Skip file"),
        PasswordSaved => ("已保存 {0} 个密码", "{0} saved password"),
        PasswordsSaved => ("已保存 {0} 个密码", "{0} saved passwords"),
        EmptyPasswordBook => ("密码簿为空", "No saved passwords"),
        DeletePasswordConfirmation => ("删除所选密码？", "Delete selected password?"),
        ConfirmDelete => ("确认删除", "Delete"),
        KeepPassword => ("保留", "Keep"),
        AddPassword => ("添加密码", "Add a password"),
        Add => ("添加", "Add"),
        Close => ("关闭", "Close"),
        DeleteSelected => ("删除所选", "Delete selected"),
        ExplorerContextMenu => ("资源管理器右键菜单", "File Explorer context menu"),
        MenuExplanation => ("为当前 Windows 用户添加「{0}」命令。", "Adds “{0}” for the current Windows user."),
        MenuRegistered => ("已注册到当前位置", "Registered for this location"),
        MenuUnregistered => ("未注册到当前位置", "Not registered for this location"),
        RegisterMenu => ("注册到当前位置", "Register here"),
        RemoveMenu => ("移除菜单", "Remove menu"),
        CheckMenuFailed => ("无法检查右键菜单。{0}", "Couldn't check the context menu. {0}"),
        RegisterMenuFailed => ("无法注册右键菜单。{0}", "Couldn't register the context menu. {0}"),
        RemoveMenuFailed => ("无法移除右键菜单。{0}", "Couldn't remove the context menu. {0}")
    }
}

fn quantity(count: usize, singular: UiKey, plural: UiKey) -> String {
    format(if count == 1 { singular } else { plural }, &[&count])
}

pub(crate) fn file_count(count: usize) -> String {
    quantity(count, UiKey::OneFile, UiKey::ManyFiles)
}

pub(crate) fn archives_count(count: usize) -> String {
    quantity(count, UiKey::OneArchive, UiKey::ManyArchives)
}

pub(crate) fn volume_count(count: usize) -> String {
    quantity(count, UiKey::OneVolume, UiKey::ManyVolumes)
}

pub(crate) fn waiting_files(count: usize) -> String {
    quantity(count, UiKey::WaitingFile, UiKey::WaitingFiles)
}

pub(crate) fn saved_passwords(count: usize) -> String {
    quantity(count, UiKey::PasswordSaved, UiKey::PasswordsSaved)
}

pub(crate) fn archive_summary(files: usize, archives: usize, depth: usize) -> String {
    format(
        UiKey::ArchiveSummary,
        &[&file_count(files), &archives_count(archives), &depth],
    )
}
