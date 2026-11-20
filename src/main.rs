#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

mod archive;
mod i18n;
mod platform;
mod rar;
mod session;
mod ui;

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use crate::i18n::{AppKey, format as localize, tr};

pub(crate) const THIRD_PARTY_NOTICES: &str = include_str!("../THIRD-PARTY-NOTICES.txt");

fn main() {
    let code = run(std::env::args_os().skip(1).collect());
    std::process::exit(code);
}

fn run(mut args: Vec<OsString>) -> i32 {
    let option = args.first().and_then(|arg| arg.to_str()).unwrap_or("");
    match option {
        "--explorer-server" => {
            if args.len() == 2
                && args[1].to_str().is_some_and(|arg| {
                    arg.eq_ignore_ascii_case("-Embedding") || arg.eq_ignore_ascii_case("/Embedding")
                })
            {
                platform::run_explorer_server()
            } else {
                2
            }
        }
        "--nested-selection-file" | "--selection-file" => {
            let outer_only = option == "--selection-file";
            if args.len() != 2 {
                notify_error(tr(AppKey::ReadSelectionFailed));
                return 2;
            }
            match platform::read_selection(Path::new(&args[1])) {
                Ok(paths) if !paths.is_empty() => {
                    if outer_only {
                        run_console(paths, true)
                    } else {
                        run_gui(paths)
                    }
                }
                Ok(_) => {
                    notify_error(tr(AppKey::NoUsableSelection));
                    2
                }
                Err(err) => {
                    notify_error(&format!("{}\n{err}", tr(AppKey::ReadSelectionFailed)));
                    2
                }
            }
        }
        "--nested" | "--outer" => {
            let outer_only = option == "--outer";
            if args.len() < 2 {
                platform::attach_console();
                print_usage();
                return 2;
            }
            run_console(args.drain(1..).map(PathBuf::from).collect(), outer_only)
        }
        "--licenses" => {
            platform::attach_console();
            print!("{THIRD_PARTY_NOTICES}");
            0
        }
        "--help" | "-h" => {
            platform::attach_console();
            print_usage();
            0
        }
        "--version" => {
            platform::attach_console();
            println!("NestedUnpack {}", env!("CARGO_PKG_VERSION"));
            0
        }
        _ => run_gui(args.into_iter().map(PathBuf::from).collect()),
    }
}

fn print_usage() {
    println!("{}", tr(AppKey::Usage));
}

fn run_gui(paths: Vec<PathBuf>) -> i32 {
    match ui::run(paths) {
        Ok(()) => 0,
        Err(err) => {
            notify_error(&localize(AppKey::OpenWindowFailed, &[&err]));
            1
        }
    }
}

fn notify_error(message: &str) {
    let _ = rfd::MessageDialog::new()
        .set_title("NestedUnpack")
        .set_level(rfd::MessageLevel::Error)
        .set_description(message)
        .show();
}

fn run_console(paths: Vec<PathBuf>, outer_only: bool) -> i32 {
    platform::attach_console();
    let cancel = AtomicBool::new(false);
    let mut console = Console {
        paths: Vec::new(),
        progress: false,
    };
    let summary = if outer_only {
        session::run_outer(paths, &cancel, &mut console)
    } else {
        session::run_batch(paths, &cancel, &mut console)
    };
    summary.exit_code()
}

struct Console {
    paths: Vec<PathBuf>,
    progress: bool,
}

impl Console {
    fn end_line(&mut self) {
        if self.progress {
            println!();
            self.progress = false;
        }
    }
}

impl session::Observer for Console {
    fn event(&mut self, event: session::Event) {
        use session::{Event, Status};
        match event {
            Event::Batch { paths } => self.paths = paths,
            Event::Started { index } => {
                self.end_line();
                println!(
                    "\n{}",
                    localize(
                        AppKey::ConsoleFile,
                        &[
                            &(index + 1),
                            &self.paths.len(),
                            &self.paths[index].display()
                        ],
                    )
                );
            }
            Event::Stage {
                description,
                percent,
                ..
            } => {
                let description = tr(description);
                if let Some(percent) = percent {
                    print!("\r{description} {percent:>3}%");
                    let _ = io::stdout().flush();
                    self.progress = true;
                } else {
                    self.end_line();
                    println!("{description}");
                }
            }
            Event::Archive { node, .. } if node.status == Status::Running => {
                self.end_line();
                println!(
                    "{}",
                    localize(
                        AppKey::ConsoleLayer,
                        &[&node.depth, &tr(node.format), &node.name]
                    )
                );
            }
            Event::Finished { result, .. } => {
                self.end_line();
                if let Some(output) = result.output {
                    println!(
                        "{}",
                        localize(
                            AppKey::ConsoleSuccess,
                            &[&result.archives, &result.depth, &result.files]
                        )
                    );
                    println!("{}", localize(AppKey::OutputAt, &[&output.display()]));
                } else {
                    let label = tr(match result.status {
                        Status::Skipped => AppKey::Skipped,
                        Status::Canceled => AppKey::Canceled,
                        _ => AppKey::Failed,
                    });
                    if let Some(detail) = result.detail {
                        println!("{label} · {detail}");
                    } else {
                        println!("{label}");
                    }
                }
            }
            Event::Warning(message) => {
                self.end_line();
                println!("{message}");
            }
            Event::Completed(summary) => {
                self.end_line();
                println!(
                    "\n{}",
                    localize(
                        AppKey::ConsoleFinished,
                        &[
                            &summary.total,
                            &summary.success,
                            &summary.skipped,
                            &summary.failed
                        ],
                    )
                );
            }
            Event::Archive { .. } => {}
        }
    }

    fn password(
        &mut self,
        request: session::PasswordRequest,
        _: &AtomicBool,
    ) -> Option<session::PasswordReply> {
        self.end_line();
        if request.retry {
            println!("{}", tr(AppKey::ConsolePasswordRetry));
        }
        print!("{} ", tr(AppKey::ConsolePasswordPrompt));
        let _ = io::stdout().flush();
        let value = platform::read_console_password().ok()?;
        println!();
        (!value.is_empty()).then_some(session::PasswordReply {
            value,
            remember: false,
        })
    }
}
