use eframe::egui::{
    self, Align, Button, Context, Id, Label, Layout, Modal, RichText, ScrollArea, TextEdit, Ui,
    vec2,
};
use egui_phosphor::regular as icon;
use zeroize::Zeroize as _;

use crate::{
    LICENSES,
    i18n::{self, AppKey, Message, UiKey, tr},
    platform,
    session::PasswordReply,
};

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
            registered: platform::menu_registered().map_err(|error| {
                Message::from(UiKey::CheckMenuFailed).arg(Message::from_error(error))
            }),
        })
    }
}

#[derive(Default)]
pub(super) struct Book {
    values: Vec<String>,
    selected: Option<usize>,
    new_password: String,
    confirming: bool,
    error: Option<Message>,
    notice: Option<Message>,
}

impl Book {
    fn refresh(&mut self) {
        self.notice = None;
        match platform::read_passwords() {
            Ok(values) => {
                self.values.zeroize();
                self.values = values;
                self.selected = self.selected.filter(|&index| index < self.values.len());
                self.error = None;
            }
            Err(error) => {
                self.error = Some(
                    Message::from(UiKey::ReadPasswordBookFailed).arg(Message::from_error(error)),
                );
            }
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
            Err(error) => {
                self.error =
                    Some(Message::from(UiKey::SavePasswordFailed).arg(Message::from_error(error)));
            }
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
                self.error = Some(
                    Message::from(UiKey::DeletePasswordFailed).arg(Message::from_error(error)),
                );
            }
        }
    }

    fn transfer(&mut self, frame: &eframe::Frame, transfer: PasswordBookTransfer) {
        let title = match transfer {
            PasswordBookTransfer::Import => UiKey::ImportPasswordsTitle,
            PasswordBookTransfer::Export => UiKey::ExportPasswordsTitle,
        };
        let dialog = rfd::FileDialog::new()
            .set_parent(frame)
            .set_title(tr(title))
            .add_filter(tr(UiKey::PasswordBackupJson), &["json"]);
        let result = match transfer {
            PasswordBookTransfer::Import => {
                let Some(path) = dialog.add_filter(tr(AppKey::AllFiles), &["*"]).pick_file() else {
                    return;
                };
                platform::import_passwords(&path)
                    .map(|count| {
                        self.refresh();
                        Message::from(UiKey::PasswordsImported).arg(count)
                    })
                    .map_err(|error| {
                        Message::from(UiKey::PasswordImportFailed).arg(Message::from_error(error))
                    })
            }
            PasswordBookTransfer::Export => {
                let Some(path) = dialog
                    .set_file_name("NestedUnpack-passwords.json")
                    .save_file()
                else {
                    return;
                };
                self.error = None;
                platform::export_passwords(&path)
                    .map(|()| Message::from(UiKey::PasswordsExported))
                    .map_err(|error| {
                        Message::from(UiKey::PasswordExportFailed).arg(Message::from_error(error))
                    })
            }
        };
        self.confirming = false;
        match result {
            Ok(notice) => self.notice = Some(notice),
            Err(error) => {
                self.notice = None;
                self.error = Some(error);
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
    registered: Result<bool, Message>,
}

pub(super) fn show(app: &mut App, ctx: &Context, frame: &eframe::Frame, p: Palette) {
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
        SettingsAction::TransferPasswords(transfer) => {
            if let Some(Settings::Book(book)) = app.settings.as_mut() {
                book.transfer(frame, transfer);
                ctx.request_repaint();
            }
        }
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
            title(ui, p, icon::KEY, tr(UiKey::ArchivePassword));
            let context = i18n::format(
                UiKey::PasswordContext,
                &[
                    &tr(prompt.request.format),
                    &prompt.request.depth,
                    &short_path(&prompt.request.archive),
                ],
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
                ui.label(RichText::new(tr(UiKey::PasswordRetry)).color(p.danger));
            }
            ui.add_space(3.0);

            let mut enter = false;
            ui.horizontal(|ui| {
                let input = ui.add_sized(
                    [ui.available_width() - 44.0, 34.0],
                    TextEdit::singleline(&mut prompt.value)
                        .password(!prompt.reveal)
                        .hint_text(tr(UiKey::EnterPassword))
                        .vertical_align(Align::Center)
                        .margin(egui::Margin::symmetric(10, 4)),
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
                let label = tr(if prompt.reveal {
                    UiKey::HidePassword
                } else {
                    UiKey::ShowPassword
                });
                if style::icon_button(ui, symbol, label).clicked() {
                    prompt.reveal = !prompt.reveal;
                }
            });
            ui.checkbox(&mut prompt.remember, tr(UiKey::RememberPassword));
            ui.add_space(6.0);
            style::row(ui, 38.0, Layout::right_to_left(Align::Center), |ui| {
                let submit = ui
                    .add_enabled(
                        !prompt.value.is_empty(),
                        style::primary(icon::CHECK, tr(UiKey::ContinueExtraction), p),
                    )
                    .clicked();
                let skip = ui
                    .add(style::button(icon::X, tr(UiKey::SkipFile)))
                    .clicked()
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
    TransferPasswords(PasswordBookTransfer),
}

#[derive(Clone, Copy)]
enum PasswordBookTransfer {
    Import,
    Export,
}

fn show_book(book: &mut Book, ctx: &Context, p: Palette) -> SettingsAction {
    let modal = Modal::new(Id::new("password-book"))
        .frame(p.card(16))
        .show(ctx, |ui| {
            ui.set_width(410.0);
            title(ui, p, icon::KEY, tr(UiKey::PasswordBook));
            ui.label(RichText::new(i18n::saved_passwords(book.values.len())).color(p.muted));
            p.card(7).fill(p.softened).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                if book.values.is_empty() {
                    ui.label(RichText::new(tr(UiKey::EmptyPasswordBook)).color(p.muted));
                } else {
                    ScrollArea::vertical()
                        .auto_shrink([false, true])
                        .max_height(140.0)
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 3.0;
                            let widgets = &mut ui.visuals_mut().widgets;
                            widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, p.line);
                            widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
                            for (index, password) in book.values.iter().enumerate() {
                                if index > 0 {
                                    ui.add(egui::Separator::default().spacing(1.0));
                                }
                                let selected = book.selected == Some(index);
                                // Keep the stroke width in the layout even when it is invisible.
                                ui.visuals_mut().widgets.inactive.bg_stroke.color = if selected {
                                    p.line
                                } else {
                                    egui::Color32::TRANSPARENT
                                };
                                let row = Button::selectable(
                                    selected,
                                    (password.as_str(), egui::Atom::grow()),
                                )
                                .frame_when_inactive(true)
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
                    ui.label(RichText::new(tr(UiKey::DeletePasswordConfirmation)).color(p.danger));
                    if ui
                        .add(style::button(icon::TRASH, tr(UiKey::ConfirmDelete)))
                        .clicked()
                    {
                        book.remove();
                    }
                    if ui.button(tr(UiKey::KeepPassword)).clicked() {
                        book.confirming = false;
                    }
                });
            }
            ui.horizontal(|ui| {
                let entry = ui.add_sized(
                    [ui.available_width() - 94.0, 34.0],
                    TextEdit::singleline(&mut book.new_password)
                        .hint_text(tr(UiKey::AddPassword))
                        .vertical_align(Align::Center)
                        .margin(egui::Margin::symmetric(10, 4)),
                );
                let enter =
                    entry.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                let add = ui
                    .add_enabled(
                        !book.new_password.is_empty(),
                        style::primary(icon::PLUS, tr(UiKey::Add), p),
                    )
                    .clicked();
                if !book.new_password.is_empty() && (add || enter) {
                    book.add();
                    entry.request_focus();
                }
            });
            if let Some(error) = &book.error {
                ui.label(RichText::new(error.render(i18n::current())).color(p.danger));
            } else if let Some(notice) = &book.notice {
                ui.label(RichText::new(notice.render(i18n::current())).color(p.positive));
            }
            ui.label(
                RichText::new(tr(UiKey::PasswordBackupPlaintext))
                    .size(12.0)
                    .color(p.muted),
            );
            style::row(ui, 38.0, Layout::left_to_right(Align::Center), |ui| {
                let import = ui
                    .add(style::button(
                        icon::DOWNLOAD_SIMPLE,
                        tr(UiKey::ImportPasswords),
                    ))
                    .clicked();
                let export = ui
                    .add_enabled(
                        !book.values.is_empty(),
                        style::button(icon::UPLOAD_SIMPLE, tr(UiKey::ExportPasswords)),
                    )
                    .clicked();
                let close = style::row(ui, 38.0, Layout::right_to_left(Align::Center), |ui| {
                    let close = ui.add(style::button(icon::X, tr(UiKey::Close))).clicked();
                    if book.selected.is_some()
                        && !book.confirming
                        && ui
                            .add(style::button(icon::TRASH, tr(UiKey::DeleteSelected)))
                            .clicked()
                    {
                        book.confirming = true;
                    }
                    close
                });
                if close {
                    SettingsAction::Close
                } else if import {
                    SettingsAction::TransferPasswords(PasswordBookTransfer::Import)
                } else if export {
                    SettingsAction::TransferPasswords(PasswordBookTransfer::Export)
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

fn show_menu(menu: &mut MenuSettings, ctx: &Context, p: Palette) -> SettingsAction {
    let modal = Modal::new(Id::new("explorer-context-menu"))
        .frame(p.card(20))
        .show(ctx, |ui| {
            ui.set_width(427.0);
            title(ui, p, icon::GEAR, tr(UiKey::ExplorerContextMenu));
            ui.label(
                RichText::new(i18n::format(
                    UiKey::MenuExplanation,
                    &[&tr(AppKey::ExplorerMenuLabel)],
                ))
                .color(p.muted),
            );
            let (status, color) = match &menu.registered {
                Ok(true) => (tr(UiKey::MenuRegistered).into(), p.positive),
                Ok(false) => (tr(UiKey::MenuUnregistered).into(), p.muted),
                Err(error) => (error.render(i18n::current()), p.danger),
            };
            p.card(10).fill(p.softened).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(RichText::new(status).color(color));
            });
            ui.horizontal(|ui| {
                if ui
                    .add(style::primary(icon::PLUS, tr(UiKey::RegisterMenu), p))
                    .clicked()
                {
                    menu.registered = platform::register_menu()
                        .and_then(|_| platform::menu_registered())
                        .map_err(|error| {
                            Message::from(UiKey::RegisterMenuFailed).arg(Message::from_error(error))
                        });
                }
                if ui
                    .add(style::button(icon::TRASH, tr(UiKey::RemoveMenu)))
                    .clicked()
                {
                    menu.registered = platform::remove_menu()
                        .and_then(|_| platform::menu_registered())
                        .map_err(|error| {
                            Message::from(UiKey::RemoveMenuFailed).arg(Message::from_error(error))
                        });
                }
            });
            ui.separator();
            style::row(ui, 38.0, Layout::left_to_right(Align::Center), |ui| {
                let licenses = ui
                    .add(style::button(icon::FILE, tr(UiKey::Licenses)))
                    .clicked();
                let close = style::row(ui, 38.0, Layout::right_to_left(Align::Center), |ui| {
                    ui.add(style::button(icon::X, tr(UiKey::Close))).clicked()
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
    let modal = Modal::new(Id::new("licenses"))
        .frame(p.card(20))
        .show(ctx, |ui| {
            ui.set_width(ui.ctx().content_rect().width().min(750.0) - 70.0);
            title(ui, p, icon::FILE, tr(UiKey::Licenses));
            ScrollArea::vertical()
                .min_scrolled_height(0.0)
                .max_height((ui.ctx().content_rect().height() - 155.0).max(0.0))
                .show(ui, |ui| {
                    ui.add(
                        Label::new(RichText::new(LICENSES).monospace().size(12.0)).selectable(true),
                    );
                });
            style::row(ui, 38.0, Layout::right_to_left(Align::Center), |ui| {
                ui.add(style::button(icon::X, tr(UiKey::Close))).clicked()
            })
        });
    if modal.inner || modal.should_close() {
        SettingsAction::Close
    } else {
        SettingsAction::Wait
    }
}

fn title(ui: &mut Ui, p: Palette, symbol: &str, label: &str) {
    style::icon_heading(ui, symbol, p, label);
    ui.add_space(3.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, RawInput, Rect, Shape, Theme, pos2};

    #[test]
    fn password_book_retains_row_layout_and_dispatches_transfer_actions() {
        for theme in [Theme::Light, Theme::Dark] {
            let ctx = Context::default();
            style::configure(&ctx);
            ctx.set_theme(theme);
            let first = "first-example";
            let second = "long-example-".repeat(12);
            let mut book = Book::default();
            book.values = vec![first.into(), second.clone()];
            book.selected = Some(1);
            let mut time = 0.0;
            let mut draw = |events| {
                time += 0.1;
                let mut action = SettingsAction::Wait;
                let mut output = ctx.run_ui(
                    RawInput {
                        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 550.0))),
                        time: Some(time),
                        events,
                        ..RawInput::default()
                    },
                    |ui| {
                        action = show_book(&mut book, ui.ctx(), Palette::current(ui.ctx()));
                    },
                );
                output.textures_delta.clear();
                (output.shapes, action)
            };
            let geometry = |shapes: &[egui::epaint::ClippedShape]| {
                [
                    first,
                    &second,
                    tr(UiKey::AddPassword),
                    tr(UiKey::ImportPasswords),
                    tr(UiKey::ExportPasswords),
                ]
                .map(|label| {
                    shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            Shape::Text(text) if text.galley.text() == label => {
                                Some(text.visual_bounding_rect())
                            }
                            _ => None,
                        })
                        .unwrap_or_else(|| panic!("missing label {label:?}"))
                })
            };
            for _ in 0..3 {
                draw(Vec::new());
            }
            let idle = geometry(&draw(Vec::new()).0);
            for row in &idle[..2] {
                let pointer = row.center_top() + vec2(0.0, 5.0);
                let (hovered, _) = draw(vec![Event::PointerMoved(pointer)]);
                let p = Palette::current(&ctx);
                assert!(hovered.iter().any(|shape| {
                    matches!(&shape.shape, Shape::Rect(rect)
                        if rect.rect.contains(pointer) && rect.stroke.color == p.accent
                            && rect.fill == p.accent_soft)
                }));
                assert_eq!(idle, geometry(&hovered), "{theme:?}");
            }
            assert_eq!(
                idle,
                geometry(&draw(vec![Event::PointerGone]).0),
                "{theme:?}"
            );
            let click = |rect: Rect| {
                let pos = rect.center();
                let button = |pressed| Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                };
                vec![Event::PointerMoved(pos), button(true), button(false)]
            };
            assert!(matches!(
                draw(click(idle[3])).1,
                SettingsAction::TransferPasswords(PasswordBookTransfer::Import)
            ));
            assert!(matches!(
                draw(click(idle[4])).1,
                SettingsAction::TransferPasswords(PasswordBookTransfer::Export)
            ));
        }
    }
}
