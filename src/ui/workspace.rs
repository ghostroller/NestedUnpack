use eframe::egui::{
    self, Align, Button, CentralPanel, Color32, Frame, Label, Layout, Margin, Panel, RichText,
    ScrollArea, Sense, Stroke, Theme, Ui, vec2,
};
use egui_extras::{Column, TableBuilder};
use egui_phosphor::regular as icon;

use super::{
    App, Job, Status,
    dialogs::Settings,
    short_path,
    style::{self, Palette},
};

pub(super) fn show(app: &mut App, ui: &mut Ui, p: Palette) {
    // The built-in panel dividers use the parent UI's noninteractive stroke.
    let previous_separator = std::mem::replace(
        &mut ui.visuals_mut().widgets.noninteractive.bg_stroke,
        Stroke::new(1.0, p.line),
    );

    Panel::top("workspace-header")
        .resizable(false)
        .show_separator_line(true)
        .frame(
            Frame::new()
                .fill(p.surface)
                .inner_margin(Margin::symmetric(14, 4)),
        )
        .show(ui, |ui| header(app, ui));

    if !app.jobs.is_empty() {
        Panel::bottom("workspace-status")
            .resizable(false)
            .show_separator_line(true)
            .frame(
                Frame::new()
                    .fill(p.surface)
                    .inner_margin(Margin::symmetric(22, 7)),
            )
            .show(ui, |ui| footer(app, ui, p));
    }

    ui.visuals_mut().widgets.noninteractive.bg_stroke = previous_separator;

    let dropping = ui.ctx().input(|input| {
        input
            .raw
            .hovered_files
            .iter()
            .any(|file| file.path.is_some())
    });
    CentralPanel::default()
        .frame(Frame::new().fill(p.canvas).inner_margin(14))
        .show(ui, |ui| {
            if let Some(notice) = app.notice.as_deref()
                && notice_banner(ui, p, notice)
            {
                app.notice = None;
            }
            if app.jobs.is_empty() {
                empty_area(app, ui, p, dropping);
            } else {
                queue_toolbar(app, ui, p);
                if app.worker.is_some() {
                    active_progress(app, ui, p);
                }
                work_area(app, ui, p);
            }
        });

    if dropping && !app.jobs.is_empty() {
        let ctx = ui.ctx();
        let rect = ctx.content_rect().shrink(5.0);
        ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("archive-drop-outline"),
        ))
        .rect_stroke(
            rect,
            8,
            Stroke::new(2.0, p.accent),
            egui::StrokeKind::Inside,
        );
    }
}

fn header(app: &mut App, ui: &mut Ui) {
    style::row(ui, 34.0, Layout::right_to_left(Align::Center), |ui| {
        let (symbol, tooltip, next_theme) = match ui.ctx().theme() {
            Theme::Dark => (icon::SUN, "切换到浅色主题", Theme::Light),
            Theme::Light => (icon::MOON, "切换到深色主题", Theme::Dark),
        };
        if style::icon_button(ui, symbol, tooltip).clicked() {
            ui.ctx().set_theme(next_theme);
        }
        if cfg!(windows) {
            if ui.add(style::button(icon::GEAR, "右键菜单")).clicked() {
                app.settings = Some(Settings::menu());
            }
        } else if ui.add(style::button(icon::FILE, "第三方许可")).clicked() {
            app.settings = Some(Settings::Licenses);
        }
        if ui.add(style::button(icon::KEY, "密码簿")).clicked() {
            app.settings = Some(Settings::book());
        }
    });
}

fn footer(app: &App, ui: &mut Ui, p: Palette) {
    ui.horizontal(|ui| {
        let summary = if app.worker.is_some() {
            if app.waiting.is_empty() {
                "正在处理当前批次".to_owned()
            } else {
                format!("另有 {} 个文件等待解压", app.waiting.len())
            }
        } else if let Some(summary) = &app.completed {
            format!(
                "{} · 共 {} 个文件 · 成功 {} · 跳过 {} · 失败 {}",
                if summary.canceled {
                    "已停止"
                } else {
                    "处理完毕"
                },
                summary.total,
                summary.success,
                summary.skipped,
                summary.failed
            )
        } else {
            format!("共 {} 个文件", app.jobs.len())
        };
        ui.label(RichText::new(summary).size(12.0).color(p.muted));
    });
}

fn notice_banner(ui: &mut Ui, p: Palette, notice: &str) -> bool {
    Frame::new()
        .fill(p.warning_soft)
        .corner_radius(9)
        .inner_margin(9)
        .show(ui, |ui| {
            style::row(ui, 34.0, Layout::left_to_right(Align::Min), |ui| {
                ui.label(
                    RichText::new(icon::WARNING_CIRCLE)
                        .size(18.0)
                        .color(p.warning),
                );
                ui.vertical(|ui| {
                    ui.set_width((ui.available_width() - 44.0).max(0.0));
                    ui.label(RichText::new(notice).color(p.warning));
                });
                style::icon_button(ui, icon::X, "关闭提示").clicked()
            })
        })
        .inner
}

fn empty_area(app: &mut App, ui: &mut Ui, p: Palette, dropping: bool) {
    let card = p
        .card(18)
        .fill(if dropping { p.accent_soft } else { p.surface })
        .stroke(Stroke::new(
            if dropping { 2.0 } else { 1.0 },
            if dropping { p.accent } else { p.line },
        ));
    let content_size = (ui.available_size() - card.total_margin().sum()).max(egui::Vec2::ZERO);
    card.show(ui, |ui| {
        ui.set_min_size(content_size);
        ui.vertical_centered(|ui| {
            ui.add_space(((content_size.y - 200.0) / 2.0).max(0.0));
            style::glyph_tile(ui, icon::UPLOAD_SIMPLE, p, 64.0);
            ui.add_space(8.0);
            ui.label(
                RichText::new(if dropping {
                    "松开鼠标开始解压"
                } else {
                    "拖入压缩包开始解压"
                })
                .size(26.0)
                .strong()
                .color(p.text),
            );
            ui.label(
                RichText::new("支持 ZIP、RAR、7z、7z 分卷和带 ZIP 的 MP4 文件").color(p.muted),
            );
            ui.add_space(8.0);
            if ui
                .add(style::primary(icon::FOLDER_OPEN, "选择文件", p))
                .clicked()
            {
                app.pick_requested = true;
            }
        });
    });
}

fn queue_toolbar(app: &mut App, ui: &mut Ui, p: Palette) {
    style::row(ui, 38.0, Layout::left_to_right(Align::Center), |ui| {
        ui.label(RichText::new("解压队列").size(22.0).strong());
        ui.label(
            RichText::new(format!("{} 个文件", app.jobs.len() + app.waiting.len())).color(p.muted),
        );
        style::row(ui, 38.0, Layout::right_to_left(Align::Center), |ui| {
            if ui
                .add_enabled(
                    !app.closing && !app.stopping(),
                    style::primary(icon::PLUS, "添加文件", p),
                )
                .clicked()
            {
                app.pick_requested = true;
            }
            if app.worker.is_some() {
                let stopping = app.stopping();
                let label = if stopping {
                    "正在停止"
                } else {
                    "停止解压"
                };
                if ui
                    .add_enabled(!stopping, style::button(icon::STOP_CIRCLE, label))
                    .clicked()
                {
                    app.stop();
                }
            }
        });
    });
    ui.add_space(3.0);
}

fn active_progress(app: &App, ui: &mut Ui, p: Palette) {
    let job = app.current.and_then(|index| app.jobs.get(index));
    let title = job.map_or_else(|| "正在准备文件".into(), |job| short_path(&job.path));
    let description = if app.stopping() {
        "正在停止并清理暂存文件".into()
    } else if let Some(job) = job {
        let stage = if job.stage.is_empty() {
            "正在识别压缩包"
        } else {
            &job.stage
        };
        job.active_layer
            .and_then(|id| job.layers.get(&id))
            .map_or_else(
                || stage.to_owned(),
                |layer| format!("第 {} 层 · {} · {}", layer.depth, layer.name, stage),
            )
    } else {
        "等待处理文件".into()
    };
    let percent = job.and_then(|job| job.percent).filter(|_| !app.stopping());
    p.card(10).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        style::row(ui, 22.0, Layout::left_to_right(Align::Center), |ui| {
            if percent.is_none() {
                ui.add(egui::Spinner::new().size(15.0).color(p.accent));
            } else {
                ui.label(RichText::new(icon::FILE_ZIP).size(18.0).color(p.accent));
            }
            let reserve = if percent.is_some() { 48.0 } else { 0.0 };
            ui.add_sized(
                [(ui.available_width() - reserve).max(0.0), 22.0],
                Label::new(RichText::new(&title).strong()).truncate(),
            )
            .on_hover_text(title.as_str());
            if let Some(percent) = percent {
                style::row(ui, 22.0, Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{percent}%"))
                            .strong()
                            .color(p.accent),
                    );
                });
            }
        });
        ui.add(Label::new(RichText::new(&description).size(12.0).color(p.muted)).truncate())
            .on_hover_text(description.as_str());
        if let Some(percent) = percent {
            ui.add(
                egui::ProgressBar::new(f32::from(percent) / 100.0)
                    .fill(p.accent_fill)
                    .desired_height(5.0),
            );
        }
    });
    ui.add_space(4.0);
}

fn work_area(app: &mut App, ui: &mut Ui, p: Palette) {
    let size = ui.available_size();
    let gap = 13.0;
    let height = size.y.max(0.0);
    if height == 0.0 {
        return;
    }
    let width = (size.x - gap).max(0.0);
    let details_width = (width * 0.41).max(292.0).min((width - 240.0).max(0.0));
    let queue_width = width - details_width;
    ui.spacing_mut().item_spacing.x = gap;
    style::row(ui, height, Layout::left_to_right(Align::Min), |ui| {
        ui.allocate_ui_with_layout(
            vec2(queue_width, height),
            Layout::top_down(Align::Min),
            |ui| {
                ui.set_width(queue_width);
                if let Some(index) = queue_table(app, ui, p, height) {
                    app.selected = Some(index);
                    app.follow = app.current == Some(index);
                }
            },
        );
        ui.allocate_ui_with_layout(
            vec2(details_width, height),
            Layout::top_down(Align::Min),
            |ui| {
                ui.set_width(details_width);
                if let Some(path) = details_panel(app, ui, p, height) {
                    app.open_output(&path);
                }
            },
        );
    });
}

fn queue_table(app: &App, ui: &mut Ui, p: Palette, height: f32) -> Option<usize> {
    let mut selected = None;
    let rows = app.jobs.len() + app.waiting.len();
    let card = p.card(3);
    let height = (height - card.total_margin().sum().y).max(0.0);
    card.show(ui, |ui| {
        let header_height = 35.0;
        let table_height = (height - header_height - ui.spacing().item_spacing.y).max(0.0);
        ui.set_min_size(vec2(ui.available_width(), height));
        // Table stripes and selections extend into the spacing around cells.
        ui.shrink_clip_rect(ui.max_rect());
        TableBuilder::new(ui)
            .id_salt("queue-table")
            .cell_layout(Layout::left_to_right(Align::Center))
            .striped(true)
            .sense(Sense::click())
            .min_scrolled_height(0.0)
            .max_scroll_height(table_height)
            .column(Column::remainder().at_least(180.0).clip(true))
            .column(Column::exact(100.0).clip(true))
            .column(Column::exact(62.0).clip(true))
            .header(header_height, |mut header| {
                for label in ["文件", "状态", "进度"] {
                    header.col(|ui| {
                        ui.add_space(9.0);
                        ui.label(RichText::new(label).color(p.muted).size(12.0).strong());
                    });
                }
            })
            .body(|body| {
                body.rows(46.0, rows, |mut row| {
                    let index = row.index();
                    let job = app.jobs.get(index);
                    let path = match job {
                        Some(job) => &job.path,
                        None => &app.waiting[index - app.jobs.len()],
                    };
                    let status = job.map_or(Status::Pending, |job| job.status);
                    let is_selected = app.selected == Some(index);
                    let mut file_clicked = false;
                    row.set_selected(is_selected);
                    row.col(|ui| {
                        ui.add_space(8.0);
                        ui.label(RichText::new(icon::FILE_ZIP).size(18.0).color(p.accent));
                        file_clicked = ui
                            .add_enabled(
                                job.is_some(),
                                Button::selectable(is_selected, short_path(path))
                                    .frame(false)
                                    .truncate(),
                            )
                            .on_hover_text(path.display().to_string())
                            .clicked();
                    });
                    row.col(|ui| {
                        ui.add_space(9.0);
                        let (symbol, label, color) = status_style(p, status);
                        ui.label(
                            RichText::new(format!("{symbol}  {label}"))
                                .size(11.5)
                                .color(color),
                        );
                    });
                    row.col(|ui| {
                        ui.add_space(9.0);
                        let progress = match status {
                            Status::Success => Some(100),
                            Status::Running => job.and_then(|job| job.percent),
                            _ => None,
                        };
                        ui.label(
                            RichText::new(
                                progress
                                    .map_or_else(|| "—".into(), |percent| format!("{percent}%")),
                            )
                            .color(p.muted)
                            .size(12.0),
                        );
                    });
                    if job.is_some() && (file_clicked || row.response().clicked()) {
                        selected = Some(index);
                    }
                });
            });
    });
    selected
}

fn details_panel(app: &App, ui: &mut Ui, p: Palette, height: f32) -> Option<std::path::PathBuf> {
    let card = p.card(14);
    let height = (height - card.total_margin().sum().y).max(0.0);
    card.show(ui, |ui| {
        ui.set_min_size(vec2(ui.available_width(), height));
        let mut output = None;
        let Some(index) = app.selected else {
            return output;
        };
        let Some(job) = app.jobs.get(index) else {
            return output;
        };

        ScrollArea::vertical()
            .id_salt(("archive-details", index))
            .min_scrolled_height(0.0)
            .max_height(height)
            .show(ui, |ui| {
                ui.add(
                    Label::new(RichText::new(short_path(&job.path)).size(18.0).strong()).truncate(),
                );
                ui.add(
                    Label::new(
                        RichText::new(job.path.display().to_string())
                            .size(12.0)
                            .color(p.muted),
                    )
                    .truncate(),
                )
                .on_hover_text(job.path.display().to_string());
                if let Some(result) = &job.result {
                    if let Some(path) = &result.output {
                        ui.label(
                            RichText::new(format!(
                                "{} 个文件 · {} 个压缩包 · 最深 {} 层",
                                result.files, result.archives, result.depth
                            ))
                            .size(12.0)
                            .color(p.muted),
                        );
                        if ui
                            .add(style::button(icon::FOLDER_OPEN, "打开解压目录"))
                            .on_hover_text(path.display().to_string())
                            .clicked()
                        {
                            output = Some(path.clone());
                        }
                    }
                    if !result.detail.is_empty() {
                        ui.label(RichText::new(&result.detail).color(
                            if matches!(job.status, Status::Failed) {
                                p.danger
                            } else {
                                p.muted
                            },
                        ));
                    }
                }
                ui.add_space(3.0);
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(icon::TREE_STRUCTURE)
                            .size(17.0)
                            .color(p.accent),
                    );
                    ui.label(RichText::new("压缩包结构").strong());
                });
                if let Some(roots) = job.children.get(&None) {
                    for &root in roots {
                        archive_node(ui, p, job, root);
                    }
                } else {
                    ui.label(
                        RichText::new(if matches!(job.status, Status::Pending | Status::Running) {
                            "等待识别压缩包"
                        } else {
                            "无压缩包结构"
                        })
                        .color(p.muted),
                    );
                }
            });
        output
    })
    .inner
}

fn archive_node(ui: &mut Ui, p: Palette, job: &Job, id: usize) {
    let Some(node) = job.layers.get(&id) else {
        return;
    };
    let (symbol, _, color) = status_style(p, node.status);
    let branch = job.children.get(&Some(id));
    let mut title = format!("{symbol}  {} · {}", node.name, node.format);
    if !node.volumes.is_empty() {
        title.push_str(&format!(" · {} 卷", node.volumes.len()));
    }
    let label = RichText::new(title.as_str()).color(color);
    if branch.is_some() || !node.volumes.is_empty() {
        egui::CollapsingHeader::new(label)
            .id_salt(id)
            .default_open(node.depth <= 2)
            .show(ui, |ui| {
                for volume in &node.volumes {
                    ui.add(
                        Label::new(
                            RichText::new(format!("{}  {volume}", icon::FILE)).color(p.muted),
                        )
                        .truncate(),
                    )
                    .on_hover_text(volume.as_str());
                }
                if let Some(children) = branch {
                    for &child in children {
                        archive_node(ui, p, job, child);
                    }
                }
                if let Some(files) = node.files.filter(|files| *files > 0) {
                    ui.label(
                        RichText::new(format!("{files} 个普通文件"))
                            .size(12.0)
                            .color(p.muted),
                    );
                }
            });
    } else {
        ui.add(Label::new(label).truncate())
            .on_hover_text(title.as_str());
    }
}

fn status_style(p: Palette, status: Status) -> (&'static str, &'static str, Color32) {
    match status {
        Status::Pending => (icon::CLOCK, "等待", p.muted),
        Status::Running => (icon::SPINNER, "解压中", p.accent),
        Status::Success => (icon::CHECK_CIRCLE, "成功", p.positive),
        Status::Skipped => (icon::SKIP_FORWARD, "已跳过", p.warning),
        Status::Failed => (icon::WARNING_CIRCLE, "失败", p.danger),
        Status::Canceled => (icon::STOP_CIRCLE, "已停止", p.muted),
    }
}
