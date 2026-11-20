use crate::archive::{self, Control, ExtractError, Kind};
use crate::platform;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tempfile::{Builder, TempDir};
use zeroize::Zeroizing;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Status {
    Pending,
    Running,
    Success,
    Skipped,
    Failed,
    Canceled,
}

#[derive(Clone, Debug)]
pub(crate) struct Layer {
    pub(crate) id: usize,
    pub(crate) parent: Option<usize>,
    pub(crate) name: String,
    pub(crate) format: &'static str,
    pub(crate) status: Status,
    pub(crate) volumes: Vec<String>,
    pub(crate) files: Option<usize>,
    pub(crate) depth: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct JobResult {
    pub(crate) status: Status,
    pub(crate) output: Option<PathBuf>,
    pub(crate) detail: String,
    pub(crate) files: usize,
    pub(crate) archives: usize,
    pub(crate) depth: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct PasswordRequest {
    pub(crate) source: PathBuf,
    pub(crate) archive: PathBuf,
    pub(crate) format: &'static str,
    pub(crate) depth: usize,
    pub(crate) retry: bool,
}

pub(crate) struct PasswordReply {
    pub(crate) value: String,
    pub(crate) remember: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct BatchSummary {
    pub(crate) total: usize,
    pub(crate) success: usize,
    pub(crate) skipped: usize,
    pub(crate) failed: usize,
    pub(crate) canceled: bool,
}

impl BatchSummary {
    pub(crate) fn exit_code(self) -> i32 {
        if self.canceled {
            2
        } else {
            i32::from(self.failed != 0)
        }
    }
}

pub(crate) enum Event {
    Batch {
        paths: Vec<PathBuf>,
    },
    Started {
        index: usize,
    },
    Stage {
        index: usize,
        layer: Option<usize>,
        description: String,
        percent: Option<u8>,
    },
    Archive {
        index: usize,
        node: Layer,
    },
    Finished {
        index: usize,
        result: JobResult,
    },
    Warning(String),
    Completed(BatchSummary),
}

pub(crate) trait Observer {
    fn event(&mut self, event: Event);
    fn password(&mut self, request: PasswordRequest, cancel: &AtomicBool) -> Option<PasswordReply>;
}

pub(crate) fn run_batch(
    paths: Vec<PathBuf>,
    cancel: &AtomicBool,
    observer: &mut dyn Observer,
) -> BatchSummary {
    run(paths, cancel, observer, false)
}

pub(crate) fn run_outer(
    paths: Vec<PathBuf>,
    cancel: &AtomicBool,
    observer: &mut dyn Observer,
) -> BatchSummary {
    run(paths, cancel, observer, true)
}

fn run(
    paths: Vec<PathBuf>,
    cancel: &AtomicBool,
    observer: &mut dyn Observer,
    outer_only: bool,
) -> BatchSummary {
    let mut seen = HashSet::new();
    let paths: Vec<_> = paths
        .into_iter()
        .map(|path| std::path::absolute(&path).unwrap_or(path))
        .filter(|path| {
            let volume = !outer_only
                && volume_name(path).is_some_and(|(base, _)| {
                    extension(&base).eq_ignore_ascii_case("7z") && path.is_file()
                });
            let key = if volume {
                path.with_extension("001")
            } else {
                path.clone()
            };
            seen.insert((volume, path_key(&key)))
        })
        .collect();
    let mut summary = BatchSummary {
        total: paths.len(),
        ..BatchSummary::default()
    };
    observer.event(Event::Batch {
        paths: paths.clone(),
    });
    let stored = platform::read_passwords().unwrap_or_else(|err| {
        observer.event(Event::Warning(format!("无法读取密码簿 · {err}")));
        Vec::new()
    });
    let mut session = Session {
        observer,
        cancel,
        recent: None,
        stored: Zeroizing::new(stored),
        index: 0,
        next_layer: 1,
        archives: 0,
        depth: 0,
    };
    for (index, source) in paths.into_iter().enumerate() {
        if session.canceled() {
            summary.canceled = true;
            break;
        }
        session.index = index;
        session.archives = 0;
        session.depth = 0;
        session.observer.event(Event::Started { index });
        let result = if outer_only {
            session.extract_outer(&source)
        } else {
            session.extract_source(&source)
        };
        let (status, output, files, detail) = match result {
            Ok(Some((path, files))) => (Status::Success, Some(path), files, String::new()),
            Ok(None) => (
                Status::Skipped,
                None,
                0,
                if outer_only {
                    "未找到外层 RAR 成员"
                } else {
                    "无法识别为 ZIP、RAR 或 7z"
                }
                .into(),
            ),
            Err(_) if session.canceled() => {
                (Status::Canceled, None, 0, "已停止处理当前文件".into())
            }
            Err(ExtractError::Skipped) => (Status::Skipped, None, 0, "未输入密码".into()),
            Err(ExtractError::Canceled) => (Status::Canceled, None, 0, "已停止处理当前文件".into()),
            Err(err) => (Status::Failed, None, 0, err.to_string()),
        };
        match status {
            Status::Success => summary.success += 1,
            Status::Skipped => summary.skipped += 1,
            Status::Failed => summary.failed += 1,
            Status::Canceled => summary.canceled = true,
            Status::Pending | Status::Running => unreachable!(),
        }
        session.observer.event(Event::Finished {
            index,
            result: JobResult {
                status,
                output,
                detail,
                files,
                archives: session.archives,
                depth: session.depth,
            },
        });
        if summary.canceled {
            break;
        }
    }
    session.observer.event(Event::Completed(summary));
    summary
}

#[derive(Debug)]
struct SourceArchive {
    kind: Kind,
    parts: Vec<PathBuf>,
    base: Option<PathBuf>,
}

impl SourceArchive {
    fn single(path: PathBuf, kind: Kind) -> Self {
        Self {
            kind,
            parts: vec![path],
            base: None,
        }
    }

    fn path(&self) -> &Path {
        &self.parts[0]
    }

    fn output_name(&self) -> &Path {
        self.base.as_deref().unwrap_or_else(|| self.path())
    }

    fn format(&self) -> &'static str {
        match (self.kind, self.base.is_some()) {
            (Kind::Zip, _) => "ZIP",
            (Kind::Rar, _) => "RAR",
            (Kind::SevenZip, true) => "7z 分卷",
            (Kind::SevenZip, false) => "7z",
        }
    }
}

struct Session<'a> {
    observer: &'a mut dyn Observer,
    cancel: &'a AtomicBool,
    recent: Option<Zeroizing<String>>,
    stored: Zeroizing<Vec<String>>,
    index: usize,
    next_layer: usize,
    archives: usize,
    depth: usize,
}

impl Session<'_> {
    fn canceled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn check_cancel(&self) -> Result<(), ExtractError> {
        if self.canceled() {
            Err(ExtractError::Canceled)
        } else {
            Ok(())
        }
    }

    fn stage(&mut self, layer: Option<usize>, description: impl Into<String>, percent: Option<u8>) {
        self.observer.event(Event::Stage {
            index: self.index,
            layer,
            description: description.into(),
            percent,
        });
    }

    fn announce(&mut self, node: &Layer) {
        self.observer.event(Event::Archive {
            index: self.index,
            node: node.clone(),
        });
    }

    fn create_layer(
        &mut self,
        archive: &SourceArchive,
        parent: Option<&Layer>,
        directory: Option<&Path>,
    ) -> Layer {
        let label = |path: &Path| {
            directory
                .and_then(|root| path.strip_prefix(root).ok())
                .unwrap_or_else(|| Path::new(path.file_name().unwrap_or(path.as_os_str())))
                .to_string_lossy()
                .into_owned()
        };
        let result = Layer {
            id: self.next_layer,
            parent: parent.map(|layer| layer.id),
            name: label(archive.output_name()),
            format: archive.format(),
            status: Status::Pending,
            volumes: if archive.base.is_some() {
                archive.parts.iter().map(|part| label(part)).collect()
            } else {
                Vec::new()
            },
            files: None,
            depth: parent.map_or(1, |layer| layer.depth + 1),
        };
        self.next_layer += 1;
        self.announce(&result);
        result
    }

    fn extract_source(&mut self, source: &Path) -> Result<Option<(PathBuf, usize)>, ExtractError> {
        self.stage(None, "识别压缩格式", None);
        let Some(archive) = find_source(source)? else {
            return Ok(None);
        };
        let stage = Builder::new()
            .prefix(".nestedunpack-")
            .tempdir_in(parent(source)?)?;
        let output_name = archive.output_name().to_owned();
        let layer = self.create_layer(&archive, None, None);
        let unpacked = self.unpack_tree(source, stage.path(), archive, layer)?;
        self.check_cancel()?;
        self.stage(None, "写入解压结果", None);
        let output = publish(unpacked.directory.path(), &output_name, false)?;
        Ok(Some((output, unpacked.files)))
    }

    fn extract_outer(&mut self, source: &Path) -> Result<Option<(PathBuf, usize)>, ExtractError> {
        if archive::detect(source, true)? != Some(Kind::Zip) {
            return Ok(None);
        }
        let archive = SourceArchive::single(source.to_owned(), Kind::Zip);
        let mut layer = self.create_layer(&archive, None, None);
        let stage = Builder::new()
            .prefix(".nestedunpack-")
            .tempdir_in(parent(source)?)?;
        layer.status = Status::Running;
        self.announce(&layer);
        let result = archive::extract_outer_rars(
            source,
            stage.path(),
            &mut self.control(source, &archive, &layer),
        );
        if let Err(err) = result {
            return Err(self.layer_error(&mut layer, err));
        }
        let files = extracted_files(stage.path())?;
        self.check_cancel()?;
        let mut output = None;
        for file in &files {
            self.check_cancel()?;
            output = Some(publish(file, source, true)?);
        }
        layer.status = Status::Success;
        layer.files = Some(files.len());
        self.announce(&layer);
        self.archives = usize::from(!files.is_empty());
        self.depth = self.archives;
        Ok(output.map(|path| (path, files.len())))
    }

    fn unpack_tree(
        &mut self,
        source: &Path,
        work: &Path,
        mut archive: SourceArchive,
        mut layer: Layer,
    ) -> Result<Unpacked, ExtractError> {
        let mut previous = None;
        loop {
            self.check_cancel()?;
            if layer.depth > 64 {
                return Err(
                    self.layer_error(&mut layer, ExtractError::Invalid("嵌套层数超过 64".into()))
                );
            }
            self.depth = self.depth.max(layer.depth);
            layer.status = Status::Running;
            self.announce(&layer);
            self.stage(Some(layer.id), "读取压缩包目录", None);
            let directory = Builder::new().prefix("layer-").tempdir_in(work)?;
            let result = archive::extract(
                archive.kind,
                &archive.parts,
                directory.path(),
                &mut self.control(source, &archive, &layer),
            );
            if let Err(err) = result {
                return Err(self.layer_error(&mut layer, err));
            }
            if let Some(stage) = previous.take() {
                let stage: TempDir = stage;
                stage.close()?;
            }
            self.check_cancel()?;
            self.stage(Some(layer.id), "检查内层文件", None);
            let files = extracted_files(directory.path())?;
            let archives = find_nested(&files).map_err(|err| self.layer_error(&mut layer, err))?;
            let remaining = files.len()
                - archives
                    .iter()
                    .map(|child| child.parts.len())
                    .sum::<usize>();
            self.archives += 1;
            layer.files = Some(remaining);
            layer.status = Status::Success;
            self.announce(&layer);
            let mut children: Vec<_> = archives
                .into_iter()
                .map(|child| {
                    let node = self.create_layer(&child, Some(&layer), Some(directory.path()));
                    (child, node)
                })
                .collect();
            if children.len() == 1 && remaining == 0 {
                (archive, layer) = children.pop().expect("one child");
                previous = Some(directory);
                continue;
            }
            let mut final_files = remaining;
            for (child, node) in children {
                let output_name = child.output_name().to_owned();
                let parts = child.parts.clone();
                let nested = self.unpack_tree(source, work, child, node)?;
                self.check_cancel()?;
                self.stage(None, "整理内层解压结果", None);
                publish(nested.directory.path(), &output_name, false)?;
                for part in parts {
                    fs::remove_file(part)?;
                }
                final_files += nested.files;
            }
            return Ok(Unpacked {
                directory,
                files: final_files,
            });
        }
    }

    fn layer_error(&mut self, layer: &mut Layer, error: ExtractError) -> ExtractError {
        layer.status = match error {
            ExtractError::Canceled => Status::Canceled,
            ExtractError::Skipped => Status::Skipped,
            _ => Status::Failed,
        };
        self.announce(layer);
        match error {
            ExtractError::Canceled | ExtractError::Skipped => error,
            _ => ExtractError::Invalid(format!("第 {} 层 {} · {error}", layer.depth, layer.name)),
        }
    }

    fn control<'a>(
        &'a mut self,
        source: &Path,
        archive: &SourceArchive,
        layer: &Layer,
    ) -> LayerControl<'a> {
        LayerControl {
            index: self.index,
            layer: layer.id,
            request: PasswordRequest {
                source: source.to_owned(),
                archive: archive.path().to_owned(),
                format: layer.format,
                depth: layer.depth,
                retry: false,
            },
            cancel: self.cancel,
            observer: self.observer,
            recent: &mut self.recent,
            stored: &mut self.stored,
            candidates: VecDeque::new(),
            remember: false,
            asked: false,
            progress: None,
        }
    }
}

struct Unpacked {
    directory: TempDir,
    files: usize,
}

struct LayerControl<'a> {
    index: usize,
    layer: usize,
    request: PasswordRequest,
    cancel: &'a AtomicBool,
    observer: &'a mut dyn Observer,
    recent: &'a mut Option<Zeroizing<String>>,
    stored: &'a mut Zeroizing<Vec<String>>,
    candidates: VecDeque<Zeroizing<String>>,
    remember: bool,
    asked: bool,
    progress: Option<Option<u8>>,
}

impl LayerControl<'_> {
    fn stage(&mut self, description: &'static str, percent: Option<u8>) {
        self.observer.event(Event::Stage {
            index: self.index,
            layer: Some(self.layer),
            description: description.into(),
            percent,
        });
    }
}

impl Control for LayerControl<'_> {
    fn canceled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn progress(&mut self, done: u64, total: Option<u64>) {
        let percent = total
            .filter(|total| *total > 0)
            .map(|total| ((u128::from(done) * 100 / u128::from(total)).min(100)) as u8);
        if self.progress != Some(percent) {
            self.progress = Some(percent);
            self.stage("解压当前压缩包", percent);
        }
    }

    fn password(&mut self, retry: bool) -> Result<String, ExtractError> {
        if self.canceled() {
            return Err(ExtractError::Canceled);
        }
        self.remember = false;
        self.progress = None;
        if !retry {
            self.asked = false;
            self.candidates.clear();
            if let Some(recent) = self.recent.as_ref() {
                self.candidates.push_back(recent.clone());
            }
            self.candidates.extend(
                self.stored
                    .iter()
                    .filter(|password| {
                        self.recent
                            .as_ref()
                            .is_none_or(|recent| &**recent != *password)
                    })
                    .cloned()
                    .map(Zeroizing::new),
            );
        }
        if let Some(candidate) = self.candidates.pop_front() {
            self.stage("尝试已知密码", None);
            return Ok((*candidate).clone());
        }
        self.request.retry = self.asked && retry;
        self.stage("等待输入密码", None);
        let reply = self.observer.password(self.request.clone(), self.cancel);
        if self.canceled() {
            return Err(ExtractError::Canceled);
        }
        let reply = reply
            .filter(|entry| !entry.value.is_empty())
            .ok_or(ExtractError::Skipped)?;
        self.asked = true;
        self.remember = reply.remember;
        self.stage("验证输入的密码", None);
        Ok(reply.value)
    }

    fn accepted(&mut self, password: &str) {
        if password.is_empty() {
            return;
        }
        *self.recent = Some(Zeroizing::new(password.to_owned()));
        if self.remember {
            match platform::remember_password(password) {
                Ok(()) if !self.stored.iter().any(|saved| saved == password) => {
                    self.stored.insert(0, password.to_owned());
                }
                Err(err) => self
                    .observer
                    .event(Event::Warning(format!("密码未能保存 · {err}"))),
                _ => {}
            }
        }
        self.remember = false;
    }
}

fn extension(path: &Path) -> &str {
    path.extension().and_then(OsStr::to_str).unwrap_or("")
}

fn path_key(path: &Path) -> OsString {
    if cfg!(windows) {
        path.to_string_lossy().to_lowercase().into()
    } else {
        path.as_os_str().to_owned()
    }
}

fn parent(path: &Path) -> io::Result<&Path> {
    path.parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "无法定位源目录"))
}

fn volume_name(path: &Path) -> Option<(PathBuf, usize)> {
    let suffix = extension(path);
    let number = suffix.parse::<usize>().ok()?;
    (number != 0 && suffix == format!("{number:03}")).then(|| (path.with_extension(""), number))
}

fn find_source(source: &Path) -> Result<Option<SourceArchive>, ExtractError> {
    if let Some((base, selected)) = volume_name(source)
        && extension(&base).eq_ignore_ascii_case("7z")
    {
        let mut volumes = Vec::new();
        for entry in fs::read_dir(parent(source)?)? {
            let path = entry?.path();
            if let Some((candidate, number)) = volume_name(&path)
                && path_key(&candidate) == path_key(&base)
            {
                volumes.push((number, path));
            }
        }
        if !volumes.iter().any(|(number, _)| *number == selected) {
            return Err(io::Error::new(io::ErrorKind::NotFound, "无法读取选中的 7z 分卷").into());
        }
        return ordered_volumes(base, volumes);
    }
    Ok(archive::detect(source, true)?.map(|kind| SourceArchive::single(source.to_owned(), kind)))
}

fn find_nested(files: &[PathBuf]) -> Result<Vec<SourceArchive>, ExtractError> {
    let mut groups: HashMap<OsString, Vec<(usize, PathBuf)>> = HashMap::new();
    for file in files {
        if let Some((base, number)) = volume_name(file) {
            groups
                .entry(path_key(&base))
                .or_default()
                .push((number, file.clone()));
        }
    }
    let mut consumed = HashSet::new();
    let mut archives = Vec::new();
    for file in files {
        if consumed.contains(file) {
            continue;
        }
        if let Some((base, _)) = volume_name(file)
            && let Some(parts) = groups.remove(&path_key(&base))
            && let Some(archive) = ordered_volumes(base, parts)?
        {
            consumed.extend(archive.parts.iter().cloned());
            archives.push(archive);
            continue;
        }
        let ext = extension(file).to_ascii_lowercase();
        if [
            "epub", "cbz", "kmz", "apk", "jar", "docx", "docm", "dotx", "dotm", "xlsx", "xlsm",
            "xlsb", "xltx", "xltm", "xlam", "pptx", "pptm", "ppsx", "ppsm", "potx", "potm", "ppam",
            "odt", "ods", "odp", "odg", "odf", "ott", "ots", "otp", "otg", "vsdx", "3mf",
        ]
        .contains(&ext.as_str())
        {
            continue;
        }
        let scan_tail =
            files.len() == 1 || ["zip", "zipx", "mp4", "rar", "7z"].contains(&ext.as_str());
        if let Some(kind) = archive::detect(file, scan_tail)? {
            archives.push(SourceArchive::single(file.clone(), kind));
        }
    }
    Ok(archives)
}

fn ordered_volumes(
    base: PathBuf,
    mut volumes: Vec<(usize, PathBuf)>,
) -> Result<Option<SourceArchive>, ExtractError> {
    let named_7z = extension(&base).eq_ignore_ascii_case("7z");
    volumes.sort_by_key(|(number, _)| *number);
    let Some((1, first)) = volumes.first() else {
        return if named_7z {
            Err(ExtractError::Invalid(format!(
                "缺少 7z 首分卷 {}.001",
                base.display()
            )))
        } else {
            Ok(None)
        };
    };
    if archive::detect(first, false)? != Some(Kind::SevenZip) {
        return if named_7z {
            Err(ExtractError::Invalid(format!(
                "无法识别 7z 首分卷 {}",
                first.display()
            )))
        } else {
            Ok(None)
        };
    }
    for (index, (number, _)) in volumes.iter().enumerate() {
        if *number != index + 1 {
            return Err(ExtractError::Invalid(format!(
                "7z 分卷不连续，预期 {}.{:03}",
                base.display(),
                index + 1
            )));
        }
    }
    Ok(Some(SourceArchive {
        kind: Kind::SevenZip,
        parts: volumes.into_iter().map(|(_, path)| path).collect(),
        base: Some(base),
    }))
}

fn extracted_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut directories = vec![root.to_owned()];
    let mut files = Vec::new();
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if entry.metadata()?.file_attributes() & 0x400 != 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "解压结果中存在重解析点",
                    ));
                }
            }
            if kind.is_dir() {
                directories.push(entry.path());
            } else if kind.is_file() {
                files.push(entry.path());
            } else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "不支持的解压成员类型",
                ));
            }
        }
    }
    files.sort();
    Ok(files)
}

fn publish(from: &Path, source: &Path, rar: bool) -> io::Result<PathBuf> {
    let directory = parent(source)?;
    let file_stem = source
        .file_stem()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "无法定位源文件名"))?
        .to_string_lossy();
    let mut stem = file_stem.as_ref();
    let mut prefix = "";
    if !rar {
        if cfg!(windows) {
            stem = stem.trim_end_matches([' ', '.']);
            if archive::windows_device_name(stem) {
                prefix = "_";
            }
        }
        if matches!(stem, "" | "." | "..") {
            stem = "解压结果";
        }
    }
    for number in 1.. {
        let suffix = match (rar, number) {
            (false, 1) => String::new(),
            (true, 1) => ".rar".into(),
            (false, _) => format!(" ({number})"),
            (true, _) => format!(" ({number}).rar"),
        };
        let mut limit = 255 - units(prefix) - units(&suffix);
        let end = stem
            .char_indices()
            .find_map(|(index, ch)| {
                let needed = if cfg!(windows) {
                    ch.len_utf16()
                } else {
                    ch.len_utf8()
                };
                limit.checked_sub(needed).map_or(Some(index), |left| {
                    limit = left;
                    None
                })
            })
            .unwrap_or(stem.len());
        let to = directory.join(format!("{prefix}{}{suffix}", &stem[..end]));
        match rename_new(from, &to) {
            Ok(()) => return Ok(to),
            Err(err)
                if err.kind() == io::ErrorKind::AlreadyExists || to.symlink_metadata().is_ok() =>
            {
                continue;
            }
            Err(err) => return Err(err),
        }
    }
    unreachable!()
}

fn units(value: &str) -> usize {
    if cfg!(windows) {
        value.encode_utf16().count()
    } else {
        value.len()
    }
}

#[cfg(windows)]
fn rename_new(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::MoveFileW;
    use windows::core::PCWSTR;

    let long_path = |path: &Path| {
        let mut path: Vec<_> = path.as_os_str().encode_wide().collect();
        const BACKSLASH: u16 = b'\\' as u16;
        if !path.starts_with(&[BACKSLASH, BACKSLASH, b'?' as u16, BACKSLASH]) {
            let prefix = if path.starts_with(&[BACKSLASH, BACKSLASH]) {
                path.drain(..2);
                r"\\?\UNC\"
            } else {
                r"\\?\"
            };
            path.splice(..0, prefix.encode_utf16());
        }
        path.push(0);
        path
    };
    let source = long_path(from);
    let destination = long_path(to);
    // Same-volume atomic rename that fails if the destination already exists.
    unsafe { MoveFileW(PCWSTR(source.as_ptr()), PCWSTR(destination.as_ptr())) }
        .map_err(|err| io::Error::from_raw_os_error((err.code().0 as u32 & 0xffff) as i32))
}

#[cfg(target_os = "linux")]
fn rename_new(from: &Path, to: &Path) -> io::Result<()> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};
    renameat_with(CWD, from, CWD, to, RenameFlags::NOREPLACE).map_err(Into::into)
}

#[cfg(target_os = "macos")]
fn rename_new(from: &Path, to: &Path) -> io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let from = CString::new(from.as_os_str().as_bytes())?;
    let to = CString::new(to.as_os_str().as_bytes())?;
    // The temporary and final paths are on the same volume.
    if unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
