//! The settings panel: all job settings, grouped by the step of the workflow they belong to.

use crate::app::{RengraveApp, StatusLevel, Tab};
use crate::widgets::{Form, Number, bit_profile, origin_grid, segmented};
use egui::{CollapsingHeader, Grid, RichText, Ui};
use rengrave_core::{
    ArcFit, BitShape, BuiltinFont, CheckScope, CleanupBit, CutType, HeightCalc, InputType, Justify,
    Origin, Tracer, TurnPolicy,
};

const PERCENT: &str = " %";
const DEGREES: &str = "°";

fn form_grid(ui: &mut Ui, id: &str, add: impl FnOnce(&mut Ui)) {
    Grid::new(id)
        .num_columns(2)
        .min_col_width(118.0)
        .spacing([8.0, 4.0])
        .show(ui, add);
}

impl RengraveApp {
    /// Draws the panel: tabs, the active tab's settings, and the primary action.
    pub(crate) fn inspector(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Design, "Design")
                .on_hover_text("What to cut, and where");
            ui.selectable_value(&mut self.tab, Tab::Toolpath, "Toolpath")
                .on_hover_text("How to cut it");
            ui.selectable_value(&mut self.tab, Tab::Output, "Output")
                .on_hover_text("G-code format and exports");
        });
        ui.separator();

        // Reserve room for the fixed action area at the bottom.
        egui::Panel::bottom("inspector_actions")
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.actions(ui));
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| match self.tab {
                Tab::Design => self.design_tab(ui),
                Tab::Toolpath => self.toolpath_tab(ui),
                Tab::Output => self.output_tab(ui),
            });
    }

    fn actions(&mut self, ui: &mut Ui) {
        ui.add_space(6.0);
        if self.settings.cut_type == CutType::VCarve {
            ui.horizontal(|ui| {
                let live = ui
                    .checkbox(&mut self.live_carve, "Live V-carve")
                    .on_hover_text(
                        "Recalculate paths after every change. Turn off for very large designs.",
                    );
                if live.changed()
                    && self.live_carve
                    && self.carve_is_stale()
                    && self.issues.is_empty()
                {
                    self.recalculate(true);
                }
                let can_calc = self.issues.is_empty();
                if ui
                    .add_enabled(can_calc, egui::Button::new("Calculate"))
                    .on_hover_text("Reload the source file and calculate V-carve paths now (F5)")
                    .clicked()
                {
                    self.reload_and_recalculate();
                }
            });
        }
        let enabled = self.can_write_gcode();
        let save = egui::Button::new(
            RichText::new("Save G-code…")
                .strong()
                .color(ui.visuals().selection.stroke.color),
        )
        .fill(ui.visuals().selection.bg_fill)
        .min_size(egui::vec2(ui.available_width(), 30.0));
        if ui
            .add_enabled(enabled, save)
            .on_disabled_hover_text("Waiting for toolpaths, or settings need attention.")
            .clicked()
        {
            self.save_gcode();
        }
        ui.add_space(6.0);
    }

    fn design_tab(&mut self, ui: &mut Ui) {
        CollapsingHeader::new("Source")
            .default_open(true)
            .show(ui, |ui| {
                segmented(
                    ui,
                    &mut self.settings.input_type,
                    &[
                        (InputType::Text, "Text", "Engrave text in a font"),
                        (InputType::Image, "Image", "Engrave a DXF drawing or bitmap"),
                    ],
                );
                ui.add_space(4.0);
                match self.settings.input_type {
                    InputType::Text => self.text_source(ui),
                    InputType::Image => self.image_source(ui),
                }
            });
        CollapsingHeader::new("Size and spacing")
            .default_open(true)
            .show(ui, |ui| self.size_section(ui));
        CollapsingHeader::new("Position")
            .default_open(true)
            .show(ui, |ui| self.position_section(ui));
        if self.settings.input_type == InputType::Text {
            CollapsingHeader::new("Text on a circle")
                .default_open(self.settings.text_radius != 0.0)
                .show(ui, |ui| self.circle_section(ui));
        }
    }

    fn text_source(&mut self, ui: &mut Ui) {
        let label = ui.label("Text to engrave");
        ui.add(
            egui::TextEdit::multiline(&mut self.settings.text)
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        )
        .labelled_by(label.id);

        ui.horizontal(|ui| {
            let selected = self.font_label();
            let label = ui.label("Font");
            let response = egui::ComboBox::from_id_salt("font_combo")
                .selected_text(selected)
                .width(170.0)
                .show_ui(ui, |ui| self.font_menu(ui));
            response
                .response
                .labelled_by(label.id)
                .on_hover_text("The font for the text");
            if ui
                .button("Folder…")
                .on_hover_text("Choose a folder of font files")
                .clicked()
            {
                self.choose_font_dir();
            }
        });
        ui.checkbox(&mut self.settings.show_font_index, "Show all characters")
            .on_hover_text("Lay out every character in the font, to see what it contains");
        ui.checkbox(&mut self.settings.extended_chars, "Extended characters")
            .on_hover_text("Load characters beyond Latin-1. Fonts may become slower to load.");
    }

    fn font_menu(&mut self, ui: &mut Ui) {
        let filter = ui.add(
            egui::TextEdit::singleline(&mut self.font_filter)
                .hint_text("Filter")
                .desired_width(180.0),
        );
        filter
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Filter fonts"));
        let needle = self.font_filter.to_lowercase();
        let matches = |name: &str| needle.is_empty() || name.to_lowercase().contains(&needle);
        egui::ScrollArea::vertical()
            .max_height(260.0)
            .show(ui, |ui| {
                for font in BuiltinFont::ALL {
                    let name = font.name();
                    if matches(name) {
                        ui.selectable_value(
                            &mut self.settings.font_file,
                            name.to_string(),
                            format!("{name} (built in)"),
                        );
                    }
                }
                ui.separator();
                for path in self.font_files.clone() {
                    let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned())
                    else {
                        continue;
                    };
                    if !matches(&name) {
                        continue;
                    }
                    let response = ui.selectable_label(self.settings.font_file == name, &name);
                    if response.clicked() {
                        if Self::font_is_loadable(&path) {
                            self.settings.font_file = name;
                        } else {
                            self.notify(StatusLevel::Error, format!("Unable to load {name}"));
                        }
                    }
                }
                if self.font_files.is_empty() {
                    ui.weak(format!("No fonts in {}", self.settings.font_dir.display()));
                }
            });
    }

    fn image_source(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let name = self.settings.image_file.file_name().map_or_else(
                || "No file selected".to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            ui.label(name);
            if ui.button("Open…").clicked() {
                self.open_image_dialog();
            }
            if ui
                .button("Reload")
                .on_hover_text("Read the file again, after changing it elsewhere (F5)")
                .clicked()
            {
                self.reload_and_recalculate();
            }
        });
        ui.weak("DXF, PNG, BMP, PBM, JPEG, GIF, or TIFF. You can also drop a file on the window.");
        CollapsingHeader::new("Bitmap tracing").show(ui, |ui| self.tracing_section(ui));
    }

    fn tracing_section(&mut self, ui: &mut Ui) {
        segmented(
            ui,
            &mut self.settings.tracer,
            &[
                (Tracer::Native, "Built in", "Trace with the built-in tracer"),
                (
                    Tracer::Potrace,
                    "Potrace",
                    "Trace with the potrace program, which must be installed",
                ),
            ],
        );
        egui::ComboBox::from_label("Turn policy")
            .selected_text(self.settings.bmp_turn_policy.name())
            .show_ui(ui, |ui| {
                for policy in TurnPolicy::ALL {
                    ui.selectable_value(&mut self.settings.bmp_turn_policy, *policy, policy.name());
                }
            })
            .response
            .on_hover_text("How to resolve pixel corners touched by two diagonal design pixels");
        let issues = self.issues.clone();
        let mut form = Form {
            issues: &issues,
            length_unit: "",
        };
        form_grid(ui, "tracing_grid", |ui| {
            form.number(
                ui,
                Number {
                    label: "Speckle size",
                    field: "bmp_turd_size",
                    tip: "Suppress speckles of up to this many pixels",
                    suffix: " px",
                    speed: 0.1,
                },
                &mut self.settings.bmp_turd_size,
            );
            form.number(
                ui,
                Number {
                    label: "Corner smoothing",
                    field: "bmp_alpha_max",
                    tip: "0 keeps corners sharp; 1.33 smooths them",
                    suffix: "",
                    speed: 0.01,
                },
                &mut self.settings.bmp_alpha_max,
            );
            form.number(
                ui,
                Number {
                    label: "Curve tolerance",
                    field: "bmp_opt_tolerance",
                    tip: "Tolerance for curve optimization",
                    suffix: "",
                    speed: 0.01,
                },
                &mut self.settings.bmp_opt_tolerance,
            );
            form.check(
                ui,
                "Curve optimization",
                "Allow simplification to join more curve",
                &mut self.settings.bmp_long_curve,
            );
        });
    }

    fn size_section(&mut self, ui: &mut Ui) {
        let is_text = self.settings.input_type == InputType::Text;
        let height_unit: &str = if self.settings.use_image_size && !is_text {
            PERCENT
        } else {
            self.unit()
        };
        let unit = self.unit();
        let issues = self.issues.clone();
        let mut form = Form {
            issues: &issues,
            length_unit: unit,
        };
        let s = &mut self.settings;
        form_grid(ui, "size_grid", |ui| {
            form.number(
                ui,
                Number {
                    label: if is_text { "Height" } else { "Image height" },
                    field: "y_scale",
                    tip: "Height of a line of text, or of the whole image",
                    suffix: height_unit,
                    speed: 0.01,
                },
                &mut s.y_scale,
            );
            form.number(
                ui,
                Number {
                    label: "Width",
                    field: "x_scale",
                    tip: "Width, as a percentage of proportional",
                    suffix: PERCENT,
                    speed: 0.5,
                },
                &mut s.x_scale,
            );
            if !is_text {
                form.check(
                    ui,
                    "Height as % of file",
                    "Scale relative to the file's own size",
                    &mut s.use_image_size,
                );
            }
            if s.cut_type == CutType::Engrave {
                form.length(
                    ui,
                    "Line thickness",
                    "stroke_thickness",
                    "Width of engraved lines, normally the cutter diameter. Affects the preview, not the G-code.",
                    &mut s.stroke_thickness,
                );
            }
            if is_text {
                form.number(
                    ui,
                    Number {
                        label: "Character spacing",
                        field: "char_space",
                        tip: "Gap between characters, as a percentage of the widest character",
                        suffix: PERCENT,
                        speed: 0.5,
                    },
                    &mut s.char_space,
                );
                form.number(
                    ui,
                    Number {
                        label: "Word spacing",
                        field: "word_space",
                        tip: "Width of a space, as a percentage of the widest character",
                        suffix: PERCENT,
                        speed: 0.5,
                    },
                    &mut s.word_space,
                );
                form.number(
                    ui,
                    Number {
                        label: "Line spacing",
                        field: "line_space",
                        tip: "Distance between lines, as a multiple of the text height",
                        suffix: " ×",
                        speed: 0.01,
                    },
                    &mut s.line_space,
                );
                ui.label("Height from")
                    .on_hover_text("Which characters determine the height");
                segmented(
                    ui,
                    &mut s.height_calc,
                    &[
                        (
                            HeightCalc::MaxUsed,
                            "Used",
                            "The tallest character in the text",
                        ),
                        (
                            HeightCalc::MaxAll,
                            "All",
                            "The tallest character in the font, so height stays constant",
                        ),
                    ],
                );
                ui.end_row();
                ui.label("Justify")
                    .on_hover_text("Alignment of multiple lines");
                segmented(
                    ui,
                    &mut s.justify,
                    &[
                        (Justify::Left, "Left", "Align left edges"),
                        (Justify::Center, "Center", "Center each line"),
                        (Justify::Right, "Right", "Align right edges"),
                    ],
                );
                ui.end_row();
            }
        });
    }

    fn position_section(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            origin_grid(ui, &mut self.settings.origin);
            ui.vertical(|ui| {
                ui.label("Work origin")
                    .on_hover_text("Which point of the design is X0 Y0");
                ui.radio_value(&mut self.settings.origin, Origin::Default, "Design origin")
                    .on_hover_text("The start of the text baseline, or the origin of the file");
                if self.settings.input_type == InputType::Text {
                    ui.radio_value(
                        &mut self.settings.origin,
                        Origin::ArcCenter,
                        "Circle center",
                    )
                    .on_hover_text("The center of the circle that text is placed on");
                }
            });
        });
        ui.add_space(4.0);
        let unit = self.unit();
        let issues = self.issues.clone();
        let mut form = Form {
            issues: &issues,
            length_unit: unit,
        };
        let s = &mut self.settings;
        form_grid(ui, "position_grid", |ui| {
            form.length(
                ui,
                "X offset",
                "x_origin",
                "Added to all X coordinates",
                &mut s.x_origin,
            );
            form.length(
                ui,
                "Y offset",
                "y_origin",
                "Added to all Y coordinates",
                &mut s.y_origin,
            );
            form.number(
                ui,
                Number {
                    label: "Rotation",
                    field: "angle",
                    tip: "Rotation of the design",
                    suffix: DEGREES,
                    speed: 0.5,
                },
                &mut s.angle,
            );
            form.check(ui, "Mirror", "Mirror about a vertical line", &mut s.mirror);
            form.check(ui, "Flip", "Mirror about a horizontal line", &mut s.flip);
            let circle_applies = s.input_type == InputType::Text
                && s.text_radius != 0.0
                && s.cut_type == CutType::Engrave;
            form.check(
                ui,
                if circle_applies { "Add circle" } else { "Add box" },
                "Surround the design. When V-carving this cuts the background instead of the design.",
                &mut s.add_box,
            );
            if s.add_box {
                form.length(
                    ui,
                    "Gap",
                    "box_gap",
                    "Distance between the design and the box",
                    &mut s.box_gap,
                );
            }
        });
    }

    fn circle_section(&mut self, ui: &mut Ui) {
        let unit = self.unit();
        let issues = self.issues.clone();
        let mut form = Form {
            issues: &issues,
            length_unit: unit,
        };
        let s = &mut self.settings;
        form_grid(ui, "circle_grid", |ui| {
            form.length(
                ui,
                "Radius",
                "text_radius",
                "Radius of the circle; zero means straight text",
                &mut s.text_radius,
            );
            ui.label("Side")
                .on_hover_text("Whether characters stand outside or inside the circle");
            segmented(
                ui,
                &mut s.outer,
                &[
                    (true, "Outside", "Text stands outside the circle"),
                    (false, "Inside", "Text stands inside the circle"),
                ],
            );
            ui.end_row();
            ui.label("Placement")
                .on_hover_text("Whether text runs over the top or under the bottom of the circle");
            segmented(
                ui,
                &mut s.upper,
                &[
                    (true, "Top", "Text runs over the top"),
                    (false, "Bottom", "Text runs under the bottom"),
                ],
            );
            ui.end_row();
        });
    }

    fn unit(&self) -> &'static str {
        match self.settings.units {
            rengrave_core::Units::Inches => " in",
            rengrave_core::Units::Millimeters => " mm",
        }
    }

    fn toolpath_tab(&mut self, ui: &mut Ui) {
        CollapsingHeader::new("Cut")
            .default_open(true)
            .show(ui, |ui| {
                segmented(
                    ui,
                    &mut self.settings.cut_type,
                    &[
                        (
                            CutType::VCarve,
                            "V-carve",
                            "Vary depth so that a shaped bit fills the outline",
                        ),
                        (
                            CutType::Engrave,
                            "Engrave",
                            "Follow the outline at constant depth",
                        ),
                    ],
                );
            });
        CollapsingHeader::new("Feeds and heights")
            .default_open(true)
            .show(ui, |ui| self.machine_section(ui));
        if self.settings.cut_type == CutType::VCarve {
            CollapsingHeader::new("Bit")
                .default_open(true)
                .show(ui, |ui| self.bit_section(ui));
            CollapsingHeader::new("Multi-pass").show(ui, |ui| self.multipass_section(ui));
            CollapsingHeader::new("Inlay").show(ui, |ui| self.inlay_section(ui));
            CollapsingHeader::new("Advanced V-carve")
                .show(ui, |ui| self.advanced_carve_section(ui));
            CollapsingHeader::new("Cleanup").show(ui, |ui| self.cleanup_section(ui));
        }
    }

    fn machine_section(&mut self, ui: &mut Ui) {
        let unit = self.unit();
        let per_min = match self.settings.units {
            rengrave_core::Units::Inches => " in/min",
            rengrave_core::Units::Millimeters => " mm/min",
        };
        let speed = if unit.contains("mm") { 1.0 } else { 0.05 };
        let issues = self.issues.clone();
        let mut form = Form {
            issues: &issues,
            length_unit: unit,
        };
        let s = &mut self.settings;
        form_grid(ui, "machine_grid", |ui| {
            form.number(
                ui,
                Number {
                    label: "Feed rate",
                    field: "feed",
                    tip: "Cutting speed",
                    suffix: per_min,
                    speed,
                },
                &mut s.feed,
            );
            form.number(
                ui,
                Number {
                    label: "Plunge rate",
                    field: "plunge",
                    tip: "Speed of vertical moves. Zero uses the feed rate.",
                    suffix: per_min,
                    speed,
                },
                &mut s.plunge,
            );
            form.length(
                ui,
                "Safe Z",
                "z_safe",
                "Height for rapid moves",
                &mut s.z_safe,
            );
            if s.cut_type == CutType::Engrave {
                form.length(
                    ui,
                    "Engrave depth",
                    "z_cut",
                    "Z of the engraving cut",
                    &mut s.z_cut,
                );
            }
        });
    }

    fn bit_section(&mut self, ui: &mut Ui) {
        segmented(
            ui,
            &mut self.settings.bit_shape,
            &[
                (BitShape::VBit, "V-bit", "A pointed conical bit"),
                (BitShape::Ball, "Ball nose", "A ball-nose end mill"),
                (
                    BitShape::Flat,
                    "Straight",
                    "A flat end mill, which clears only where it fits",
                ),
            ],
        );
        bit_profile(ui, &self.settings);
        let unit = self.unit();
        let issues = self.issues.clone();
        let mut form = Form {
            issues: &issues,
            length_unit: unit,
        };
        let s = &mut self.settings;
        form_grid(ui, "bit_grid", |ui| {
            if s.bit_shape == BitShape::VBit {
                form.number(
                    ui,
                    Number {
                        label: "Angle",
                        field: "v_bit_angle",
                        tip: "Included angle of the bit",
                        suffix: DEGREES,
                        speed: 0.5,
                    },
                    &mut s.v_bit_angle,
                );
            }
            form.length(
                ui,
                "Diameter",
                "v_bit_dia",
                "Diameter of the bit",
                &mut s.v_bit_dia,
            );
            form.length(
                ui,
                "Depth limit",
                "v_depth_limit",
                "Deepest allowed cut. Zero means the bit's full depth.",
                &mut s.v_depth_limit,
            );
            let dp = s.units.coord_decimals();
            ui.label("Deepest cut")
                .on_hover_text("The deepest Z this bit will reach");
            ui.monospace(format!("{:.dp$}{unit}", s.max_cut_depth()));
            ui.end_row();
        });
    }

    fn multipass_section(&mut self, ui: &mut Ui) {
        let unit = self.unit();
        let issues = self.issues.clone();
        let mut form = Form {
            issues: &issues,
            length_unit: unit,
        };
        let s = &mut self.settings;
        form_grid(ui, "multipass_grid", |ui| {
            form.length(
                ui,
                "Finish stock",
                "v_rough_stock",
                "Material left for the finish pass. Zero disables multi-pass cutting.",
                &mut s.v_rough_stock,
            );
            let enabled = s.v_rough_stock > 0.0;
            let label = ui.add_enabled(enabled, egui::Label::new("Depth per pass"));
            ui.add_enabled(
                enabled,
                egui::DragValue::new(&mut s.v_max_cut)
                    .speed(0.002)
                    .suffix(unit.to_string())
                    .max_decimals(5),
            )
            .labelled_by(label.id)
            .on_hover_text("Deepest step of each roughing pass (negative)");
            ui.end_row();
        });
    }

    fn inlay_section(&mut self, ui: &mut Ui) {
        let unit = self.unit();
        let issues = self.issues.clone();
        let mut form = Form {
            issues: &issues,
            length_unit: unit,
        };
        let s = &mut self.settings;
        form_grid(ui, "inlay_grid", |ui| {
            form.check(
                ui,
                "Prismatic cut",
                "Cut a plug for inlay work. Also add a box on the Design tab.",
                &mut s.inlay,
            );
            form.length(
                ui,
                "Overcut",
                "allowance",
                "Extra depth for the prismatic cut (negative)",
                &mut s.allowance,
            );
            form.check(
                ui,
                "Cut outside",
                "Cut on the opposite side of outlines",
                &mut s.v_flop,
            );
        });
    }

    fn advanced_carve_section(&mut self, ui: &mut Ui) {
        let unit = self.unit();
        let issues = self.issues.clone();
        let mut form = Form {
            issues: &issues,
            length_unit: unit,
        };
        let s = &mut self.settings;
        form_grid(ui, "advanced_grid", |ui| {
            form.length(
                ui,
                "Step length",
                "v_step_len",
                "Distance between samples along outlines. Smaller is finer and slower.",
                &mut s.v_step_len,
            );
            form.number(
                ui,
                Number {
                    label: "Drive to corner",
                    field: "v_drive_corner",
                    tip: "Inside corners sharper than this cut all the way into the corner",
                    suffix: DEGREES,
                    speed: 0.5,
                },
                &mut s.v_drive_corner,
            );
            form.number(
                ui,
                Number {
                    label: "Step around corner",
                    field: "v_step_corner",
                    tip: "Outside corners beyond this gain extra samples around the corner",
                    suffix: DEGREES,
                    speed: 0.5,
                },
                &mut s.v_step_corner,
            );
            ui.label("Limit by")
                .on_hover_text("Which outlines limit the cut at each point");
            segmented(
                ui,
                &mut s.v_check_scope,
                &[
                    (CheckScope::All, "All", "The whole design"),
                    (
                        CheckScope::Char,
                        "Character",
                        "Only the current character, so characters may overlap",
                    ),
                ],
            );
            ui.end_row();
        });
    }

    fn cleanup_section(&mut self, ui: &mut Ui) {
        let unit = self.unit();
        ui.weak("Clears flat regions that the V-bit cannot reach.");
        let issues = self.issues.clone();
        let mut form = Form {
            issues: &issues,
            length_unit: unit,
        };
        let s = &mut self.settings;
        form_grid(ui, "cleanup_grid", |ui| {
            form.length(
                ui,
                "Straight bit",
                "clean_dia",
                "Diameter of the straight cleanup bit",
                &mut s.clean_dia,
            );
            form.number(
                ui,
                Number {
                    label: "Step over",
                    field: "clean_step",
                    tip: "Distance between passes, as a percentage of the bit diameter",
                    suffix: PERCENT,
                    speed: 0.5,
                },
                &mut s.clean_step,
            );
            form.length(
                ui,
                "V-bit step",
                "clean_v",
                "Width of each V-bit cleanup pass. Zero disables the operation.",
                &mut s.clean_v,
            );
        });
        Grid::new("cleanup_ops").num_columns(5).show(ui, |ui| {
            ui.label("");
            for (name, tip) in [
                ("Perimeter", "A pass around the area"),
                ("X", "Passes along X"),
                ("Y", "Passes along Y"),
                ("Loops", "Concentric inward loops"),
            ] {
                ui.label(name).on_hover_text(tip);
            }
            ui.end_row();
            ui.label("Straight");
            let flop = self.settings.v_flop;
            let ops = &mut self.settings.clean_straight;
            ui.checkbox(&mut ops.perimeter, "")
                .on_hover_text("Straight perimeter");
            ui.checkbox(&mut ops.x, "").on_hover_text("Straight X");
            ui.checkbox(&mut ops.y, "").on_hover_text("Straight Y");
            ui.add_enabled(!flop, egui::Checkbox::without_text(&mut ops.loops))
                .on_hover_text("Straight loops");
            ui.end_row();
            ui.label("V-bit");
            let ops = &mut self.settings.clean_v_bit;
            ui.checkbox(&mut ops.perimeter, "")
                .on_hover_text("V-bit perimeter");
            ui.checkbox(&mut ops.x, "").on_hover_text("V-bit X");
            ui.checkbox(&mut ops.y, "").on_hover_text("V-bit Y");
            ui.checkbox(&mut ops.loops, "").on_hover_text("V-bit loops");
            ui.end_row();
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.can_write_gcode(),
                    egui::Button::new("Calculate cleanup"),
                )
                .clicked()
            {
                self.calculate_cleanup();
            }
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.has_cleanup(CleanupBit::Straight),
                    egui::Button::new("Save straight…"),
                )
                .clicked()
            {
                self.save_cleanup(CleanupBit::Straight);
            }
            if ui
                .add_enabled(
                    self.has_cleanup(CleanupBit::VBit),
                    egui::Button::new("Save V-bit…"),
                )
                .clicked()
            {
                self.save_cleanup(CleanupBit::VBit);
            }
        });
    }

    fn output_tab(&mut self, ui: &mut Ui) {
        CollapsingHeader::new("G-code format")
            .default_open(true)
            .show(ui, |ui| {
                let unit = self.unit();
                let issues = self.issues.clone();
                let mut form = Form {
                    issues: &issues,
                    length_unit: unit,
                };
                let s = &mut self.settings;
                form_grid(ui, "format_grid", |ui| {
                    ui.label("Arcs").on_hover_text("How curves are written");
                    segmented(
                        ui,
                        &mut s.arc_fit,
                        &[
                            (
                                ArcFit::None,
                                "Lines",
                                "Lines only; works with every controller",
                            ),
                            (ArcFit::Center, "Center", "G2/G3 with I and J"),
                            (ArcFit::Radius, "Radius", "G2/G3 with R"),
                        ],
                    );
                    ui.end_row();
                    form.length(
                        ui,
                        "Accuracy",
                        "accuracy",
                        "Allowed deviation when simplifying paths",
                        &mut s.accuracy,
                    );
                    form.number(
                        ui,
                        Number {
                            label: "Arc angle",
                            field: "seg_arc",
                            tip: "Angular step for flattening curves in fonts and drawings",
                            suffix: DEGREES,
                            speed: 0.1,
                        },
                        &mut s.seg_arc,
                    );
                    let mut use_variables = !s.disable_variables;
                    form.check(
                        ui,
                        "Use #1 and #2 variables",
                        "Write Z values as variables, for editing at the controller",
                        &mut use_variables,
                    );
                    s.disable_variables = !use_variables;
                    form.check(
                        ui,
                        "Omit comments",
                        "Leave out comments, including recoverable settings",
                        &mut s.no_comments,
                    );
                });
                let label = ui
                    .label("Header")
                    .on_hover_text("Written before the first move. | starts a new line.");
                ui.add(egui::TextEdit::singleline(&mut s.g_pre).desired_width(f32::INFINITY))
                    .labelled_by(label.id);
                let label = ui
                    .label("Footer")
                    .on_hover_text("Written after the last move. | starts a new line.");
                ui.add(egui::TextEdit::singleline(&mut s.g_post).desired_width(f32::INFINITY))
                    .labelled_by(label.id);
            });
        CollapsingHeader::new("Export")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.button("SVG…").clicked() {
                        self.save_svg();
                    }
                    if ui.button("DXF…").clicked() {
                        self.save_dxf(false);
                    }
                    if ui.button("DXF, closed…").clicked() {
                        self.save_dxf(true);
                    }
                    if ui.button("Settings…").clicked() {
                        self.save_settings();
                    }
                });
            });
        if let Some(design) = self.design().cloned() {
            CollapsingHeader::new("Job summary")
                .default_open(true)
                .show(ui, |ui| {
                    let unit = self.unit();
                    let dp = self.settings.units.coord_decimals();
                    let b = design.bounds;
                    Grid::new("summary_grid").num_columns(2).show(ui, |ui| {
                        ui.label("Size");
                        ui.monospace(format!("{:.dp$} × {:.dp$}{unit}", b.width(), b.height()));
                        ui.end_row();
                        ui.label("X range");
                        ui.monospace(format!("{:.dp$} to {:.dp$}", b.min_x, b.max_x));
                        ui.end_row();
                        ui.label("Y range");
                        ui.monospace(format!("{:.dp$} to {:.dp$}", b.min_y, b.max_y));
                        ui.end_row();
                        if self.settings.cut_type == CutType::VCarve {
                            ui.label("Deepest cut");
                            ui.monospace(format!("{:.dp$}{unit}", self.settings.max_cut_depth()));
                            ui.end_row();
                        }
                        if self.can_write_gcode()
                            && let Ok(lines) = self.gcode()
                        {
                            ui.label("G-code lines");
                            ui.monospace(lines.len().to_string());
                            ui.end_row();
                        }
                    });
                });
        }
    }
}
