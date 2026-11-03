// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! Small reusable pieces of the settings panel.

use crate::style::Look;
use egui::{Color32, DragValue, Response, RichText, Sense, Stroke, Ui, pos2, vec2};
use rengrave_core::{BitShape, Issue, Origin, Settings};

/// A two-column form that labels each row, shows its units, and flags invalid values.
pub struct Form<'a> {
    /// Problems with the current settings.
    pub issues: &'a [Issue],
    /// Suffix for lengths, such as `" in"`.
    pub length_unit: &'a str,
}

/// How a numeric row should behave.
#[derive(Clone, Copy)]
pub struct Number<'a> {
    /// Visible row name; also the accessible name.
    pub label: &'a str,
    /// The [`Settings`] field, for finding validation issues.
    pub field: &'a str,
    /// Explanation shown on hover.
    pub tip: &'a str,
    /// Suffix after the number.
    pub suffix: &'a str,
    /// Change per pixel dragged.
    pub speed: f64,
}

impl Form<'_> {
    /// Adds a numeric row.
    pub fn number(&mut self, ui: &mut Ui, spec: Number<'_>, value: &mut f64) -> Response {
        let issue = self.issues.iter().find(|i| i.field == spec.field);
        let text = if issue.is_some() {
            RichText::new(spec.label).color(ui.visuals().error_fg_color)
        } else {
            ui.control_label(spec.label)
        };
        let label = ui.label(text).on_hover_text(spec.tip);
        let response = ui
            .compact(|ui| {
                ui.add(
                    DragValue::new(value)
                        .speed(spec.speed)
                        .suffix(spec.suffix.to_string())
                        .max_decimals(5),
                )
            })
            .labelled_by(label.id)
            .on_hover_text(issue.map_or(spec.tip, |i| i.message));
        ui.end_row();
        if let Some(issue) = issue {
            ui.label("");
            ui.colored_label(ui.visuals().error_fg_color, issue.message);
            ui.end_row();
        }
        response
    }

    /// Adds a numeric row in the job's length unit.
    pub fn length(
        &mut self,
        ui: &mut Ui,
        label: &str,
        field: &str,
        tip: &str,
        value: &mut f64,
    ) -> Response {
        let unit = self.length_unit;
        let speed = if unit.contains("mm") { 0.05 } else { 0.002 };
        self.number(
            ui,
            Number {
                label,
                field,
                tip,
                suffix: unit,
                speed,
            },
            value,
        )
    }

    /// Adds a checkbox row.
    pub fn check(&mut self, ui: &mut Ui, label: &str, tip: &str, value: &mut bool) {
        ui.label("");
        ui.compact(|ui| ui.checkbox(value, label))
            .on_hover_text(tip);
        ui.end_row();
    }
}

/// Composite controls for the settings panel.
pub trait UiExt {
    /// Runs `add` with the dense padding and rounding used by sliders, drag values, checkboxes,
    /// and segmented pills.
    fn compact<R>(&mut self, add: impl FnOnce(&mut Ui) -> R) -> R;

    /// Wraps a row of options in one bordered, rounded frame.
    fn option_group(&mut self, add: impl FnOnce(&mut Ui));

    /// A row of tabs that connects to the pane drawn below it. Each choice is a value, a name, and
    /// a tooltip. A line runs across the available width, open beneath the selected tab.
    fn tab_bar<T: Copy + PartialEq>(&mut self, value: &mut T, choices: &[(T, &str, &str)]);

    /// Text for a label that names a control, in the accent color.
    fn control_label(&self, text: impl Into<String>) -> RichText;

    /// Text for a section heading: bold, in the accent color.
    fn section_heading(&self, text: impl Into<String>) -> RichText;

    /// A button with an icon before its text. Its accessible name is the text alone.
    fn icon_button(&mut self, enabled: bool, icon: char, text: &str) -> Response;

    /// A button that switches `value` on and off, and looks like a button in both states.
    fn toggle_button(&mut self, value: &mut bool, text: &str) -> Response;

    /// A menu entry with an icon, and optionally the text of its keyboard shortcut.
    fn menu_item(
        &mut self,
        enabled: bool,
        icon: char,
        text: &str,
        shortcut: Option<String>,
    ) -> Response;

    /// A row of mutually exclusive buttons. Returns `true` if the choice changed.
    fn segmented<T: Copy + PartialEq>(
        &mut self,
        value: &mut T,
        choices: &[(T, &str, &str)],
    ) -> bool;

    /// A 3-by-3 grid of dots for choosing the work origin relative to the design's bounding box.
    fn origin_grid(&mut self, value: &mut Origin) -> bool;

    /// Draws a cross-section of the tool at its full cutting depth.
    fn bit_profile(&mut self, settings: &Settings);

    /// Lays out a two-column form with a consistent label width.
    fn form_grid(&mut self, id: &str, add: impl FnOnce(&mut Ui));

    /// A labeled single-line text field, with a small button that restores `default`.
    fn resettable_text(&mut self, name: &str, tip: &str, value: &mut String, default: &str);
}

/// The geometry of one tab.
struct TabShape;

impl TabShape {
    const PADDING: egui::Vec2 = egui::vec2(12.0, 5.0);
    const RADIUS: f32 = 6.0;
    const CORNER_STEPS: u32 = 4;

    /// The tab's outline, up one side, around the top, and down the other, open at the bottom.
    fn outline(rect: egui::Rect) -> Vec<egui::Pos2> {
        let r = Self::RADIUS.min(rect.width() / 2.0).min(rect.height());
        let quarter = std::f32::consts::FRAC_PI_2;
        let mut points = vec![rect.left_bottom()];
        let corners = [
            (rect.left() + r, 2.0 * quarter),
            (rect.right() - r, 3.0 * quarter),
        ];
        for (center_x, start) in corners {
            for step in 0..=Self::CORNER_STEPS {
                let angle = start + quarter * step as f32 / Self::CORNER_STEPS as f32;
                let (sin, cos) = angle.sin_cos();
                points.push(pos2(center_x + r * cos, rect.top() + r + r * sin));
            }
        }
        points.push(rect.right_bottom());
        points
    }
}

impl UiExt for Ui {
    fn tab_bar<T: Copy + PartialEq>(&mut self, value: &mut T, choices: &[(T, &str, &str)]) {
        let row = self.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let mut tabs = Vec::with_capacity(choices.len());
            for &(choice, name, tip) in choices {
                let galley = egui::WidgetText::from(name).into_galley(
                    ui,
                    Some(egui::TextWrapMode::Extend),
                    f32::INFINITY,
                    egui::TextStyle::Button,
                );
                let size = galley.size() + 2.0 * TabShape::PADDING;
                let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                let selected = *value == choice;
                response.widget_info(|| {
                    egui::WidgetInfo::selected(egui::Role::Tab, true, selected, name)
                });
                let response = response.on_hover_text(tip);
                if response.clicked() {
                    *value = choice;
                }
                tabs.push((rect, response, galley, *value == choice));
            }
            tabs
        });
        let tabs = row.inner;
        let visuals = self.visuals().clone();
        let line = visuals.widgets.noninteractive.bg_stroke;
        let panel = visuals.panel_fill;
        let painter = self.painter().clone();
        let clip = self.clip_rect();
        let bottom = row.response.rect.bottom().floor() + 0.5;

        // Inactive tabs first, so that the line runs across their feet.
        for (rect, response, galley, selected) in &tabs {
            let (x_min, x_max) = (rect.left().floor() + 0.5, rect.right().floor() + 0.5);
            let shape = egui::Rect::from_min_max(
                pos2(x_min, rect.top().floor() + 0.5),
                pos2(x_max, bottom),
            );
            let radius = TabShape::RADIUS as u8;
            let rounding = egui::CornerRadius {
                nw: radius,
                ne: radius,
                sw: 0,
                se: 0,
            };
            let (fill, text) = if *selected {
                (panel, visuals.hyperlink_color)
            } else if response.hovered() {
                (
                    visuals.widgets.hovered.weak_bg_fill,
                    visuals.strong_text_color(),
                )
            } else {
                (visuals.faint_bg_color, visuals.text_color())
            };
            painter.rect_filled(shape, rounding, fill);
            let outline = if response.has_focus() {
                Stroke::new(1.0, visuals.hyperlink_color)
            } else {
                line
            };
            painter.add(egui::Shape::line(TabShape::outline(shape), outline));
            painter.galley(rect.min + TabShape::PADDING, galley.clone(), text);
        }
        let gap = tabs
            .iter()
            .find(|(.., selected)| *selected)
            .map(|(rect, ..)| (rect.left().floor() + 0.5, rect.right().floor() + 0.5));
        let (left, right) = (clip.left(), clip.right());
        match gap {
            Some((a, b)) => {
                painter.line_segment([pos2(left, bottom), pos2(a, bottom)], line);
                painter.line_segment([pos2(b, bottom), pos2(right, bottom)], line);
            }
            None => {
                painter.line_segment([pos2(left, bottom), pos2(right, bottom)], line);
            }
        }
    }

    fn compact<R>(&mut self, add: impl FnOnce(&mut Ui) -> R) -> R {
        self.scope(|ui| {
            Look::compact_control_style(ui.style_mut());
            add(ui)
        })
        .inner
    }

    fn option_group(&mut self, add: impl FnOnce(&mut Ui)) {
        let stroke = self.visuals().widgets.noninteractive.bg_stroke;
        egui::Frame::new()
            .stroke(stroke)
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::same(2))
            .show(self, |ui| {
                ui.horizontal(|ui| {
                    Look::compact_control_style(ui.style_mut());
                    let style = ui.style_mut();
                    style.spacing.item_spacing.x = 2.0;
                    // Options sit flat in the frame until pointed at.
                    let rest = &mut style.visuals.widgets.inactive;
                    rest.weak_bg_fill = Color32::TRANSPARENT;
                    rest.bg_stroke = Stroke::new(1.0, Color32::TRANSPARENT);
                    add(ui);
                });
            });
    }

    fn control_label(&self, text: impl Into<String>) -> RichText {
        RichText::new(text).color(self.visuals().hyperlink_color)
    }

    fn section_heading(&self, text: impl Into<String>) -> RichText {
        Look::bold(text).color(self.visuals().hyperlink_color)
    }

    fn toggle_button(&mut self, value: &mut bool, text: &str) -> Response {
        let mut response = self.add(egui::Button::new(text).selected(*value));
        if response.clicked() {
            *value = !*value;
            response.mark_changed();
        }
        response
    }

    fn icon_button(&mut self, enabled: bool, icon: char, text: &str) -> Response {
        let button = egui::Button::new((Look::icon(icon), text));
        let response = self.add_enabled(enabled, button);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, text));
        response
    }

    fn menu_item(
        &mut self,
        enabled: bool,
        icon: char,
        text: &str,
        shortcut: Option<String>,
    ) -> Response {
        let mut button = egui::Button::new((Look::icon(icon), text));
        // The accessible name includes the shortcut, as it does for any text in the button.
        let mut name = text.to_owned();
        if let Some(shortcut) = shortcut {
            name = format!("{text} {shortcut}");
            button = button.shortcut_text(shortcut);
        }
        let response = self.add_enabled(enabled, button);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, &name));
        response
    }

    fn resettable_text(&mut self, name: &str, tip: &str, value: &mut String, default: &str) {
        let label = self
            .horizontal(|ui| {
                let label = ui.label(ui.control_label(name)).on_hover_text(tip);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let differs = value != default;
                    let reset = ui
                        .add_enabled(
                            differs,
                            egui::Button::new((Look::icon(crate::icons::RESET), "Reset")).small(),
                        )
                        .on_hover_text(format!("Restore the default: {default}"));
                    reset.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::Role::Button,
                            differs,
                            format!("Reset {}", name.to_lowercase()),
                        )
                    });
                    if reset.clicked() {
                        default.clone_into(value);
                    }
                });
                label
            })
            .inner;
        self.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY))
            .labelled_by(label.id);
    }

    fn segmented<T: Copy + PartialEq>(
        &mut self,
        value: &mut T,
        choices: &[(T, &str, &str)],
    ) -> bool {
        let mut changed = false;
        self.option_group(|ui| {
            for &(choice, name, tip) in choices {
                if ui
                    .selectable_label(*value == choice, name)
                    .on_hover_text(tip)
                    .clicked()
                    && *value != choice
                {
                    *value = choice;
                    changed = true;
                }
            }
        });
        changed
    }

    fn origin_grid(&mut self, value: &mut Origin) -> bool {
        const ROWS: [[(Origin, &str); 3]; 3] = [
            [
                (Origin::TopLeft, "Top left"),
                (Origin::TopCenter, "Top center"),
                (Origin::TopRight, "Top right"),
            ],
            [
                (Origin::MidLeft, "Middle left"),
                (Origin::MidCenter, "Middle center"),
                (Origin::MidRight, "Middle right"),
            ],
            [
                (Origin::BotLeft, "Bottom left"),
                (Origin::BotCenter, "Bottom center"),
                (Origin::BotRight, "Bottom right"),
            ],
        ];
        const CELL: f32 = 22.0;
        let mut changed = false;
        egui::Grid::new("origin_grid")
            .spacing(vec2(0.0, 0.0))
            .min_col_width(CELL)
            .min_row_height(CELL)
            .show(self, |ui| {
                for row in ROWS {
                    for (origin, name) in row {
                        let (rect, response) =
                            ui.allocate_exact_size(vec2(CELL, CELL), Sense::click());
                        let selected = *value == origin;
                        response.widget_info(|| {
                            egui::WidgetInfo::selected(
                                egui::Role::RadioButton,
                                true,
                                selected,
                                name,
                            )
                        });
                        let response = response
                            .on_hover_text(format!("Origin at the {}", name.to_lowercase()));
                        if response.clicked() && !selected {
                            *value = origin;
                            changed = true;
                        }
                        let visuals = ui.style().interact_selectable(&response, selected);
                        ui.painter().rect_stroke(
                            rect.shrink(1.0),
                            1.0,
                            Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
                            egui::StrokeKind::Inside,
                        );
                        let radius = if selected { 5.0 } else { 3.0 };
                        let color = if selected {
                            ui.visuals().selection.bg_fill
                        } else {
                            visuals.fg_stroke.color
                        };
                        ui.painter().circle_filled(rect.center(), radius, color);
                    }
                    ui.end_row();
                }
            });
        changed
    }

    fn bit_profile(&mut self, settings: &Settings) {
        let (rect, _) = self.allocate_exact_size(
            vec2(self.available_width().min(220.0), 72.0),
            Sense::hover(),
        );
        let painter = self.painter_at(rect);
        let stroke = Stroke::new(1.5, self.visuals().strong_text_color());
        let surface = rect.top() + 22.0;
        painter.line_segment(
            [pos2(rect.left(), surface), pos2(rect.right(), surface)],
            Stroke::new(1.0, self.visuals().weak_text_color()),
        );

        let cx = rect.center().x;
        let depth = (-settings.max_cut_depth()).max(1e-9) as f32;
        let radius = (settings.effective_bit_dia() / 2.0).max(1e-9) as f32;
        let scale = (46.0 / depth).min(60.0 / radius);
        let (d, r) = (depth * scale, radius * scale);
        if !d.is_finite() || !r.is_finite() {
            return;
        }
        let tip = pos2(cx, surface + d);
        let cut_fill = Color32::from_rgba_unmultiplied(120, 170, 220, 90);
        match settings.bit_shape {
            BitShape::VBit => {
                let left = pos2(cx - r, surface);
                let right = pos2(cx + r, surface);
                painter.add(egui::Shape::convex_polygon(
                    vec![left, right, tip],
                    cut_fill,
                    Stroke::NONE,
                ));
                painter.line(
                    vec![
                        pos2(left.x, rect.top()),
                        left,
                        tip,
                        right,
                        pos2(right.x, rect.top()),
                    ],
                    stroke,
                );
            }
            BitShape::Ball => {
                let n = 24;
                let full_radius = (settings.v_bit_dia / 2.0) as f32 * scale;
                let center_y = tip.y - full_radius;
                let mut pts = vec![pos2(cx - full_radius, rect.top())];
                for i in 0..=n {
                    let a = std::f32::consts::PI * i as f32 / n as f32;
                    pts.push(pos2(
                        cx - full_radius * a.cos(),
                        center_y + full_radius * a.sin(),
                    ));
                }
                pts.push(pos2(cx + full_radius, rect.top()));
                painter.line(pts, stroke);
            }
            BitShape::Flat => {
                painter.line(
                    vec![
                        pos2(cx - r, rect.top()),
                        pos2(cx - r, tip.y),
                        pos2(cx + r, tip.y),
                        pos2(cx + r, rect.top()),
                    ],
                    stroke,
                );
            }
        }
    }

    fn form_grid(&mut self, id: &str, add: impl FnOnce(&mut Ui)) {
        egui::Grid::new(id)
            .num_columns(2)
            .min_col_width(118.0)
            .spacing([8.0, 4.0])
            .show(self, add);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_outline_is_open_at_the_bottom_and_rounded_at_the_top() {
        let rect = egui::Rect::from_min_max(pos2(10.0, 20.0), pos2(90.0, 50.0));
        let points = TabShape::outline(rect);
        assert_eq!(points[0], rect.left_bottom());
        assert_eq!(points[points.len() - 1], rect.right_bottom());
        // Nothing crosses the bottom edge, so the tab stays open to its pane.
        assert!(
            points[1..points.len() - 1]
                .iter()
                .all(|p| p.y < rect.bottom())
        );
        assert!(points.iter().all(|p| rect.expand(0.01).contains(*p)));
        // The square corners themselves are cut off.
        assert!(points.iter().all(|p| (*p - rect.left_top()).length() > 1.0));
        assert!(
            points
                .iter()
                .all(|p| (*p - rect.right_top()).length() > 1.0)
        );
    }

    #[test]
    fn tab_outline_handles_tabs_narrower_than_its_corners() {
        let rect = egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(4.0, 2.0));
        assert!(
            TabShape::outline(rect)
                .iter()
                .all(|p| p.x.is_finite() && p.y.is_finite())
        );
    }
}
