// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! The application's fonts and colors, for both themes.

use egui::{
    Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, RichText,
    Stroke, Style, Theme, Visuals, vec2,
};
use std::sync::Arc;

const LATO_REGULAR: &[u8] = include_bytes!("../assets/fonts/Lato-Regular.ttf");
const LATO_BOLD: &[u8] = include_bytes!("../assets/fonts/Lato-Bold.ttf");
const LUCIDE: &[u8] = include_bytes!("../assets/fonts/Lucide.ttf");

const BOLD_FAMILY: &str = "lato-bold";
const ICON_FAMILY: &str = "lucide";

/// Corner radius of buttons and other interactive widgets.
const RADIUS: u8 = 6;
/// Corner radius of compact controls.
const COMPACT_RADIUS: u8 = 2;

/// Colors for one theme.
struct Palette {
    dark: bool,
    /// Marks focus: links, hovered, active and open widgets, and selection.
    accent: Color32,
    /// Warnings and errors.
    alert: Color32,
    panel: Color32,
    window: Color32,
    faint: Color32,
    extreme: Color32,
    code: Color32,
    text: Color32,
    selection_fill: Color32,
}

impl Palette {
    fn dark() -> Self {
        Self {
            dark: true,
            accent: Color32::from_rgb(0x55, 0xBB, 0xB7),
            alert: Color32::from_rgb(0xFF, 0x66, 0x00),
            panel: Color32::from_gray(24),
            window: Color32::from_gray(30),
            faint: Color32::from_gray(28),
            extreme: Color32::from_gray(16),
            code: Color32::from_gray(20),
            text: Color32::from_gray(215),
            selection_fill: Color32::from_rgb(0x17, 0x4F, 0x4D),
        }
    }

    fn light() -> Self {
        Self {
            dark: false,
            accent: Color32::from_rgb(0x2F, 0x67, 0x65),
            alert: Color32::from_rgb(0xB3, 0x47, 0x00),
            panel: Color32::from_gray(246),
            window: Color32::WHITE,
            faint: Color32::from_gray(241),
            extreme: Color32::WHITE,
            code: Color32::from_gray(240),
            text: Color32::from_gray(25),
            selection_fill: Color32::from_rgb(0xD5, 0xEE, 0xED),
        }
    }

    fn visuals(&self) -> Visuals {
        let mut v = if self.dark {
            Visuals::dark()
        } else {
            Visuals::light()
        };
        // Neutral grays for chrome, lighter or darker than the panel.
        let (rest, rest_weak, line, hover_weak) = if self.dark {
            (
                Color32::from_gray(48),
                Color32::from_gray(40),
                Color32::from_gray(62),
                Color32::from_gray(52),
            )
        } else {
            (
                Color32::from_gray(226),
                Color32::from_gray(234),
                Color32::from_gray(200),
                Color32::from_gray(224),
            )
        };
        let strong = if self.dark {
            Color32::from_gray(245)
        } else {
            Color32::BLACK
        };
        let radius = CornerRadius::same(RADIUS);
        let w = &mut v.widgets;

        w.noninteractive.bg_fill = self.panel;
        w.noninteractive.weak_bg_fill = self.panel;
        w.noninteractive.bg_stroke = Stroke::new(1.0, line);
        w.noninteractive.fg_stroke = Stroke::new(1.0, self.text);

        w.inactive.bg_fill = rest;
        w.inactive.weak_bg_fill = rest_weak;
        w.inactive.bg_stroke = Stroke::new(1.0, line);
        w.inactive.fg_stroke = Stroke::new(1.0, self.text);

        w.hovered.bg_fill = hover_weak;
        w.hovered.weak_bg_fill = hover_weak;
        w.hovered.bg_stroke = Stroke::new(1.0, self.accent);
        w.hovered.fg_stroke = Stroke::new(1.5, self.accent);

        w.active.bg_fill = self.accent;
        w.active.weak_bg_fill = self.selection_fill;
        w.active.bg_stroke = Stroke::new(1.0, self.accent);
        w.active.fg_stroke = Stroke::new(1.5, strong);

        w.open.bg_fill = self.selection_fill;
        w.open.weak_bg_fill = self.selection_fill;
        w.open.bg_stroke = Stroke::new(1.0, self.accent);
        w.open.fg_stroke = Stroke::new(1.0, self.accent);

        for widget in [
            &mut w.noninteractive,
            &mut w.inactive,
            &mut w.hovered,
            &mut w.active,
            &mut w.open,
        ] {
            widget.corner_radius = radius;
            // Nothing moves on hover or press; only colors change.
            widget.expansion = 0.0;
        }

        v.selection.bg_fill = self.selection_fill;
        v.selection.stroke = Stroke::new(1.0, self.accent);
        v.hyperlink_color = self.accent;
        v.warn_fg_color = self.alert;
        v.error_fg_color = self.alert;
        v.panel_fill = self.panel;
        v.window_fill = self.window;
        v.window_stroke = Stroke::new(1.0, line);
        v.window_corner_radius = CornerRadius::same(8);
        v.menu_corner_radius = radius;
        v.faint_bg_color = self.faint;
        v.extreme_bg_color = self.extreme;
        v.code_bg_color = self.code;
        v.override_text_color = None;
        // The scroll bar thumb takes the widget's background, so it is gray at rest and
        // accented while dragged.
        v
    }

    fn style(&self, base: &Style) -> Style {
        let mut style = base.clone();
        style.visuals = self.visuals();
        style.spacing.button_padding = vec2(8.0, 6.0);
        style.spacing.scroll.foreground_color = false;
        style
    }
}

/// The application's look: installs fonts and styles, and supplies styled text.
pub struct Look;

impl Look {
    /// Installs the fonts and the dark and light styles into `ctx`.
    pub fn install(ctx: &Context) {
        ctx.data_mut(|d| d.insert_temp(egui::Id::unique("rengrave_look"), true));
        Self::install_fonts(ctx);
        for (theme, palette) in [
            (Theme::Dark, Palette::dark()),
            (Theme::Light, Palette::light()),
        ] {
            ctx.style_mut_of(theme, |style| *style = palette.style(style));
        }
    }

    /// Installs the look unless `ctx` already has it. A host may draw the application into a
    /// different context than the one given to the constructor.
    pub fn ensure_installed(ctx: &Context) {
        let done = ctx.data(|d| d.get_temp::<bool>(egui::Id::unique("rengrave_look")));
        if done != Some(true) {
            Self::install(ctx);
        }
    }

    fn install_fonts(ctx: &Context) {
        let mut fonts = FontDefinitions::default();
        for (name, bytes) in [
            ("lato-regular", LATO_REGULAR),
            ("lato-bold", LATO_BOLD),
            ("lucide", LUCIDE),
        ] {
            fonts
                .font_data
                .insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
        }
        // The built-in fonts stay behind each new one, for glyphs it lacks.
        let fallbacks = fonts
            .families
            .get(&FontFamily::Proportional)
            .cloned()
            .unwrap_or_default();
        let chain = |first: &str| {
            let mut chain = vec![first.to_owned()];
            chain.extend(fallbacks.iter().cloned());
            chain
        };
        fonts
            .families
            .insert(FontFamily::Proportional, chain("lato-regular"));
        fonts
            .families
            .insert(Self::bold_family(), chain("lato-bold"));
        fonts.families.insert(Self::icon_family(), chain("lucide"));
        ctx.set_fonts(fonts);
    }

    /// The family of Lato Bold, for text that should be heavier.
    pub fn bold_family() -> FontFamily {
        FontFamily::Name(Arc::from(BOLD_FAMILY))
    }

    /// The family of the Lucide icon font.
    pub fn icon_family() -> FontFamily {
        FontFamily::Name(Arc::from(ICON_FAMILY))
    }

    /// Bold text, in Lato Bold. `strong()` only changes color.
    pub fn bold(text: impl Into<String>) -> RichText {
        RichText::new(text).family(Self::bold_family())
    }

    /// An icon glyph from [`crate::icons`].
    pub fn icon(glyph: char) -> RichText {
        RichText::new(glyph).font(FontId::new(14.0, Self::icon_family()))
    }

    /// Reduces padding and rounding for controls that sit in dense rows.
    pub fn compact_control_style(style: &mut Style) {
        style.spacing.button_padding = vec2(4.0, 1.0);
        let radius = CornerRadius::same(COMPACT_RADIUS);
        let w = &mut style.visuals.widgets;
        for widget in [
            &mut w.noninteractive,
            &mut w.inactive,
            &mut w.hovered,
            &mut w.active,
            &mut w.open,
        ] {
            widget.corner_radius = radius;
        }
    }

    /// Menu style: the standard menu look, with the application's button padding.
    pub fn menu_style(style: &mut Style) {
        egui::containers::menu::menu_style(style);
        style.spacing.button_padding = vec2(8.0, 6.0);
    }
}
