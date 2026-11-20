#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

mod archive;
mod platform;
mod rar;
mod session;
mod ui;

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

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
                notify_error("无法读取资源管理器选区。");
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
                    notify_error("选区内没有可处理的文件。");
                    2
                }
                Err(err) => {
                    notify_error(&format!("无法读取资源管理器选区。\n{err}"));
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
    println!("NestedUnpack [文件...]");
    println!("NestedUnpack --nested <文件> [更多文件...]");
    println!("NestedUnpack --outer <文件> [更多文件...]");
}

fn run_gui(paths: Vec<PathBuf>) -> i32 {
    match ui::run(paths) {
        Ok(()) => 0,
        Err(err) => {
            notify_error(&format!("无法打开解压窗口。\n{err}"));
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
                    "\n文件 {}/{} · {}",
                    index + 1,
                    self.paths.len(),
                    self.paths[index].display()
                );
            }
            Event::Stage {
                description,
                percent,
                ..
            } => {
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
                println!("第 {} 层 · {} · {}", node.depth, node.format, node.name);
            }
            Event::Finished { result, .. } => {
                self.end_line();
                if let Some(output) = result.output {
                    println!(
                        "解压完成 · {} 个压缩包 · 最深 {} 层 · {} 个文件",
                        result.archives, result.depth, result.files
                    );
                    println!("已解压到 {}", output.display());
                } else {
                    let label = match result.status {
                        Status::Skipped => "跳过",
                        Status::Canceled => "已取消",
                        _ => "失败",
                    };
                    println!("{label} · {}", result.detail);
                }
            }
            Event::Warning(message) => {
                self.end_line();
                println!("{message}");
            }
            Event::Completed(summary) => {
                self.end_line();
                println!(
                    "\n处理结束 · 共 {} 个文件 · 成功 {} 个 · 跳过 {} 个 · 失败 {} 个",
                    summary.total, summary.success, summary.skipped, summary.failed
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
            println!("密码校验失败，请重试。");
        }
        print!("请输入压缩包密码（留空则跳过）");
        let _ = io::stdout().flush();
        let value = platform::read_console_password().ok()?;
        println!();
        (!value.is_empty()).then_some(session::PasswordReply {
            value,
            remember: false,
        })
    }
}
