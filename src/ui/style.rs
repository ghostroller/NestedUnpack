use std::{fs, path::PathBuf};

use eframe::egui::{
    self, Button, Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId,
    Frame, Layout, Margin, Response, RichText, Stroke, TextStyle, Theme, Ui, Visuals, WidgetInfo,
    WidgetType, vec2,
};

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
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
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

pub(super) fn button(icon: &str, label: &str) -> Button<'static> {
    Button::new(format!("{icon}  {label}")).min_size(vec2(0.0, 34.0))
}

pub(super) fn primary(icon: &str, label: &str, p: Palette) -> Button<'static> {
    Button::new(
        RichText::new(format!("{icon}  {label}"))
            .color(Color32::WHITE)
            .strong(),
    )
    .fill(p.accent_fill)
    .stroke(Stroke::NONE)
    .min_size(vec2(0.0, 36.0))
}

pub(super) fn icon_button(ui: &mut Ui, symbol: &str, label: &str) -> Response {
    let response = ui.add(Button::new(RichText::new(symbol).size(17.0)).min_size(vec2(34.0, 34.0)));
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
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        FontId::proportional(side * 0.49),
        p.accent,
    );
}
