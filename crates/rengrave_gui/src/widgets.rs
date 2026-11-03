// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! Small reusable pieces of the settings panel.

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
        let text = RichText::new(spec.label);
        let text = if issue.is_some() {
            text.color(ui.visuals().error_fg_color)
        } else {
            text
        };
        let label = ui.label(text).on_hover_text(spec.tip);
        let response = ui
            .add(
                DragValue::new(value)
                    .speed(spec.speed)
                    .suffix(spec.suffix.to_string())
                    .max_decimals(5),
            )
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
        ui.checkbox(value, label).on_hover_text(tip);
        ui.end_row();
    }
}

/// Composite controls for the settings panel.
pub trait UiExt {
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

impl UiExt for Ui {
    fn resettable_text(&mut self, name: &str, tip: &str, value: &mut String, default: &str) {
        let label = self
            .horizontal(|ui| {
                let label = ui.label(name).on_hover_text(tip);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let differs = value != default;
                    let reset = ui
                        .add_enabled(differs, egui::Button::new("Reset").small())
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
        self.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
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
