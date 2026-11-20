use std::{collections::BTreeMap, ffi::OsString, path::Component};

use eframe::egui::{RichText, Stroke, Ui, collapsing_header::CollapsingState};
use egui_phosphor::regular as icon;

use crate::{
    i18n::{self, UiKey, tr},
    session::ArchiveMember,
};

use super::{
    Job,
    style::{self, Palette, status_style},
};

pub(super) fn show(ui: &mut Ui, p: Palette, job: &Job, roots: &[usize]) {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        ui.visuals_mut().widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.line);
        for &id in roots {
            archive(ui, p, job, id);
        }
    });
}

fn archive(ui: &mut Ui, p: Palette, job: &Job, id: usize) {
    let Some(node) = job.layers.get(&id) else {
        return;
    };
    let (symbol, _, color) = status_style(p, node.status);
    let children = job.children.get(&Some(id));
    let has_archive_children = children.is_some() || !node.volumes.is_empty();
    let files = job.file_trees.get(&id).zip(node.contents.as_deref());
    let mut title = format!("{} · {}", node.name, tr(node.format));
    if !node.volumes.is_empty() {
        title.push_str(" · ");
        title.push_str(&i18n::volume_count(node.volumes.len()));
    }
    let label = RichText::new(title.as_str()).color(color);
    let mut state = (has_archive_children || files.is_some()).then(|| {
        CollapsingState::load_with_default_open(
            ui.ctx(),
            ui.make_persistent_id(("archive", id, has_archive_children)),
            has_archive_children && node.depth <= 2,
        )
    });
    let response =
        style::tree_row(ui, p, symbol, color, label, state.as_mut()).on_hover_text(&title);
    if let Some(state) = &mut state {
        state.show_body_indented(&response, ui, |ui| {
            for volume in &node.volumes {
                style::tree_row(ui, p, icon::FILE, p.muted, RichText::new(volume), None)
                    .on_hover_text(volume);
            }
            if let Some(children) = children {
                for &child in children {
                    archive(ui, p, job, child);
                }
            }
            if let Some((tree, members)) = files {
                if has_archive_children {
                    let file_count = node.files.unwrap_or_default();
                    let label = if file_count == 0 {
                        tr(UiKey::Contents).into()
                    } else {
                        i18n::format(UiKey::ContentsCount, &[&i18n::file_count(file_count)])
                    };
                    let mut state = CollapsingState::load_with_default_open(
                        ui.ctx(),
                        ui.make_persistent_id("archive-files"),
                        false,
                    );
                    let response = style::tree_row(
                        ui,
                        p,
                        icon::FILES,
                        p.muted,
                        RichText::new(label).color(p.muted),
                        Some(&mut state),
                    );
                    state.show_body_indented(&response, ui, |ui| {
                        tree.get_or_init(|| FileTree::new(members)).show(ui, p, 0);
                    });
                } else {
                    tree.get_or_init(|| FileTree::new(members)).show(ui, p, 0);
                }
            }
        });
    }
}

#[derive(Default)]
struct Directory {
    folders: BTreeMap<OsString, usize>,
    files: Vec<OsString>,
}

pub(super) struct FileTree {
    directories: Vec<Directory>,
}

impl FileTree {
    fn new(members: &[ArchiveMember]) -> Self {
        let mut directories = vec![Directory::default()];
        for member in members {
            let mut components = match member {
                ArchiveMember::Directory(path) | ArchiveMember::File(path) => path.components(),
            };
            let file = if matches!(member, ArchiveMember::File(_)) {
                components.next_back().map(Component::as_os_str)
            } else {
                None
            };
            let mut parent = 0;
            for component in components {
                let name = component.as_os_str();
                parent = if let Some(&index) = directories[parent].folders.get(name) {
                    index
                } else {
                    let index = directories.len();
                    directories[parent].folders.insert(name.to_owned(), index);
                    directories.push(Directory::default());
                    index
                };
            }
            if let Some(name) = file {
                directories[parent].files.push(name.to_owned());
            }
        }
        Self { directories }
    }

    fn show(&self, ui: &mut Ui, p: Palette, index: usize) {
        let directory = &self.directories[index];
        for (name, &index) in &directory.folders {
            let child = &self.directories[index];
            let has_items = !child.folders.is_empty() || !child.files.is_empty();
            let mut state = has_items.then(|| {
                CollapsingState::load_with_default_open(
                    ui.ctx(),
                    ui.make_persistent_id(("directory", index)),
                    false,
                )
            });
            let symbol = if state.as_ref().is_some_and(CollapsingState::is_open) {
                icon::FOLDER_OPEN
            } else {
                icon::FOLDER
            };
            let name = name.to_string_lossy();
            let response = style::tree_row(
                ui,
                p,
                symbol,
                p.muted,
                RichText::new(name.as_ref()),
                state.as_mut(),
            )
            .on_hover_text(name.as_ref());
            if let Some(state) = &mut state {
                state.show_body_indented(&response, ui, |ui| self.show(ui, p, index));
            }
        }
        for name in &directory.files {
            let name = name.to_string_lossy();
            style::tree_row(
                ui,
                p,
                icon::FILE,
                p.muted,
                RichText::new(name.as_ref()),
                None,
            )
            .on_hover_text(name.as_ref());
        }
    }
}
