use std::{fs, path::PathBuf};

use eframe::egui::{
    self, Align2, Atom, AtomExt as _, AtomLayout, AtomLayoutResponse, Button, Color32, Context,
    CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Frame, Galley, Id, Layout, Margin,
    Pos2, Response, RichText, Sense, Stroke, TextStyle, TextWrapMode, Theme, Ui, Visuals, Widget,
    WidgetInfo, WidgetText, WidgetType,
    collapsing_header::CollapsingState,
    emath::{GuiRounding as _, Rot2},
    vec2,
};
use egui_phosphor::regular as icon;

use crate::{
    i18n::{AppKey, format as localize, tr},
    session::Status,
};

const ICON_FONT: &str = "phosphor";

#[derive(Clone, Copy)]
pub(super) struct Palette {
    pub canvas: Color32,
    pub surface: Color32,
    pub softened: Color32,
    pub line: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub accent_fill: Color32,
    pub accent_soft: Color32,
    pub positive: Color32,
    pub warning: Color32,
    pub warning_soft: Color32,
    pub danger: Color32,
}

pub(super) fn status_style(p: Palette, status: Status) -> (&'static str, &'static str, Color32) {
    match status {
        Status::Pending => (icon::CLOCK, tr(AppKey::Pending), p.muted),
        Status::Running => (icon::SPINNER, tr(AppKey::Running), p.accent),
        Status::Success => (icon::CHECK_CIRCLE, tr(AppKey::Success), p.positive),
        Status::Skipped => (icon::SKIP_FORWARD, tr(AppKey::Skipped), p.warning),
        Status::Failed => (icon::WARNING_CIRCLE, tr(AppKey::Failed), p.danger),
        Status::Canceled => (icon::STOP_CIRCLE, tr(AppKey::Canceled), p.muted),
    }
}

const fn rgb(red: u8, green: u8, blue: u8) -> Color32 {
    Color32::from_rgb(red, green, blue)
}

impl Palette {
    pub fn current(ctx: &Context) -> Self {
        Self::for_theme(ctx.theme())
    }

    fn for_theme(theme: Theme) -> Self {
        match theme {
            Theme::Light => Self {
                canvas: rgb(244, 247, 252),
                surface: Color32::WHITE,
                softened: rgb(246, 248, 252),
                line: rgb(218, 225, 237),
                text: rgb(24, 36, 55),
                muted: rgb(87, 104, 127),
                accent: rgb(36, 82, 183),
                accent_fill: rgb(42, 88, 197),
                accent_soft: rgb(230, 238, 255),
                positive: rgb(21, 114, 82),
                warning: rgb(131, 78, 20),
                warning_soft: rgb(252, 239, 214),
                danger: rgb(164, 49, 64),
            },
            Theme::Dark => Self {
                canvas: rgb(16, 23, 36),
                surface: rgb(24, 34, 51),
                softened: rgb(30, 42, 61),
                line: rgb(49, 65, 90),
                text: rgb(232, 239, 250),
                muted: rgb(166, 181, 203),
                accent: rgb(149, 180, 255),
                accent_fill: rgb(54, 101, 217),
                accent_soft: rgb(37, 56, 93),
                positive: rgb(126, 220, 182),
                warning: rgb(244, 192, 117),
                warning_soft: rgb(66, 52, 33),
                danger: rgb(255, 154, 168),
            },
        }
    }

    pub fn card(self, margin: i8) -> Frame {
        Frame::new()
            .fill(self.surface)
            .stroke(Stroke::new(1.0, self.line))
            .corner_radius(12)
            .inner_margin(Margin::same(margin))
    }
}

pub(super) fn configure(ctx: &Context) {
    let mut fonts = FontDefinitions::default();
    let (mut ui_font, mut cjk_font) = (None, None);
    if cfg!(windows) {
        let font_dir = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
            .join("Fonts");
        let read = |names: &[&str]| {
            names
                .iter()
                .find_map(|name| fs::read(font_dir.join(name)).ok())
                .map(FontData::from_owned)
        };
        ui_font = read(&["segoeui.ttf", "arial.ttf"]);
        cjk_font = read(&["msyh.ttc", "Deng.ttf", "simsun.ttc"]);
    }
    if ui_font.is_none() || cjk_font.is_none() {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let find = |families: &[fontdb::Family<'_>]| {
            let id = db.query(&fontdb::Query {
                families,
                ..Default::default()
            })?;
            db.with_face_data(id, |data, index| {
                let mut face = FontData::from_owned(data.to_vec());
                face.index = index;
                face
            })
        };
        use fontdb::Family::{Name, SansSerif};
        ui_font = ui_font.or_else(|| {
            find(&[
                Name("SF Pro Text"),
                Name("Helvetica Neue"),
                Name("Noto Sans"),
                Name("DejaVu Sans"),
                Name("Liberation Sans"),
                Name("Arial"),
                SansSerif,
            ])
        });
        cjk_font = cjk_font.or_else(|| {
            find(&[
                Name("PingFang SC"),
                Name("Noto Sans CJK SC"),
                Name("Microsoft YaHei"),
                Name("WenQuanYi Zen Hei"),
                Name("Songti SC"),
                Name("SimSun"),
            ])
        });
    }
    for (name, font, first) in [
        ("system-ui", ui_font, true),
        ("system-cjk", cjk_font, false),
    ] {
        let Some(font) = font else {
            continue;
        };
        fonts.font_data.insert(name.into(), font.into());
        let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
        if first {
            proportional.insert(0, name.into());
        } else {
            proportional.push(name.into());
        }
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .push(name.into());
    }
    fonts.font_data.insert(
        ICON_FONT.into(),
        FontData::from_static(egui_phosphor::Variant::Regular.font_bytes()).into(),
    );
    fonts
        .families
        .insert(FontFamily::Name(ICON_FONT.into()), vec![ICON_FONT.into()]);
    ctx.set_fonts(fonts);

    for theme in [Theme::Light, Theme::Dark] {
        let p = Palette::for_theme(theme);
        ctx.style_mut_of(theme, |style| {
            let mut visuals = match theme {
                Theme::Light => Visuals::light(),
                Theme::Dark => Visuals::dark(),
            };
            visuals.panel_fill = p.canvas;
            visuals.window_fill = p.surface;
            visuals.extreme_bg_color = p.softened;
            visuals.faint_bg_color = p.softened;
            visuals.hyperlink_color = p.accent;
            visuals.override_text_color = Some(p.text);
            visuals.selection.bg_fill = p.accent_soft;
            visuals.selection.stroke = Stroke::new(1.0, p.accent);
            visuals.widgets.inactive.bg_fill = p.surface;
            visuals.widgets.inactive.weak_bg_fill = p.surface;
            visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, p.line);
            visuals.widgets.hovered.bg_fill = p.accent_soft;
            visuals.widgets.hovered.weak_bg_fill = p.accent_soft;
            visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, p.accent);
            visuals.widgets.active.bg_fill = p.accent_soft;
            visuals.widgets.active.weak_bg_fill = p.accent_soft;
            visuals.widgets.active.bg_stroke = Stroke::new(1.0, p.accent);
            for widget in [
                &mut visuals.widgets.inactive,
                &mut visuals.widgets.hovered,
                &mut visuals.widgets.active,
            ] {
                widget.corner_radius = CornerRadius::same(7);
                widget.fg_stroke.color = p.text;
            }
            style.visuals = visuals;
            style.interaction.selectable_labels = false;
            style.spacing.item_spacing = vec2(8.0, 8.0);
            style.spacing.button_padding = vec2(12.0, 6.0);
            style.spacing.interact_size.y = 30.0;
            style.spacing.indent = 18.0;
            style
                .text_styles
                .insert(TextStyle::Heading, FontId::proportional(20.0));
            style
                .text_styles
                .insert(TextStyle::Body, FontId::proportional(14.0));
            style
                .text_styles
                .insert(TextStyle::Button, FontId::proportional(14.0));
            style
                .text_styles
                .insert(TextStyle::Small, FontId::proportional(12.0));
        });
    }
}

fn paint_icon(ui: &Ui, center: Pos2, symbol: &str, size: f32, color: Color32) {
    let font = FontId::new(size, FontFamily::Name(ICON_FONT.into()));
    let galley = ui
        .painter()
        .layout_no_wrap(symbol.to_owned(), font, Color32::PLACEHOLDER);
    if galley.mesh_bounds.is_positive() {
        ui.painter().galley(
            center - galley.mesh_bounds.center().to_vec2(),
            galley,
            color,
        );
    }
}

struct InlineIcon<'a> {
    symbol: &'a str,
    size: f32,
    text: RichText,
}

impl InlineIcon<'_> {
    fn atoms(&self, slot_size: f32) -> (Atom<'static>, Atom<'static>) {
        (
            Atom::custom(Id::new("icon"), vec2(slot_size, slot_size)),
            self.text
                .clone()
                .atom_id(Id::new("caption"))
                .atom_shrink(true),
        )
    }

    fn paint(
        &self,
        ui: &Ui,
        layout: &AtomLayoutResponse,
        color: Color32,
        font: TextStyle,
        galley: Option<&Galley>,
    ) -> Option<Pos2> {
        let (Some(icon), Some(caption)) = (
            layout.rect(Id::new("icon")),
            layout.rect(Id::new("caption")),
        ) else {
            return None;
        };
        if !ui.is_rect_visible(icon) {
            return None;
        }

        let fallback;
        let galley = match galley {
            Some(galley) => galley,
            None => {
                fallback = WidgetText::from(self.text.clone()).into_galley(
                    ui,
                    Some(TextWrapMode::Extend),
                    f32::INFINITY,
                    font,
                );
                &fallback
            }
        };
        let mut center = icon.center();
        if let Some(line) = galley.rows.first()
            && line.row.visuals.mesh_bounds.is_positive()
        {
            let pixels_per_point = ui.pixels_per_point();
            let mut top = f32::INFINITY;
            let mut bottom = f32::NEG_INFINITY;
            // Align with the body of the first line. An extension such as .mp4
            // should not pull the icon down with its punctuation and descenders.
            for glyph in &line.row.glyphs {
                if !glyph.chr.is_alphanumeric() || glyph.uv_rect.is_nothing() {
                    continue;
                }
                let glyph_top =
                    (glyph.pos.y + glyph.uv_rect.offset.y).round_to_pixels(pixels_per_point);
                let mut glyph_bottom = glyph_top + glyph.uv_rect.size.y;
                if glyph.chr.is_ascii_alphanumeric() {
                    glyph_bottom = glyph_bottom.min(glyph.pos.y.round_to_pixels(pixels_per_point));
                }
                top = top.min(glyph_top);
                bottom = bottom.max(glyph_bottom);
            }
            let line_center = if top < bottom {
                (top + bottom) * 0.5
            } else {
                line.row.visuals.mesh_bounds.center().y
            };
            let mut caption_top = caption.top();
            if ui
                .ctx()
                .tessellation_options(|options| options.round_text_to_pixels)
            {
                caption_top = caption_top.round_to_pixels(pixels_per_point);
            }
            center.y = caption_top + line.pos.y + line_center;
        }
        paint_icon(ui, center, self.symbol, self.size, color);
        Some(center)
    }
}

pub(super) fn icon_label(
    ui: &mut Ui,
    symbol: &str,
    size: f32,
    color: Color32,
    text: RichText,
    mode: TextWrapMode,
) -> Response {
    let content = InlineIcon { symbol, size, text };
    let prepared = AtomLayout::new(content.atoms(size))
        .gap(6.0)
        .fallback_font(TextStyle::Body)
        .wrap_mode(mode)
        .allocate(ui);
    let galley = prepared.sized.iter_texts().next().cloned();
    let layout = prepared.paint(ui);
    let _ = content.paint(ui, &layout, color, TextStyle::Body, galley.as_deref());
    layout.response.widget_info(|| {
        WidgetInfo::labeled(WidgetType::Label, ui.is_enabled(), content.text.text())
    });
    layout.response
}

pub(super) fn tree_row(
    ui: &mut Ui,
    p: Palette,
    symbol: &str,
    color: Color32,
    text: RichText,
    state: Option<&mut CollapsingState>,
) -> Response {
    let content = InlineIcon {
        symbol,
        size: 14.0,
        text,
    };
    let indent = ui.spacing().indent;
    // Equal columns put each icon over the next level's arrow and indent guide.
    let (icon, caption) = content.atoms(indent);
    let disclosure = Id::new("disclosure");
    let slot = Atom::custom(disclosure, vec2(indent, content.size));
    let caption_gap = Atom::default().atom_size(vec2(4.0, 0.0));
    let mut layout = AtomLayout::new((slot, icon, caption_gap, caption))
        .align2(Align2::LEFT_CENTER)
        .gap(0.0)
        .min_size(vec2(0.0, 23.0))
        .fallback_font(TextStyle::Body)
        .wrap_mode(TextWrapMode::Truncate);
    if let Some(branch) = &state {
        layout = layout.id(branch.id().with("header")).sense(Sense::click());
    }
    let prepared = layout.allocate(ui);
    let galley = prepared.sized.iter_texts().next().cloned();
    let layout = prepared.paint(ui);
    let icon_center = content.paint(ui, &layout, color, TextStyle::Body, galley.as_deref());
    let response = &layout.response;

    if let Some(state) = state {
        if response.clicked() {
            state.toggle(ui);
        }
        if let (Some(slot), Some(icon_center)) = (layout.rect(disclosure), icon_center) {
            let center = egui::pos2(slot.center().x, icon_center.y);
            let angle = (state.openness(ui.ctx()) - 1.0) * std::f32::consts::FRAC_PI_2;
            let rotation = Rot2::from_angle(angle);
            let points = [vec2(-3.0, -1.5), vec2(0.0, 1.5), vec2(3.0, -1.5)]
                .map(|point| center + rotation * point);
            let color = if response.hovered() || response.has_focus() {
                p.accent
            } else {
                p.muted
            };
            ui.painter()
                .add(egui::Shape::line(points.to_vec(), Stroke::new(1.35, color)));
        }
        response.widget_info(|| {
            let action = if state.is_open() {
                AppKey::Collapse
            } else {
                AppKey::Expand
            };
            WidgetInfo::labeled(
                WidgetType::CollapsingHeader,
                ui.is_enabled(),
                localize(action, &[&content.text.text()]),
            )
        });
    } else {
        response.widget_info(|| {
            WidgetInfo::labeled(WidgetType::Label, ui.is_enabled(), content.text.text())
        });
    }
    layout.response
}

pub(super) fn icon_text_button<'a>(
    symbol: &'a str,
    size: f32,
    color: Option<Color32>,
    text: RichText,
    configure: impl FnOnce(Button<'static>) -> Button<'static> + 'a,
) -> impl Widget + 'a {
    move |ui: &mut Ui| {
        let content = InlineIcon { symbol, size, text };
        let layout = configure(Button::new(content.atoms(size)).gap(6.0)).atom_ui(ui);
        let color = color.unwrap_or_else(|| ui.style().interact(&layout.response).fg_stroke.color);
        let _ = content.paint(ui, &layout, color, TextStyle::Button, None);
        layout.response
    }
}

pub(super) fn button<'a>(symbol: &'a str, label: &str) -> impl Widget + 'a + use<'a> {
    icon_text_button(symbol, 14.0, None, RichText::new(label), |button| {
        button.min_size(vec2(0.0, 34.0))
    })
}

pub(super) fn primary<'a>(symbol: &'a str, label: &str, p: Palette) -> impl Widget + 'a + use<'a> {
    icon_text_button(
        symbol,
        14.0,
        Some(Color32::WHITE),
        RichText::new(label).color(Color32::WHITE).strong(),
        move |button| {
            button
                .fill(p.accent_fill)
                .stroke(Stroke::NONE)
                .min_size(vec2(0.0, 36.0))
        },
    )
}

pub(super) fn icon_button(ui: &mut Ui, symbol: &str, label: &str) -> Response {
    let response = ui.add(
        Button::new(Atom::custom(Id::new("icon"), vec2(17.0, 17.0))).min_size(vec2(34.0, 34.0)),
    );
    let color = ui.style().interact(&response).fg_stroke.color;
    if ui.is_rect_visible(response.rect) {
        paint_icon(ui, response.rect.center(), symbol, 17.0, color);
    }
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), label));
    response.on_hover_text(label)
}

pub(super) fn row<R>(
    ui: &mut Ui,
    height: f32,
    layout: Layout,
    content: impl FnOnce(&mut Ui) -> R,
) -> R {
    let size = vec2(ui.available_width().max(0.0), height);
    ui.allocate_ui_with_layout(size, layout, content).inner
}

pub(super) fn glyph_tile(ui: &mut Ui, icon: &str, p: Palette, side: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(side, side), egui::Sense::hover());
    ui.painter().rect_filled(rect, 11, p.accent_soft);
    paint_icon(ui, rect.center(), icon, side * 0.49, p.accent);
}

pub(super) fn icon_heading(ui: &mut Ui, symbol: &str, p: Palette, label: &str) {
    let content = InlineIcon {
        symbol,
        size: 18.0,
        text: RichText::new(label).size(20.0).strong(),
    };
    let layout = AtomLayout::new(content.atoms(36.0))
        .gap(10.0)
        .fallback_font(TextStyle::Body)
        .show(ui);
    if let Some(tile) = layout.rect(Id::new("icon")) {
        ui.painter().rect_filled(tile, 11, p.accent_soft);
    }
    let _ = content.paint(ui, &layout, p.accent, TextStyle::Body, None);
    layout
        .response
        .widget_info(|| WidgetInfo::labeled(WidgetType::Label, ui.is_enabled(), label));
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use egui::{RawInput, Shape, ViewportId};
    use egui_phosphor::regular as icon;

    #[test]
    fn tree_rows_align_disclosure_and_status_columns() {
        let ctx = Context::default();
        configure(&ctx);
        for scale in [1.0, 1.25, 1.75, 2.0] {
            let mut input = RawInput::default();
            input
                .viewports
                .get_mut(&ViewportId::ROOT)
                .unwrap()
                .native_pixels_per_point = Some(scale);
            let mut output = ctx.run_ui(input, |ui| {
                let p = Palette::current(ui.ctx());
                let draw_row = |ui: &mut Ui, name: &str, state: Option<&mut CollapsingState>| {
                    tree_row(
                        ui,
                        p,
                        icon::CHECK_CIRCLE,
                        p.positive,
                        RichText::new(name),
                        state,
                    )
                };
                let mut branch =
                    CollapsingState::load_with_default_open(ui.ctx(), Id::new("branch"), true);
                let response = draw_row(ui, "项目.zip · ZIP", Some(&mut branch));
                branch.show_body_indented(&response, ui, |ui| {
                    let mut child =
                        CollapsingState::load_with_default_open(ui.ctx(), Id::new("child"), false);
                    draw_row(ui, "内层.zip · ZIP", Some(&mut child));
                });
                draw_row(ui, "leaf.zip · ZIP", None);
            });
            output.textures_delta.clear();
            let status_centers: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    Shape::Text(text) if text.galley.text() == icon::CHECK_CIRCLE => Some(
                        text.pos.round_to_pixels(scale)
                            + text.galley.mesh_bounds.center().to_vec2(),
                    ),
                    _ => None,
                })
                .collect();
            let chevrons: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    Shape::Path(path) if path.points.len() == 3 => Some(&path.points),
                    _ => None,
                })
                .collect();
            assert_eq!(status_centers.len(), 3);
            assert_eq!(chevrons.len(), 2);
            let root_status = status_centers[0];
            assert_eq!(root_status.x, status_centers[2].x);
            let root_arrow_y = (chevrons[0][0].y + chevrons[0][1].y) * 0.5;
            assert!((root_arrow_y - root_status.y).abs() * scale <= 0.51);
            let child_arrow_x = (chevrons[1][0].x + chevrons[1][1].x) * 0.5;
            assert!(
                (child_arrow_x - root_status.x).abs() * scale <= 0.51,
                "scale {scale}: parent status {} vs child disclosure {child_arrow_x}",
                root_status.x
            );
        }
    }

    #[test]
    fn archive_icon_centers_on_mixed_script_filename() {
        let ctx = Context::default();
        configure(&ctx);
        let caption = "归档预览.mp4 · ZIP";
        for scale in [1.0, 1.25, 1.75, 2.0] {
            let mut input = RawInput::default();
            input
                .viewports
                .get_mut(&ViewportId::ROOT)
                .unwrap()
                .native_pixels_per_point = Some(scale);
            let mut output = ctx.run_ui(input, |ui| {
                let p = Palette::current(ui.ctx());
                icon_label(
                    ui,
                    icon::CHECK_CIRCLE,
                    14.0,
                    p.positive,
                    RichText::new(caption).color(p.positive),
                    TextWrapMode::Truncate,
                );
            });
            output.textures_delta.clear();
            let shape = |label| {
                output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        Shape::Text(text) if text.galley.text() == label => Some(text),
                        _ => None,
                    })
                    .unwrap()
            };
            let text = shape(caption);
            let line = &text.galley.rows[0];
            let pixel_y = |y: f32| (y * scale).round();
            let (top, bottom) = line.row.glyphs.iter().take(4).fold(
                (f32::INFINITY, f32::NEG_INFINITY),
                |(top, bottom), glyph| {
                    let glyph_top = pixel_y(glyph.pos.y + glyph.uv_rect.offset.y);
                    (
                        top.min(glyph_top),
                        bottom.max(glyph_top + glyph.uv_rect.size.y * scale),
                    )
                },
            );
            let caption_center = pixel_y(text.pos.y) + line.pos.y * scale + (top + bottom) * 0.5;
            let icon = shape(icon::CHECK_CIRCLE);
            let icon_center = pixel_y(icon.pos.y) + icon.galley.mesh_bounds.center().y * scale;
            assert!(
                (caption_center - icon_center).abs() <= 1.0,
                "scale {scale}: {caption_center} vs {icon_center}"
            );
        }
    }
}
