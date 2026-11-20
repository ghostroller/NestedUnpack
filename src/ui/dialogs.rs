use eframe::egui::{
    self, Align, Button, Context, Id, Label, Layout, Modal, RichText, ScrollArea, TextEdit, Ui,
    vec2,
};
use egui_phosphor::regular as icon;
use zeroize::Zeroize as _;

use crate::{THIRD_PARTY_NOTICES, platform, session::PasswordReply};

use super::{
    App, PasswordPrompt, short_path,
    style::{self, Palette},
};

pub(super) enum Settings {
    Book(Book),
    Menu(MenuSettings),
    Licenses,
}

impl Settings {
    pub fn book() -> Self {
        let mut book = Book::default();
        book.refresh();
        Self::Book(book)
    }

    pub fn menu() -> Self {
        Self::Menu(MenuSettings {
            registered: platform::menu_registered().map_err(|e| e.to_string()),
        })
    }
}

#[derive(Default)]
pub(super) struct Book {
    values: Vec<String>,
    selected: Option<usize>,
    new_password: String,
    confirming: bool,
    error: Option<String>,
}

impl Book {
    fn refresh(&mut self) {
        match platform::read_passwords() {
            Ok(values) => {
                self.values.zeroize();
                self.values = values;
                self.selected = self.selected.filter(|&index| index < self.values.len());
                self.error = None;
            }
            Err(error) => self.error = Some(format!("无法读取密码簿。{error}")),
        }
        self.confirming = false;
    }

    fn add(&mut self) {
        match platform::remember_password(&self.new_password) {
            Ok(_) => {
                self.refresh();
                self.selected = self
                    .values
                    .iter()
                    .position(|password| password == &self.new_password);
                self.new_password.zeroize();
            }
            Err(error) => self.error = Some(format!("无法保存密码。{error}")),
        }
    }

    fn remove(&mut self) {
        let Some(password) = self.selected.and_then(|index| self.values.get(index)) else {
            return;
        };
        match platform::forget_password(password) {
            Ok(_) => {
                self.selected = None;
                self.refresh();
            }
            Err(error) => {
                self.confirming = false;
                self.error = Some(format!("无法删除密码。{error}"));
            }
        }
    }
}

impl Drop for Book {
    fn drop(&mut self) {
        self.values.zeroize();
        self.new_password.zeroize();
    }
}

impl Drop for PasswordPrompt {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}

pub(super) struct MenuSettings {
    registered: Result<bool, String>,
}

pub(super) fn show(app: &mut App, ctx: &Context, p: Palette) {
    if let Some(prompt) = app.password.as_mut() {
        match show_password(prompt, ctx, p) {
            PasswordAction::Wait => {}
            PasswordAction::Skip => app.resolve_password(None),
            PasswordAction::Submit => {
                let reply = PasswordReply {
                    value: std::mem::take(&mut prompt.value),
                    remember: prompt.remember,
                };
                app.resolve_password(Some(reply));
            }
        }
        return;
    }

    let action = match app.settings.as_mut() {
        Some(Settings::Book(book)) => show_book(book, ctx, p),
        Some(Settings::Menu(menu)) => show_menu(menu, ctx, p),
        Some(Settings::Licenses) => show_licenses(ctx, p),
        None => SettingsAction::Wait,
    };
    match action {
        SettingsAction::Wait => {}
        SettingsAction::Close => app.settings = None,
        SettingsAction::Licenses => app.settings = Some(Settings::Licenses),
    }
}

#[derive(Clone, Copy)]
enum PasswordAction {
    Wait,
    Submit,
    Skip,
}

fn show_password(prompt: &mut PasswordPrompt, ctx: &Context, p: Palette) -> PasswordAction {
    Modal::new(Id::new("archive-password"))
        .frame(p.card(20))
        .show(ctx, |ui| {
            ui.set_width(410.0);
            title(ui, p, icon::KEY, "压缩包密码");
            let context = format!(
                "{} · 第 {} 层 · {}",
                prompt.request.format,
                prompt.request.depth,
                short_path(&prompt.request.archive)
            );
            ui.add(Label::new(RichText::new(&context).color(p.muted)).truncate())
                .on_hover_text(prompt.request.archive.display().to_string());
            if prompt.request.source != prompt.request.archive {
                ui.add(
                    Label::new(
                        RichText::new(short_path(&prompt.request.source))
                            .size(12.0)
                            .color(p.muted),
                    )
                    .truncate(),
                )
                .on_hover_text(prompt.request.source.display().to_string());
            }
            if prompt.request.retry {
                ui.label(RichText::new("密码验证未通过，请重试。").color(p.danger));
            }
            ui.add_space(3.0);

            let mut enter = false;
            ui.horizontal(|ui| {
                let input = ui.add_sized(
                    [ui.available_width() - 44.0, 34.0],
                    TextEdit::singleline(&mut prompt.value)
                        .password(!prompt.reveal)
                        .hint_text("输入密码"),
                );
                if std::mem::take(&mut prompt.autofocus) {
                    input.request_focus();
                }
                enter = input.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                let symbol = if prompt.reveal {
                    icon::EYE_SLASH
                } else {
                    icon::EYE
                };
                let label = if prompt.reveal {
                    "隐藏密码"
                } else {
                    "显示密码"
                };
                if style::icon_button(ui, symbol, label).clicked() {
                    prompt.reveal = !prompt.reveal;
                }
            });
            ui.checkbox(&mut prompt.remember, "验证成功后保存到密码簿");
            ui.add_space(6.0);
            style::row(ui, 38.0, Layout::right_to_left(Align::Center), |ui| {
                let submit = ui
                    .add_enabled(
                        !prompt.value.is_empty(),
                        style::primary(icon::CHECK, "继续解压", p),
                    )
                    .clicked();
                let skip = ui.add(style::button(icon::X, "跳过此文件")).clicked()
                    || ui.input_mut(|input| {
                        input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                    });
                if !prompt.value.is_empty() && (submit || enter) {
                    PasswordAction::Submit
                } else if skip {
                    PasswordAction::Skip
                } else {
                    PasswordAction::Wait
                }
            })
        })
        .inner
}

#[derive(Clone, Copy)]
enum SettingsAction {
    Wait,
    Close,
    Licenses,
}

fn show_book(book: &mut Book, ctx: &Context, p: Palette) -> SettingsAction {
    let modal = Modal::new(Id::new("password-book"))
        .frame(p.card(16))
        .show(ctx, |ui| {
            ui.set_width(410.0);
            title(ui, p, icon::KEY, "密码簿");
            ui.label(RichText::new(format!("已保存 {} 个密码", book.values.len())).color(p.muted));
            p.card(7).fill(p.softened).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                if book.values.is_empty() {
                    ui.label(RichText::new("密码簿为空").color(p.muted));
                } else {
                    ScrollArea::vertical()
                        .auto_shrink([false, true])
                        .max_height(140.0)
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 3.0;
                            ui.visuals_mut().widgets.noninteractive.bg_stroke =
                                egui::Stroke::new(1.0, p.line);
                            for (index, password) in book.values.iter().enumerate() {
                                if index > 0 {
                                    ui.add(egui::Separator::default().spacing(1.0));
                                }
                                let row = Button::selectable(
                                    book.selected == Some(index),
                                    (password.as_str(), egui::Atom::grow()),
                                )
                                .min_size(vec2(ui.available_width(), 30.0))
                                .wrap();
                                if ui.add(row).clicked() {
                                    book.selected = Some(index);
                                    book.confirming = false;
                                }
                            }
                        });
                }
            });

            if book.confirming {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("删除所选密码？").color(p.danger));
                    if ui.add(style::button(icon::TRASH, "确认删除")).clicked() {
                        book.remove();
                    }
                    if ui.button("保留").clicked() {
                        book.confirming = false;
                    }
                });
            }
            ui.horizontal(|ui| {
                let entry = ui.add_sized(
                    [ui.available_width() - 94.0, 34.0],
                    TextEdit::singleline(&mut book.new_password)
                        .hint_text("添加密码")
                        .vertical_align(Align::Center)
                        .margin(egui::Margin::symmetric(10, 4)),
                );
                let enter =
                    entry.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                let add = ui
                    .add_enabled(
                        !book.new_password.is_empty(),
                        style::primary(icon::PLUS, "添加", p),
                    )
                    .clicked();
                if !book.new_password.is_empty() && (add || enter) {
                    book.add();
                    entry.request_focus();
                }
            });
            if let Some(error) = &book.error {
                ui.label(RichText::new(error).color(p.danger));
            }
            style::row(ui, 38.0, Layout::right_to_left(Align::Center), |ui| {
                let close = ui.add(style::button(icon::X, "关闭")).clicked();
                if book.selected.is_some()
                    && !book.confirming
                    && ui.add(style::button(icon::TRASH, "删除所选")).clicked()
                {
                    book.confirming = true;
                }
                close
            })
        });
    if modal.inner || modal.should_close() {
        SettingsAction::Close
    } else {
        SettingsAction::Wait
    }
}

fn show_menu(menu: &mut MenuSettings, ctx: &Context, p: Palette) -> SettingsAction {
    let modal = Modal::new(Id::new("explorer-context-menu"))
        .frame(p.card(20))
        .show(ctx, |ui| {
            ui.set_width(427.0);
            title(ui, p, icon::GEAR, "资源管理器右键菜单");
            ui.label(
                RichText::new("为当前 Windows 用户添加「解开嵌套压缩包」命令。").color(p.muted),
            );
            let (status, color) = match &menu.registered {
                Ok(true) => ("已注册到当前位置".into(), p.positive),
                Ok(false) => ("未注册到当前位置".into(), p.muted),
                Err(error) => (error.clone(), p.danger),
            };
            p.card(10).fill(p.softened).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(RichText::new(status).color(color));
            });
            ui.horizontal(|ui| {
                if ui
                    .add(style::primary(icon::PLUS, "注册到当前位置", p))
                    .clicked()
                {
                    menu.registered = platform::register_menu()
                        .and_then(|_| platform::menu_registered())
                        .map_err(|e| format!("无法注册右键菜单。{e}"));
                }
                if ui.add(style::button(icon::TRASH, "移除菜单")).clicked() {
                    menu.registered = platform::remove_menu()
                        .and_then(|_| platform::menu_registered())
                        .map_err(|e| format!("无法移除右键菜单。{e}"));
                }
            });
            ui.separator();
            style::row(ui, 38.0, Layout::left_to_right(Align::Center), |ui| {
                let licenses = ui.add(style::button(icon::FILE, "第三方许可")).clicked();
                let close = style::row(ui, 38.0, Layout::right_to_left(Align::Center), |ui| {
                    ui.add(style::button(icon::X, "关闭")).clicked()
                });
                if licenses {
                    SettingsAction::Licenses
                } else if close {
                    SettingsAction::Close
                } else {
                    SettingsAction::Wait
                }
            })
        });
    if modal.should_close() {
        SettingsAction::Close
    } else {
        modal.inner
    }
}

fn show_licenses(ctx: &Context, p: Palette) -> SettingsAction {
    let modal = Modal::new(Id::new("third-party-notices"))
        .frame(p.card(20))
        .show(ctx, |ui| {
            ui.set_width(ui.ctx().content_rect().width().min(750.0) - 70.0);
            title(ui, p, icon::FILE, "第三方许可");
            ScrollArea::vertical()
                .min_scrolled_height(0.0)
                .max_height((ui.ctx().content_rect().height() - 155.0).max(0.0))
                .show(ui, |ui| {
                    ui.add(
                        Label::new(RichText::new(THIRD_PARTY_NOTICES).monospace().size(12.0))
                            .selectable(true),
                    );
                });
            style::row(ui, 38.0, Layout::right_to_left(Align::Center), |ui| {
                ui.add(style::button(icon::X, "关闭")).clicked()
            })
        });
    if modal.inner || modal.should_close() {
        SettingsAction::Close
    } else {
        SettingsAction::Wait
    }
}

fn title(ui: &mut Ui, p: Palette, symbol: &str, label: &str) {
    style::row(ui, 38.0, Layout::left_to_right(Align::Center), |ui| {
        style::glyph_tile(ui, symbol, p, 36.0);
        ui.add_space(2.0);
        ui.label(RichText::new(label).size(20.0).strong());
    });
    ui.add_space(3.0);
}
