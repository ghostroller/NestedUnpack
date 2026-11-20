mod archive_tree;
mod dialogs;
mod style;
mod workspace;

use std::{
    cell::OnceCell,
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender, SyncSender, TryRecvError},
    },
    thread,
    time::Duration,
};

use eframe::egui::{self, Context, Ui};

use crate::{
    i18n::{self, AppKey, Message, Preference, Text, tr},
    platform,
    session::{
        self, BatchSummary, Event, JobResult, Layer, Observer, PasswordReply, PasswordRequest,
        Status,
    },
};

use self::{archive_tree::FileTree, dialogs::Settings, style::Palette};

pub(crate) fn run(paths: Vec<PathBuf>) -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("NestedUnpack")
            .with_app_id("NestedUnpack")
            .with_inner_size([800.0, 520.0])
            .with_min_inner_size([760.0, 500.0])
            .with_drag_and_drop(true),
        centered: true,
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "NestedUnpack",
        options,
        Box::new(move |creation| {
            style::configure(&creation.egui_ctx);
            let mut app = App::default();
            app.enqueue(paths, &creation.egui_ctx);
            app.refresh_menu_language();
            Ok(Box::new(app))
        }),
    )
}

#[derive(Default)]
struct App {
    jobs: Vec<Job>,
    waiting: Vec<PathBuf>,
    stopped_waiting: Vec<PathBuf>,
    worker: Option<Worker>,
    selected: Option<usize>,
    current: Option<usize>,
    follow: bool,
    completed: Option<BatchSummary>,
    password: Option<PasswordPrompt>,
    settings: Option<Settings>,
    notice: Option<Message>,
    closing: bool,
    pick_requested: bool,
}

struct Job {
    path: PathBuf,
    status: Status,
    stage: Option<Text>,
    percent: Option<u8>,
    active_layer: Option<usize>,
    layers: BTreeMap<usize, Layer>,
    children: BTreeMap<Option<usize>, Vec<usize>>,
    file_trees: BTreeMap<usize, OnceCell<FileTree>>,
    result: Option<JobResult>,
}

impl Job {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            status: Status::Pending,
            stage: None,
            percent: None,
            active_layer: None,
            layers: BTreeMap::new(),
            children: BTreeMap::new(),
            file_trees: BTreeMap::new(),
            result: None,
        }
    }

    fn set_layer(&mut self, layer: Layer) {
        if !self.layers.contains_key(&layer.id) {
            self.children
                .entry(layer.parent)
                .or_default()
                .push(layer.id);
        }
        if let Some(contents) = &layer.contents
            && !contents.is_empty()
        {
            self.file_trees.insert(layer.id, OnceCell::new());
        }
        self.layers.insert(layer.id, layer);
    }
}

struct Worker {
    offset: usize,
    cancel: Arc<AtomicBool>,
    events: Receiver<WorkerMessage>,
}

enum WorkerMessage {
    Event(Event),
    Password {
        request: PasswordRequest,
        reply: SyncSender<Option<PasswordReply>>,
    },
    Done(BatchSummary),
}

struct UiObserver {
    ctx: Context,
    events: Sender<WorkerMessage>,
}

impl UiObserver {
    fn send(&self, event: WorkerMessage) -> bool {
        let sent = self.events.send(event).is_ok();
        if sent {
            self.ctx.request_repaint();
        }
        sent
    }
}

impl Observer for UiObserver {
    fn event(&mut self, event: Event) {
        self.send(WorkerMessage::Event(event));
    }

    fn wants_archive_contents(&self) -> bool {
        true
    }

    fn password(&mut self, request: PasswordRequest, cancel: &AtomicBool) -> Option<PasswordReply> {
        let (reply, wait) = mpsc::sync_channel(1);
        if !self.send(WorkerMessage::Password { request, reply }) {
            return None;
        }
        while !cancel.load(Ordering::Relaxed) {
            match wait.recv_timeout(Duration::from_millis(100)) {
                Ok(answer) => return answer,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        None
    }
}

struct PasswordPrompt {
    request: PasswordRequest,
    reply: SyncSender<Option<PasswordReply>>,
    value: String,
    remember: bool,
    reveal: bool,
    autofocus: bool,
}

impl App {
    fn enqueue(&mut self, paths: Vec<PathBuf>, ctx: &Context) {
        if self.closing || paths.is_empty() {
            return;
        }
        self.waiting.extend(paths);
        self.start_next(ctx);
    }

    fn start_next(&mut self, ctx: &Context) {
        if self.worker.is_some() || self.waiting.is_empty() || self.closing {
            return;
        }
        let offset = self.jobs.len();
        let paths = std::mem::take(&mut self.waiting);
        let paths_for_display = paths.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&cancel);
        let (send, events) = mpsc::channel();
        let mut observer = UiObserver {
            ctx: ctx.clone(),
            events: send,
        };
        match thread::Builder::new()
            .name("nestedunpack-worker".into())
            .spawn(move || {
                let summary = session::run_batch(paths, &stop, &mut observer);
                observer.send(WorkerMessage::Done(summary));
            }) {
            Ok(_) => {
                self.worker = Some(Worker {
                    offset,
                    cancel,
                    events,
                });
                self.jobs
                    .extend(paths_for_display.into_iter().map(Job::new));
                self.selected = Some(offset);
                self.follow = true;
                self.completed = None;
            }
            Err(error) => {
                self.notice = Some(Message::from(AppKey::WorkerStartFailed).arg(error.to_string()));
            }
        }
    }

    fn poll(&mut self, ctx: &Context) {
        for _ in 0..512 {
            let Some(worker) = &self.worker else { return };
            let offset = worker.offset;
            match worker.events.try_recv() {
                Ok(WorkerMessage::Event(event)) => self.update_event(offset, event),
                Ok(WorkerMessage::Password { request, reply }) => {
                    if self.stopping() {
                        let _ = reply.send(None);
                    } else {
                        self.password = Some(PasswordPrompt {
                            request,
                            reply,
                            value: String::new(),
                            remember: false,
                            reveal: false,
                            autofocus: true,
                        });
                    }
                }
                Ok(WorkerMessage::Done(summary)) => {
                    self.complete(offset, summary);
                    self.start_next(ctx);
                    return;
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.worker = None;
                    self.current = None;
                    self.password = None;
                    for job in &mut self.jobs[offset..] {
                        if matches!(job.status, Status::Pending | Status::Running) {
                            job.status = Status::Failed;
                            job.stage = Some(AppKey::WorkerEnded.into());
                        }
                    }
                    self.finish_waiting();
                    self.notice = Some(AppKey::WorkerEnded.into());
                    self.start_next(ctx);
                    return;
                }
            }
        }
        ctx.request_repaint();
    }

    fn update_event(&mut self, offset: usize, event: Event) {
        match event {
            Event::Batch { paths } => {
                self.jobs.truncate(offset);
                self.jobs.extend(paths.into_iter().map(Job::new));
                if self.selected.is_some_and(|index| index >= self.jobs.len()) {
                    self.selected = self.jobs.len().checked_sub(1);
                }
            }
            Event::Started { index } => {
                let index = offset + index;
                if let Some(job) = self.jobs.get_mut(index) {
                    job.status = Status::Running;
                    self.current = Some(index);
                    if self.follow || self.selected.is_none() {
                        self.selected = Some(index);
                    }
                }
            }
            Event::Stage {
                index,
                layer,
                description,
                percent,
            } => {
                if let Some(job) = self.jobs.get_mut(offset + index) {
                    job.stage = Some(description.into());
                    job.percent = percent;
                    job.active_layer = layer;
                }
            }
            Event::Archive { index, node } => {
                if let Some(job) = self.jobs.get_mut(offset + index) {
                    job.set_layer(node);
                }
            }
            Event::Finished { index, result } => {
                let index = offset + index;
                if let Some(job) = self.jobs.get_mut(index) {
                    job.status = result.status;
                    job.result = Some(result);
                }
                if self.current == Some(index) {
                    self.current = None;
                }
            }
            Event::Warning(message) => self.notice = Some(message),
            Event::Completed(summary) => self.completed = Some(summary),
        }
    }

    fn complete(&mut self, offset: usize, summary: BatchSummary) {
        if summary.canceled {
            for job in &mut self.jobs[offset..] {
                if matches!(job.status, Status::Pending | Status::Running) {
                    job.status = Status::Canceled;
                }
            }
        }
        self.worker = None;
        self.current = None;
        self.password = None;
        self.finish_waiting();
        self.completed = Some(summary);
    }

    fn stopping(&self) -> bool {
        self.worker
            .as_ref()
            .is_some_and(|worker| worker.cancel.load(Ordering::Relaxed))
    }

    fn stop(&mut self) {
        if let Some(worker) = &self.worker {
            worker.cancel.store(true, Ordering::Relaxed);
        }
        self.resolve_password(None);
        self.stopped_waiting.append(&mut self.waiting);
    }

    fn finish_waiting(&mut self) {
        self.jobs.extend(self.stopped_waiting.drain(..).map(|path| {
            let mut job = Job::new(path);
            job.status = Status::Canceled;
            job
        }));
    }

    fn resolve_password(&mut self, reply: Option<PasswordReply>) {
        if let Some(prompt) = self.password.take() {
            let _ = prompt.reply.send(reply);
        }
    }

    fn set_language(&mut self, preference: Preference) {
        if preference == i18n::preference() {
            return;
        }
        if let Err(error) = i18n::set_preference(preference) {
            self.notice =
                Some(Message::from(AppKey::SaveLanguageFailed).arg(Message::from_error(error)));
        } else {
            self.refresh_menu_language();
        }
    }

    fn refresh_menu_language(&mut self) {
        if let Err(error) = platform::refresh_menu_language() {
            self.notice = Some(
                Message::from(AppKey::UpdateMenuLanguageFailed).arg(Message::from_error(error)),
            );
        }
    }

    fn pick(&mut self, frame: &eframe::Frame, ctx: &Context) {
        if let Some(paths) = rfd::FileDialog::new()
            .set_parent(frame)
            .set_title(tr(AppKey::ChooseArchives))
            .add_filter(
                tr(AppKey::ArchivesAndVideos),
                &["zip", "zipx", "rar", "7z", "001", "mp4"],
            )
            .add_filter(tr(AppKey::AllFiles), &["*"])
            .pick_files()
        {
            self.enqueue(paths, ctx);
        }
    }

    fn open_output(&mut self, path: &Path) {
        if let Err(error) = platform::open_directory(path) {
            self.notice =
                Some(Message::from(AppKey::OpenOutputFailed).arg(Message::from_error(error)));
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
        if ctx.input(|input| input.viewport().close_requested()) && self.worker.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.closing = true;
            self.stop();
        }
        if self.closing && self.worker.is_none() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let dropped = ctx.input_mut(|input| std::mem::take(&mut input.raw.dropped_files));
        self.enqueue(
            dropped
                .into_iter()
                .map(|file| file.path().to_path_buf())
                .filter(|path| !path.as_os_str().is_empty())
                .collect(),
            &ctx,
        );
        if self.settings.is_none()
            && self.password.is_none()
            && !self.closing
            && !self.stopping()
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::O))
        {
            self.pick_requested = true;
        }
        let p = Palette::current(&ctx);
        workspace::show(self, ui, p);
        dialogs::show(self, &ctx, frame, p);
        if std::mem::take(&mut self.pick_requested) {
            self.pick(frame, &ctx);
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if let Some(worker) = &self.worker {
            worker.cancel.store(true, Ordering::Relaxed);
        }
    }
}

fn short_path(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}
