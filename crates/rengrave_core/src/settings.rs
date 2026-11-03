// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2011-2026 Scorch
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch
//
// Ported from the settings, validation, `calc_vbit_dia`, `calc_depth_limit`, `Scale_Linear_Inputs`, and
// `Open_G_Code_File` of F-Engrave 1.78.

//! The complete description of an engraving job, its validation, and derived tool values.
//!
//! [`Settings::to_settings_lines`] and [`Settings::apply_settings_text`] read and write the
//! `(fengrave_set NAME VALUE)` comment format, so settings files and G-code produced by
//! F-Engrave can be loaded and the other way around.

use crate::geom::round_significant;
use std::path::PathBuf;
use std::str::FromStr;

/// Declares a settings enum with its stable file-format names.
macro_rules! named_enum {
    ($(#[$m:meta])* $name:ident { $($(#[$vm:meta])* $var:ident => $text:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name { $($(#[$vm])* $var),+ }

        impl $name {
            /// All variants, in display order.
            pub const ALL: &'static [Self] = &[$(Self::$var),+];

            /// The name used in settings files and on the command line.
            pub const fn name(self) -> &'static str {
                match self { $(Self::$var => $text),+ }
            }
        }

        impl FromStr for $name {
            type Err = String;
            fn from_str(s: &str) -> Result<Self, String> {
                let s = s.trim();
                Self::ALL
                    .iter()
                    .copied()
                    .find(|v| v.name().eq_ignore_ascii_case(s))
                    .ok_or_else(|| {
                        let names: Vec<_> = Self::ALL.iter().map(|v| v.name()).collect();
                        format!("unknown value {s:?}; expected one of {}", names.join(", "))
                    })
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.name())
            }
        }
    };
}

named_enum!(
    /// Linear unit for every dimension in the job.
    Units {
        /// Inches, with feeds per minute (`G20`).
        Inches => "in",
        /// Millimeters, with feeds per minute (`G21`).
        Millimeters => "mm",
    }
);
named_enum!(
    /// How the design is cut.
    CutType {
        /// Follow the outline at constant depth.
        Engrave => "engrave",
        /// Vary the depth so a shaped bit fills the outline.
        VCarve => "v-carve",
    }
);
named_enum!(
    /// What the design is made from.
    InputType {
        /// Text in the selected font.
        Text => "text",
        /// A DXF drawing or a traced raster image.
        Image => "image",
    }
);
named_enum!(
    /// Tool profile used for V-carving.
    BitShape {
        /// A pointed conical bit.
        VBit => "VBIT",
        /// A ball-nose end mill.
        Ball => "BALL",
        /// A flat-bottomed end mill.
        Flat => "FLAT",
    }
);
named_enum!(
    /// Alignment of multiple lines of text.
    Justify {
        /// Align the left edges.
        Left => "Left",
        /// Center each line.
        Center => "Center",
        /// Align the right edges.
        Right => "Right",
    }
);
named_enum!(
    /// Which point of the design becomes the work origin.
    Origin {
        /// The design's own origin: the text baseline start, or the file origin of a drawing.
        Default => "Default",
        /// Top left of the bounding box.
        TopLeft => "Top-Left",
        /// Top center of the bounding box.
        TopCenter => "Top-Center",
        /// Top right of the bounding box.
        TopRight => "Top-Right",
        /// Middle left of the bounding box.
        MidLeft => "Mid-Left",
        /// Center of the bounding box.
        MidCenter => "Mid-Center",
        /// Middle right of the bounding box.
        MidRight => "Mid-Right",
        /// Bottom left of the bounding box.
        BotLeft => "Bot-Left",
        /// Bottom center of the bounding box.
        BotCenter => "Bot-Center",
        /// Bottom right of the bounding box.
        BotRight => "Bot-Right",
        /// The center of the circle that text is placed on.
        ArcCenter => "Arc-Center",
    }
);
named_enum!(
    /// Which characters determine the scale of a line of text.
    HeightCalc {
        /// Only the characters in the text.
        MaxUsed => "max_use",
        /// Every character in the font, so height does not change with the text.
        MaxAll => "max_all",
    }
);
named_enum!(
    /// Arc output in G-code.
    ArcFit {
        /// Lines only.
        None => "none",
        /// `G2`/`G3` with `I`/`J` center offsets.
        Center => "center",
        /// `G2`/`G3` with an `R` radius word.
        Radius => "radius",
    }
);
named_enum!(
    /// Which outlines limit the carve at each point.
    CheckScope {
        /// Only the current character, which allows overlap between characters.
        Char => "chr",
        /// The whole design.
        All => "all",
    }
);
named_enum!(
    /// How a bitmap tracer resolves pixel corners touched by two diagonal design pixels.
    TurnPolicy {
        /// Connect design pixels.
        Black => "black",
        /// Connect background pixels.
        White => "white",
        /// Always turn right, which keeps diagonal design pixels apart in the native tracer.
        Right => "right",
        /// Always turn left, which connects diagonal design pixels in the native tracer.
        Left => "left",
        /// Connect whichever color is less common in the image.
        Minority => "minority",
        /// Connect whichever color is more common in the image.
        Majority => "majority",
        /// Choose at random in `potrace`; the native tracer connects design pixels.
        Random => "random",
    }
);
named_enum!(
    /// Program that converts bitmaps to outlines.
    Tracer {
        /// The built-in tracer.
        Native => "native",
        /// The external `potrace` program, which must be on the `PATH`.
        Potrace => "potrace",
    }
);
named_enum!(
    /// Tool used for a cleanup operation.
    CleanupBit {
        /// A flat end mill, for clearing open areas.
        Straight => "straight",
        /// A small V-bit, for tidying the edges left by the straight bit.
        VBit => "v-bit",
    }
);

impl Units {
    /// Decimal places written for coordinates.
    pub const fn coord_decimals(self) -> usize {
        match self {
            Self::Inches => 4,
            Self::Millimeters => 3,
        }
    }

    /// Decimal places written for feed rates.
    pub const fn feed_decimals(self) -> usize {
        match self {
            Self::Inches => 2,
            Self::Millimeters => 1,
        }
    }

    /// The smallest usable V-carve step, which also bounds calculation time.
    pub const fn min_step_len(self) -> f64 {
        match self {
            Self::Inches => 0.0005,
            Self::Millimeters => 0.01,
        }
    }
}

/// Which cuts a cleanup operation includes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CleanupOps {
    /// Perimeter pass around the area to clear.
    pub perimeter: bool,
    /// Parallel passes along X.
    pub x: bool,
    /// Parallel passes along Y.
    pub y: bool,
    /// Concentric inward loops.
    pub loops: bool,
}

impl CleanupOps {
    /// Returns `true` if any cut is enabled.
    pub const fn any(self) -> bool {
        self.perimeter || self.x || self.y || self.loops
    }
}

impl Default for CleanupOps {
    fn default() -> Self {
        Self {
            perimeter: true,
            x: true,
            y: false,
            loops: false,
        }
    }
}

/// All inputs to a job. Lengths are in [`Settings::units`].
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    // ---- Input
    /// Whether the design is text or an image.
    pub input_type: InputType,
    /// The text to engrave; `\n` separates lines.
    pub text: String,
    /// Directory containing font files.
    pub font_dir: PathBuf,
    /// Font file name within `font_dir`, or a built-in font name. Empty uses the default.
    pub font_file: String,
    /// Image or DXF file for image input.
    pub image_file: PathBuf,
    /// Load every code point of a font instead of Latin-1 only.
    pub extended_chars: bool,
    /// Lay out every character of the font instead of `text`.
    pub show_font_index: bool,

    // ---- Size and spacing
    /// Height of a line of text, or of the image; a percentage when `use_image_size` is set.
    pub y_scale: f64,
    /// Width as a percentage of the proportional width.
    pub x_scale: f64,
    /// Interpret an image's `y_scale` as a percentage of its native size.
    pub use_image_size: bool,
    /// Line thickness of an engraved stroke; display only, unless a box is added.
    pub stroke_thickness: f64,
    /// Line spacing as a multiple of the text height.
    pub line_space: f64,
    /// Character spacing as a percentage of the widest character.
    pub char_space: f64,
    /// Word spacing as a percentage of the widest character.
    pub word_space: f64,
    /// How the scale is determined.
    pub height_calc: HeightCalc,

    // ---- Placement
    /// Rotation in degrees.
    pub angle: f64,
    /// Radius of the circle text is placed on; zero for straight text.
    pub text_radius: f64,
    /// Place text outside the circle rather than inside.
    pub outer: bool,
    /// Place text on the top of the circle rather than the bottom.
    pub upper: bool,
    /// Mirror about a horizontal line.
    pub flip: bool,
    /// Mirror about a vertical line.
    pub mirror: bool,
    /// Multi-line alignment.
    pub justify: Justify,
    /// Work origin location.
    pub origin: Origin,
    /// X offset added to the whole design.
    pub x_origin: f64,
    /// Y offset added to the whole design.
    pub y_origin: f64,
    /// Add a box (or a circle, for engraved circular text) around the design.
    pub add_box: bool,
    /// Gap between the design and the box.
    pub box_gap: f64,

    // ---- Machine
    /// Linear units.
    pub units: Units,
    /// Cut type.
    pub cut_type: CutType,
    /// Cutting feed rate, in units per minute.
    pub feed: f64,
    /// Plunge feed rate; zero uses `feed`.
    pub plunge: f64,
    /// Height for rapid moves.
    pub z_safe: f64,
    /// Depth of an engraving cut. Not used when V-carving.
    pub z_cut: f64,
    /// Arc output format.
    pub arc_fit: ArcFit,
    /// Maximum deviation allowed when simplifying output paths.
    pub accuracy: f64,
    /// Angular step for flattening arcs in fonts and drawings, in degrees.
    pub seg_arc: f64,
    /// Lines written before the first move; `|` separates lines.
    pub g_pre: String,
    /// Lines written after the last move; `|` separates lines.
    pub g_post: String,
    /// Write literal numbers instead of `#1`/`#2` parameters.
    pub disable_variables: bool,
    /// Omit comments, including the settings block, from G-code.
    pub no_comments: bool,

    // ---- V-carve
    /// Tool profile.
    pub bit_shape: BitShape,
    /// Included angle of a V-bit, in degrees.
    pub v_bit_angle: f64,
    /// Tool diameter.
    pub v_bit_dia: f64,
    /// Depth limit (negative); zero means the tool's full depth.
    pub v_depth_limit: f64,
    /// Corners sharper than this drive to the corner point, in degrees.
    pub v_drive_corner: f64,
    /// Corners more open than this gain sub-steps, in degrees.
    pub v_step_corner: f64,
    /// Distance between carve samples along the outline.
    pub v_step_len: f64,
    /// Depth of the final pass left by roughing; zero disables multi-pass cutting.
    pub v_rough_stock: f64,
    /// Depth per roughing pass (negative).
    pub v_max_cut: f64,
    /// Which outlines limit the carve.
    pub v_check_scope: CheckScope,
    /// Cut on the opposite side of the outlines.
    pub v_flop: bool,
    /// Cut a prismatic plug for inlay work.
    pub inlay: bool,
    /// Prismatic overcut (negative).
    pub allowance: f64,

    // ---- Cleanup
    /// Diameter of the straight cleanup bit.
    pub clean_dia: f64,
    /// Straight cleanup step-over, as a percentage of its diameter.
    pub clean_step: f64,
    /// Width of each V-bit cleanup pass.
    pub clean_v: f64,
    /// Cuts for the straight cleanup operation.
    pub clean_straight: CleanupOps,
    /// Cuts for the V-bit cleanup operation.
    pub clean_v_bit: CleanupOps,

    // ---- Image import
    /// Program that traces bitmaps.
    pub tracer: Tracer,
    /// Speckle suppression, in pixels.
    pub bmp_turd_size: f64,
    /// Corner smoothing: 0 keeps sharp corners, 1.33 smooths them.
    pub bmp_alpha_max: f64,
    /// Curve optimization tolerance.
    pub bmp_opt_tolerance: f64,
    /// Enable curve optimization.
    pub bmp_long_curve: bool,
    /// How ambiguous pixel corners are resolved.
    pub bmp_turn_policy: TurnPolicy,

    // ---- Preview preferences stored with the settings
    /// Show the origin axes.
    pub show_axis: bool,
    /// Show the bounding box.
    pub show_box: bool,
    /// Show engraved line thickness.
    pub show_thick: bool,
    /// Show the V-carve toolpath.
    pub show_v_path: bool,
    /// Show the V-carved area.
    pub show_v_area: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            input_type: InputType::Text,
            text: "F-Engrave".into(),
            font_dir: PathBuf::from("fonts"),
            font_file: String::new(),
            image_file: PathBuf::new(),
            extended_chars: false,
            show_font_index: false,
            y_scale: 2.0,
            x_scale: 100.0,
            use_image_size: false,
            stroke_thickness: 0.01,
            line_space: 1.1,
            char_space: 25.0,
            word_space: 100.0,
            height_calc: HeightCalc::MaxUsed,
            angle: 0.0,
            text_radius: 0.0,
            outer: true,
            upper: true,
            flip: false,
            mirror: false,
            justify: Justify::Left,
            origin: Origin::Default,
            x_origin: 0.0,
            y_origin: 0.0,
            add_box: false,
            box_gap: 0.25,
            units: Units::Inches,
            cut_type: CutType::Engrave,
            feed: 5.0,
            plunge: 0.0,
            z_safe: 0.25,
            z_cut: -0.005,
            arc_fit: ArcFit::None,
            accuracy: 0.001,
            seg_arc: 5.0,
            g_pre: "G17 M3 S3000".into(),
            g_post: "M5|M2".into(),
            disable_variables: true,
            no_comments: true,
            bit_shape: BitShape::VBit,
            v_bit_angle: 60.0,
            v_bit_dia: 0.5,
            v_depth_limit: 0.0,
            v_drive_corner: 135.0,
            v_step_corner: 200.0,
            v_step_len: 0.01,
            v_rough_stock: 0.0,
            v_max_cut: -1.0,
            v_check_scope: CheckScope::All,
            v_flop: false,
            inlay: false,
            allowance: 0.0,
            clean_dia: 0.25,
            clean_step: 50.0,
            clean_v: 0.05,
            clean_straight: CleanupOps::default(),
            clean_v_bit: CleanupOps::default(),
            tracer: Tracer::Native,
            bmp_turd_size: 2.0,
            bmp_alpha_max: 1.0,
            bmp_opt_tolerance: 0.2,
            bmp_long_curve: true,
            bmp_turn_policy: TurnPolicy::Minority,
            show_axis: true,
            show_box: true,
            show_thick: true,
            show_v_path: true,
            show_v_area: true,
        }
    }
}

/// A problem with one setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    /// The name of the [`Settings`] field.
    pub field: &'static str,
    /// What is wrong, in user-facing terms.
    pub message: &'static str,
}

impl Settings {
    /// Default settings in the requested units, with lengths at sensible metric values.
    pub fn with_units(units: Units) -> Self {
        let mut settings = Self::default();
        settings.set_units(units);
        settings
    }

    /// Returns every problem that would prevent a correct result.
    ///
    /// Only settings that affect the job as configured are checked: the box gap only when a box
    /// is added, tool settings only when V-carving, and so on.
    pub fn validate(&self) -> Vec<Issue> {
        let positive = |v: f64| v > 0.0 && v.is_finite();
        let non_negative = |v: f64| v >= 0.0 && v.is_finite();
        let non_positive = |v: f64| v <= 0.0 && v.is_finite();

        let mut issues = Vec::new();
        let mut check = |ok: bool, field: &'static str, message: &'static str| {
            if !ok {
                issues.push(Issue { field, message });
            }
        };
        let is_text = self.input_type == InputType::Text;
        let is_carve = self.cut_type == CutType::VCarve;

        check(
            positive(self.y_scale),
            "y_scale",
            "Height should be greater than 0",
        );
        check(
            positive(self.x_scale),
            "x_scale",
            "Width should be greater than 0",
        );
        check(
            self.angle > -360.0 && self.angle < 360.0,
            "angle",
            "Angle should be between -360 and 360",
        );
        check(
            self.x_origin.is_finite(),
            "x_origin",
            "X offset should be a number",
        );
        check(
            self.y_origin.is_finite(),
            "y_origin",
            "Y offset should be a number",
        );
        check(positive(self.feed), "feed", "Feed should be greater than 0");
        check(
            non_negative(self.plunge),
            "plunge",
            "Plunge rate should be greater than or equal to 0",
        );
        check(
            self.z_safe.is_finite(),
            "z_safe",
            "Safe Z should be a number",
        );
        check(
            self.z_cut.is_finite(),
            "z_cut",
            "Engrave depth should be a number",
        );
        check(
            positive(self.seg_arc),
            "seg_arc",
            "Arc angle should be greater than 0",
        );
        check(
            positive(self.accuracy),
            "accuracy",
            "Accuracy should be greater than 0",
        );
        if self.add_box {
            check(
                positive(self.box_gap),
                "box_gap",
                "Gap should be greater than 0",
            );
        }
        if is_text {
            check(
                non_negative(self.line_space),
                "line_space",
                "Line space should be greater than or equal to 0",
            );
            check(
                non_negative(self.char_space),
                "char_space",
                "Character space should be greater than or equal to 0",
            );
            check(
                non_negative(self.word_space),
                "word_space",
                "Word space should be greater than or equal to 0",
            );
            check(
                non_negative(self.text_radius),
                "text_radius",
                "Radius should be greater than or equal to 0",
            );
        } else {
            check(
                self.bmp_turd_size >= 1.0 && self.bmp_turd_size.is_finite(),
                "bmp_turd_size",
                "Speckle size should be greater than or equal to 1",
            );
            check(
                (0.0..=4.0 / 3.0).contains(&self.bmp_alpha_max),
                "bmp_alpha_max",
                "Corner smoothing should be between 0 and 1.333",
            );
            check(
                non_negative(self.bmp_opt_tolerance),
                "bmp_opt_tolerance",
                "Curve tolerance should be greater than or equal to 0",
            );
        }
        if !is_carve {
            check(
                non_negative(self.stroke_thickness),
                "stroke_thickness",
                "Thickness should be greater than or equal to 0",
            );
            return issues;
        }

        if self.bit_shape == BitShape::VBit {
            check(
                self.v_bit_angle > 0.0 && self.v_bit_angle < 180.0,
                "v_bit_angle",
                "Bit angle should be between 0 and 180",
            );
        }
        check(
            positive(self.v_bit_dia),
            "v_bit_dia",
            "Bit diameter should be greater than 0",
        );
        check(
            non_positive(self.v_depth_limit),
            "v_depth_limit",
            "Depth limit should be less than or equal to 0",
        );
        check(
            self.v_drive_corner > 0.0 && self.v_drive_corner < 180.0,
            "v_drive_corner",
            "Drive to corner should be between 0 and 180",
        );
        check(
            self.v_step_corner > 180.0 && self.v_step_corner < 360.0,
            "v_step_corner",
            "Step around corner should be between 180 and 360",
        );
        check(
            positive(self.v_step_len),
            "v_step_len",
            "Step length should be greater than 0",
        );
        check(
            non_positive(self.allowance),
            "allowance",
            "Overcut should be less than or equal to 0",
        );
        check(
            non_negative(self.v_rough_stock),
            "v_rough_stock",
            "Finish stock should be positive or zero",
        );
        if self.v_rough_stock > 0.0 {
            check(
                self.v_max_cut < 0.0 && self.v_max_cut.is_finite(),
                "v_max_cut",
                "Depth per pass should be less than 0",
            );
        }
        check(
            non_negative(self.clean_v),
            "clean_v",
            "V-bit step should be greater than or equal to 0",
        );
        check(
            positive(self.clean_dia),
            "clean_dia",
            "Straight bit should be greater than 0",
        );
        check(
            self.clean_step > 0.0 && self.clean_step <= 100.0,
            "clean_step",
            "Step over should be between 0% and 100%",
        );
        issues
    }

    /// Fails with [`crate::Error::InvalidSettings`] if [`Settings::validate`] finds problems.
    pub fn ensure_valid(&self) -> crate::Result<()> {
        let issues = self.validate();
        if issues.is_empty() {
            Ok(())
        } else {
            Err(crate::Error::InvalidSettings(
                issues
                    .iter()
                    .map(|i| format!("{}: {}", i.field, i.message))
                    .collect(),
            ))
        }
    }

    /// Changes units, converting every length (to three significant digits).
    pub fn set_units(&mut self, units: Units) {
        if units == self.units {
            return;
        }
        let factor = match units {
            Units::Millimeters => 25.4,
            Units::Inches => 1.0 / 25.4,
        };
        for value in [
            &mut self.y_scale,
            &mut self.text_radius,
            &mut self.z_safe,
            &mut self.z_cut,
            &mut self.stroke_thickness,
            &mut self.feed,
            &mut self.plunge,
            &mut self.box_gap,
            &mut self.v_bit_dia,
            &mut self.v_depth_limit,
            &mut self.v_step_len,
            &mut self.allowance,
            &mut self.v_max_cut,
            &mut self.v_rough_stock,
            &mut self.x_origin,
            &mut self.y_origin,
            &mut self.accuracy,
            &mut self.clean_v,
            &mut self.clean_dia,
        ] {
            *value = round_significant(*value * factor, 3);
        }
        self.units = units;
    }

    /// Half the bit's included angle, in radians.
    fn half_angle(&self) -> f64 {
        (self.v_bit_angle / 2.0).to_radians()
    }

    /// Diameter of the cone actually cut, given the depth limit and inlay mode.
    pub fn effective_bit_dia(&self) -> f64 {
        let half_angle = self.half_angle();
        if self.inlay && self.bit_shape == BitShape::VBit {
            return (-2.0 * self.allowance * half_angle.tan()).max(0.001);
        }
        if self.v_depth_limit < 0.0 {
            match self.bit_shape {
                BitShape::VBit => return -2.0 * self.v_depth_limit * half_angle.tan(),
                BitShape::Ball => {
                    let r = self.v_bit_dia / 2.0;
                    if self.v_depth_limit > -r {
                        return 2.0 * (r.powi(2) - (r + self.v_depth_limit).powi(2)).sqrt();
                    }
                }
                BitShape::Flat => {}
            }
        }
        self.v_bit_dia
    }

    /// Deepest Z the bit reaches, rounded to three decimals.
    pub fn max_cut_depth(&self) -> f64 {
        let bit_depth = match self.bit_shape {
            BitShape::VBit => -self.v_bit_dia / 2.0 / self.half_angle().tan(),
            BitShape::Ball | BitShape::Flat => -self.v_bit_dia / 2.0,
        };
        let limit = self.v_depth_limit;
        let depth = if limit < 0.0 {
            if self.bit_shape == BitShape::Flat {
                limit
            } else {
                bit_depth.max(limit)
            }
        } else {
            bit_depth
        };
        format!("{depth:.3}").parse().unwrap_or(depth)
    }

    /// The direction the carve runs, after the transformations that reverse it.
    pub(crate) fn effective_flop(&self, for_cleanup: bool) -> bool {
        let mut flop = self.v_flop;
        if self.input_type == InputType::Text && !for_cleanup {
            flop ^= self.add_box;
            flop ^= self.mirror;
            flop ^= self.flip;
        }
        flop
    }

    /// Finds anything in the header and footer that FluidNC would reject.
    ///
    /// FluidNC refuses a whole line containing an unsupported code, so anything else on that line,
    /// such as a spindle start, would not run. Each entry describes one problem. Lines using
    /// expressions or named parameters are not checked.
    pub fn controller_issues(&self) -> Vec<String> {
        let mut issues = Vec::new();
        for (name, text) in [("Header", &self.g_pre), ("Footer", &self.g_post)] {
            for line in text.split('|') {
                for word in Self::code_words(line) {
                    if !Self::fluidnc_accepts(&word) {
                        issues.push(format!(
                            "{name} line \"{}\" uses {word}, which FluidNC rejects along with \
                             everything else on that line",
                            line.trim()
                        ));
                    }
                }
            }
        }
        issues
    }

    /// Returns `true` if [`Settings::remove_unsupported_codes`] would change anything.
    pub fn has_removable_codes(&self) -> bool {
        [&self.g_pre, &self.g_post]
            .iter()
            .flat_map(|text| text.split('|'))
            .flat_map(Self::code_words)
            .any(|word| Self::is_removable(&word))
    }

    /// Removes path blending modes (`G64`, `G61.1`) from the header and footer, which FluidNC
    /// rejects and which only affect how smoothly corners are taken.
    ///
    /// Other rejected codes stay, because removing one could change what the rest of its line
    /// does: without `G81`, the line `G81 X1 Z-1` would be a move.
    pub fn remove_unsupported_codes(&mut self) {
        self.g_pre = Self::without_removable_words(&self.g_pre);
        self.g_post = Self::without_removable_words(&self.g_post);
    }

    /// Splits a line into its letter-and-value words, without comments or spaces, in upper case.
    ///
    /// Returns nothing for lines this cannot read reliably: expressions, named parameters, and
    /// `$` system commands.
    fn code_words(line: &str) -> Vec<String> {
        let mut code = String::with_capacity(line.len());
        let mut in_comment = false;
        for ch in line.chars() {
            match ch {
                ';' if !in_comment => break,
                '(' => in_comment = true,
                ')' => in_comment = false,
                c if !in_comment && !c.is_whitespace() => code.push(c.to_ascii_uppercase()),
                _ => {}
            }
        }
        if code.starts_with('$') || code.contains(['[', '<']) {
            return Vec::new();
        }
        let mut words: Vec<String> = Vec::new();
        for ch in code.chars() {
            match words.last_mut() {
                Some(word) if !ch.is_ascii_alphabetic() => word.push(ch),
                _ if ch.is_ascii_alphabetic() => words.push(ch.to_string()),
                // Text before the first letter, such as a `#1=2` assignment, is not a word.
                _ => {}
            }
        }
        words
    }

    /// Whether FluidNC accepts a word from [`Settings::code_words`]. Only `G` and `M` codes, and
    /// words with no value at all, can be judged without knowing the rest of the line.
    fn fluidnc_accepts(word: &str) -> bool {
        // Tenths of a code number: G91.1 is 911.
        const G_CODES: [i32; 41] = [
            0, 10, 20, 30, 40, 100, 170, 180, 190, 200, 210, 280, 281, 300, 301, 382, 383, 384,
            385, 400, 431, 490, 530, 540, 550, 560, 570, 580, 590, 591, 592, 593, 610, 800, 900,
            910, 911, 920, 921, 930, 940,
        ];
        const M_CODES: [i32; 24] = [
            0, 10, 20, 30, 40, 50, 60, 70, 80, 90, 300, 560, 610, 620, 630, 640, 650, 660, 670,
            680, 700, 710, 720, 730,
        ];
        let mut chars = word.chars();
        let letter = chars.next().unwrap_or_default();
        let value = chars.as_str();
        if value.is_empty() {
            return false;
        }
        // A variable's value is only known when the program runs.
        let Ok(number) = value.parse::<f64>() else {
            return true;
        };
        let code = (number * 10.0).round() as i32;
        match letter {
            'G' => G_CODES.contains(&code),
            'M' => M_CODES.contains(&code),
            _ => true,
        }
    }

    fn is_removable(word: &str) -> bool {
        word.strip_prefix('G')
            .and_then(|value| value.parse::<f64>().ok())
            .is_some_and(|n| n == 64.0 || n == 61.1)
    }

    /// Drops blending modes, along with a `P` tolerance that follows one.
    fn without_removable_words(text: &str) -> String {
        let lines: Vec<String> = text
            .split('|')
            .map(|line| {
                if Self::code_words(line)
                    .iter()
                    .all(|w| !Self::is_removable(w))
                {
                    return line.to_string();
                }
                let mut kept: Vec<String> = Vec::new();
                let mut dropped_previous = false;
                for word in Self::code_words(line) {
                    if Self::is_removable(&word) {
                        dropped_previous = true;
                    } else {
                        if !(dropped_previous && word.starts_with('P')) {
                            kept.push(word);
                        }
                        dropped_previous = false;
                    }
                }
                kept.join(" ")
            })
            .collect();
        lines.join("|")
    }

    /// Renders the settings as `(fengrave_set NAME VALUE )` comment lines.
    pub fn to_settings_lines(&self) -> Vec<String> {
        let flag = u8::from;
        let mut out: Vec<String> = Vec::with_capacity(80);
        let mut set = |name: &str, value: String| {
            out.push(format!("(fengrave_set {name:<11} {value} )"));
        };
        set("show_axis", flag(self.show_axis).to_string());
        set("show_box", flag(self.show_box).to_string());
        set("show_thick", flag(self.show_thick).to_string());
        set("flip", flag(self.flip).to_string());
        set("mirror", flag(self.mirror).to_string());
        set("outer", flag(self.outer).to_string());
        set("upper", flag(self.upper).to_string());
        set("v_flop", flag(self.v_flop).to_string());
        set("inlay", flag(self.inlay).to_string());
        set("bmp_long", flag(self.bmp_long_curve).to_string());
        set("var_dis", flag(self.disable_variables).to_string());
        set("ext_char", flag(self.extended_chars).to_string());
        set("useIMGsize", flag(self.use_image_size).to_string());
        set("no_comments", flag(self.no_comments).to_string());
        set("plotbox", flag(self.add_box).to_string());
        set("show_v_path", flag(self.show_v_path).to_string());
        set("show_v_area", flag(self.show_v_area).to_string());
        set("arc_fit", self.arc_fit.to_string());
        set("YSCALE", self.y_scale.to_string());
        set("XSCALE", self.x_scale.to_string());
        set("LSPACE", self.line_space.to_string());
        set("CSPACE", self.char_space.to_string());
        set("WSPACE", self.word_space.to_string());
        set("TANGLE", self.angle.to_string());
        set("TRADIUS", self.text_radius.to_string());
        set("ZSAFE", self.z_safe.to_string());
        set("ZCUT", self.z_cut.to_string());
        set("STHICK", self.stroke_thickness.to_string());
        set("origin", self.origin.to_string());
        set("justify", self.justify.to_string());
        set("units", self.units.to_string());
        set("xorigin", self.x_origin.to_string());
        set("yorigin", self.y_origin.to_string());
        set("segarc", self.seg_arc.to_string());
        set("accuracy", self.accuracy.to_string());
        set("FEED", self.feed.to_string());
        set("PLUNGE", self.plunge.to_string());
        set("fontfile", format!("\"{}\"", self.font_file));
        set("H_CALC", self.height_calc.to_string());
        set("boxgap", self.box_gap.to_string());
        set("cut_type", self.cut_type.to_string());
        set("bit_shape", self.bit_shape.to_string());
        set("v_bit_angle", self.v_bit_angle.to_string());
        set("v_bit_dia", self.v_bit_dia.to_string());
        set("v_drv_crner", self.v_drive_corner.to_string());
        set("v_stp_crner", self.v_step_corner.to_string());
        set("v_step_len", self.v_step_len.to_string());
        set("allowance", self.allowance.to_string());
        set("v_max_cut", self.v_max_cut.to_string());
        set("v_rough_stk", self.v_rough_stock.to_string());
        set("v_depth_lim", self.v_depth_limit.to_string());
        set("v_check_all", self.v_check_scope.to_string());
        set("bmp_turnp", self.bmp_turn_policy.to_string());
        set("bmp_turds", self.bmp_turd_size.to_string());
        set("bmp_alpha", self.bmp_alpha_max.to_string());
        set("bmp_optto", self.bmp_opt_tolerance.to_string());
        set("fontdir", format!("\"{}\"", self.font_dir.display()));
        set("gpre", self.g_pre.clone());
        set("gpost", self.g_post.clone());
        set("imagefile", format!("\"{}\"", self.image_file.display()));
        set("input_type", self.input_type.to_string());
        set("clean_dia", self.clean_dia.to_string());
        set("clean_step", self.clean_step.to_string());
        set("clean_v", self.clean_v.to_string());
        let (s, v) = (self.clean_straight, self.clean_v_bit);
        set(
            "clean_paths",
            format!(
                "{},{},{},{},{},{},{},{}",
                flag(s.perimeter),
                flag(s.x),
                flag(s.y),
                flag(v.perimeter),
                flag(v.y),
                flag(v.x),
                flag(s.loops),
                flag(v.loops)
            ),
        );
        set("tracer", self.tracer.to_string());
        let codes: Vec<u32> = self.text.chars().map(|c| c as u32).collect();
        if codes.is_empty() {
            out.push("(fengrave_set TCODE )".to_string());
        }
        for chunk in codes.chunks(11) {
            let joined: Vec<String> = chunk.iter().map(|c| format!("{c:03}")).collect();
            out.push(format!("(fengrave_set TCODE {} )", joined.join(" ")));
        }
        out
    }

    /// Applies every `fengrave_set` line found in `text`, which may be a settings file or
    /// G-code carrying a settings block. Unknown names and unparsable values are ignored,
    /// and each is reported in the returned list.
    pub fn apply_settings_text(&mut self, text: &str) -> Vec<String> {
        let mut problems = Vec::new();
        let mut codes: Vec<u32> = Vec::new();
        let mut saw_codes = false;
        let mut box_size: Option<f64> = None;
        for line in text.lines() {
            let Some(idx) = line.find("fengrave_set") else {
                continue;
            };
            let rest = line[idx + "fengrave_set".len()..].trim();
            let rest = rest.strip_suffix(')').unwrap_or(rest).trim();
            let (name, value) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            let value = value.trim();
            if name == "TCODE" {
                saw_codes = true;
                codes.extend(
                    value
                        .split_whitespace()
                        .filter_map(|t| t.parse::<u32>().ok()),
                );
                continue;
            }
            if name == "boxsize" {
                box_size = value.parse().ok();
                continue;
            }
            if let Err(e) = self.apply_setting(name, value) {
                problems.push(format!("{name}: {e}"));
            }
        }
        if saw_codes {
            self.text = codes.iter().filter_map(|&c| char::from_u32(c)).collect();
        }
        if let Some(size) = box_size.filter(|&s| s != 0.0) {
            self.box_gap = size * self.stroke_thickness;
        }
        problems
    }

    /// Sets one value by its settings-file name.
    pub fn apply_setting(&mut self, name: &str, value: &str) -> Result<(), String> {
        fn num(value: &str) -> Result<f64, String> {
            value
                .parse()
                .map_err(|_| format!("not a number: {value:?}"))
        }
        fn flag(value: &str) -> Result<bool, String> {
            match value {
                "1" | "true" | "True" => Ok(true),
                "0" | "false" | "False" => Ok(false),
                other => Err(format!("not a flag: {other:?}")),
            }
        }
        fn quoted(value: &str) -> &str {
            value.trim().trim_matches('"')
        }
        match name {
            "show_axis" => self.show_axis = flag(value)?,
            "show_box" => self.show_box = flag(value)?,
            "show_thick" => self.show_thick = flag(value)?,
            "show_v_path" => self.show_v_path = flag(value)?,
            "show_v_area" => self.show_v_area = flag(value)?,
            "flip" => self.flip = flag(value)?,
            "mirror" => self.mirror = flag(value)?,
            "outer" => self.outer = flag(value)?,
            "upper" => self.upper = flag(value)?,
            "v_flop" => self.v_flop = flag(value)?,
            "inlay" => self.inlay = flag(value)?,
            "bmp_long" => self.bmp_long_curve = flag(value)?,
            "var_dis" => self.disable_variables = flag(value)?,
            "ext_char" => self.extended_chars = flag(value)?,
            "useIMGsize" => self.use_image_size = flag(value)?,
            "no_comments" => self.no_comments = flag(value)?,
            "plotbox" => {
                self.add_box = match value {
                    "box" => true,
                    "no_box" => false,
                    other => flag(other)?,
                }
            }
            "arc_fit" => {
                self.arc_fit = match value {
                    "0" => ArcFit::None,
                    "1" => ArcFit::Center,
                    other => other.parse().unwrap_or(ArcFit::Center),
                }
            }
            "YSCALE" => self.y_scale = num(value)?,
            "XSCALE" => self.x_scale = num(value)?,
            "LSPACE" => self.line_space = num(value)?,
            "CSPACE" => self.char_space = num(value)?,
            "WSPACE" => self.word_space = num(value)?,
            "TANGLE" => self.angle = num(value)?,
            "TRADIUS" => self.text_radius = num(value)?,
            "ZSAFE" => self.z_safe = num(value)?,
            "ZCUT" => self.z_cut = num(value)?,
            "STHICK" => self.stroke_thickness = num(value)?,
            "origin" => self.origin = value.parse()?,
            "justify" => self.justify = value.parse()?,
            "units" => self.units = value.parse()?,
            "xorigin" => self.x_origin = num(value)?,
            "yorigin" => self.y_origin = num(value)?,
            "segarc" => self.seg_arc = num(value)?,
            "accuracy" => self.accuracy = num(value)?,
            "FEED" => self.feed = num(value)?,
            "PLUNGE" => self.plunge = num(value)?,
            "fontfile" => self.font_file = quoted(value).to_string(),
            "H_CALC" => self.height_calc = value.parse()?,
            "boxgap" => self.box_gap = num(value)?,
            "cut_type" => self.cut_type = value.parse()?,
            "bit_shape" => self.bit_shape = value.parse()?,
            "v_bit_angle" => self.v_bit_angle = num(value)?,
            "v_bit_dia" => self.v_bit_dia = num(value)?,
            "v_drv_crner" => self.v_drive_corner = num(value)?,
            "v_stp_crner" => self.v_step_corner = num(value)?,
            "v_step_len" => self.v_step_len = num(value)?,
            "allowance" => self.allowance = num(value)?,
            "v_max_cut" => self.v_max_cut = num(value)?,
            "v_rough_stk" => self.v_rough_stock = num(value)?,
            "v_depth_lim" => self.v_depth_limit = num(value)?,
            "v_check_all" => self.v_check_scope = value.parse()?,
            "bmp_turnp" => self.bmp_turn_policy = value.parse()?,
            "bmp_turds" => self.bmp_turd_size = num(value)?,
            "bmp_alpha" => self.bmp_alpha_max = num(value)?,
            "bmp_optto" => self.bmp_opt_tolerance = num(value)?,
            "fontdir" => self.font_dir = PathBuf::from(quoted(value)),
            "gpre" => self.g_pre = value.to_string(),
            "gpost" => self.g_post = value.to_string(),
            "imagefile" => self.image_file = PathBuf::from(quoted(value)),
            "input_type" => self.input_type = value.parse()?,
            "clean_dia" => self.clean_dia = num(value)?,
            "clean_step" => self.clean_step = num(value)?,
            "clean_v" => self.clean_v = num(value)?,
            "clean_paths" => {
                let bits: Vec<bool> = value
                    .split(',')
                    .map(|t| num(t.trim()).map(|v| v != 0.0))
                    .collect::<Result<_, _>>()?;
                if let [p, x, y, vp, vy, vx, rest @ ..] = bits.as_slice() {
                    self.clean_straight.perimeter = *p;
                    self.clean_straight.x = *x;
                    self.clean_straight.y = *y;
                    self.clean_v_bit.perimeter = *vp;
                    self.clean_v_bit.y = *vy;
                    self.clean_v_bit.x = *vx;
                    if let [l, vl, ..] = rest {
                        self.clean_straight.loops = *l;
                        self.clean_v_bit.loops = *vl;
                    }
                }
            }
            "tracer" => self.tracer = value.parse()?,
            // Stored by F-Engrave but with no meaning here.
            "v_pplot" | "NGC_DIR" => {}
            other => return Err(format!("unknown setting {other:?}")),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_abs_diff_eq;

    #[test]
    fn defaults_are_valid() {
        assert!(Settings::default().validate().is_empty());
        assert!(
            Settings::with_units(Units::Millimeters)
                .validate()
                .is_empty()
        );
    }

    #[test]
    fn validation_reports_each_bad_field() {
        let settings = Settings {
            cut_type: CutType::VCarve,
            y_scale: 0.0,
            feed: -1.0,
            v_step_corner: 90.0,
            clean_step: 150.0,
            ..Settings::default()
        };
        let issues = settings.validate();
        let fields: Vec<_> = issues.iter().map(|i| i.field).collect();
        assert_eq!(fields, ["y_scale", "feed", "v_step_corner", "clean_step"]);
        assert!(settings.ensure_valid().is_err());
    }

    #[test]
    fn non_finite_values_are_rejected() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let cases = [
                Settings {
                    y_scale: bad,
                    ..Settings::default()
                },
                Settings {
                    z_safe: bad,
                    ..Settings::default()
                },
                Settings {
                    z_cut: bad,
                    ..Settings::default()
                },
                Settings {
                    x_origin: bad,
                    ..Settings::default()
                },
                Settings {
                    feed: bad,
                    ..Settings::default()
                },
                Settings {
                    cut_type: CutType::VCarve,
                    v_bit_dia: bad,
                    ..Settings::default()
                },
                Settings {
                    cut_type: CutType::VCarve,
                    v_step_len: bad,
                    ..Settings::default()
                },
            ];
            for case in cases {
                assert!(!case.validate().is_empty(), "{bad} should be rejected");
            }
        }
    }

    #[test]
    fn v_bit_needs_a_real_angle() {
        for angle in [0.0, 180.0] {
            let v_bit = Settings {
                cut_type: CutType::VCarve,
                v_bit_angle: angle,
                ..Settings::default()
            };
            assert_eq!(v_bit.validate()[0].field, "v_bit_angle");
            let ball = Settings {
                bit_shape: BitShape::Ball,
                ..v_bit
            };
            assert!(
                ball.validate().is_empty(),
                "the angle is unused by a ball bit"
            );
        }
    }

    #[test]
    fn settings_that_do_not_apply_are_not_checked() {
        let engrave = Settings {
            box_gap: 0.0,
            clean_dia: 0.0,
            clean_step: 0.0,
            v_bit_dia: 0.0,
            v_max_cut: 1.0,
            bmp_turd_size: 0.0,
            ..Settings::default()
        };
        assert!(engrave.validate().is_empty());

        let boxed = Settings {
            add_box: true,
            ..engrave.clone()
        };
        assert_eq!(boxed.validate()[0].field, "box_gap");

        let carve = Settings {
            cut_type: CutType::VCarve,
            ..engrave.clone()
        };
        let fields: Vec<_> = carve.validate().iter().map(|i| i.field).collect();
        assert!(fields.contains(&"v_bit_dia") && fields.contains(&"clean_dia"));
        assert!(
            !fields.contains(&"v_max_cut"),
            "unused without finish stock"
        );

        let image = Settings {
            input_type: InputType::Image,
            ..engrave
        };
        assert_eq!(image.validate()[0].field, "bmp_turd_size");
    }

    #[test]
    fn defaults_suit_fluidnc() {
        assert!(Settings::default().controller_issues().is_empty());
        let old = Settings {
            g_pre: "G17 G64 P0.001 M3 S3000".into(),
            ..Settings::default()
        };
        let issues = old.controller_issues();
        assert_eq!(issues.len(), 1);
        assert!(
            issues[0].contains("G64") && issues[0].contains("FluidNC"),
            "{}",
            issues[0]
        );
    }

    #[test]
    fn unsupported_codes_are_recognized_in_any_form() {
        for header in ["g64", "G64.0", "G17|G64 P0.1", "G61.1 M3"] {
            let s = Settings {
                g_pre: header.into(),
                g_post: String::new(),
                ..Settings::default()
            };
            assert_eq!(s.controller_issues().len(), 1, "{header}");
        }
        for fine in ["G17 M3 S3000", "G61", "M64 P1", ""] {
            let s = Settings {
                g_pre: fine.into(),
                g_post: String::new(),
                ..Settings::default()
            };
            assert!(s.controller_issues().is_empty(), "{fine}");
        }
        let footer = Settings {
            g_post: "G64|M2".into(),
            g_pre: String::new(),
            ..Settings::default()
        };
        assert!(footer.controller_issues()[0].starts_with("Footer"));
    }

    #[test]
    fn everything_fluidnc_accepts_passes() {
        let accepted = "G0 G1 G2 G3 G4 G10 L2 P1 G17 G18 G19 G20 G21 G28 G28.1 G30 G30.1 G38.2 G38.3 \
                        G38.4 G38.5 G40 G43.1 Z1 G49 G53 G54 G55 G56 G57 G58 G59 G59.1 G59.2 G59.3 \
                        G61 G80 G90 G91 G91.1 G92 G92.1 G93 G94 M0 M1 M2 M3 M4 M5 M6 T1 M7 M8 M9 M30 \
                        M56 M61 M62 P1 M63 M64 M65 M66 M67 M68 M70 M71 M72 M73 G00 G01 M03 F100 S500";
        for word in accepted.split_whitespace() {
            let s = Settings {
                g_pre: word.into(),
                g_post: String::new(),
                ..Settings::default()
            };
            assert!(
                s.controller_issues().is_empty(),
                "{word}: {:?}",
                s.controller_issues()
            );
        }
    }

    #[test]
    fn everything_fluidnc_rejects_is_flagged() {
        for word in [
            "G5", "G33", "G38.1", "G38.6", "G41", "G42", "G43", "G59.4", "G61.1", "G64", "G73",
            "G76", "G81", "G83", "G89", "G90.1", "G95", "G96", "G98", "M10", "M31", "M100", "M3.5",
        ] {
            let s = Settings {
                g_pre: word.into(),
                g_post: String::new(),
                ..Settings::default()
            };
            let issues = s.controller_issues();
            assert_eq!(issues.len(), 1, "{word}");
            assert!(issues[0].contains(word), "{}", issues[0]);
        }
    }

    #[test]
    fn check_reads_lines_as_fluidnc_does() {
        let issues = |header: &str| {
            let s = Settings {
                g_pre: header.into(),
                g_post: String::new(),
                ..Settings::default()
            };
            s.controller_issues()
        };
        // Words may touch, in either case, and comments do not count.
        assert_eq!(issues("g17g81x1").len(), 1);
        assert!(issues("G17 (G81) M3").is_empty());
        assert!(issues("G17 ; G81").is_empty());
        // A letter with no value is not a word.
        assert_eq!(issues("hello").len(), 5);
        // What cannot be judged here is left alone.
        assert!(issues("#1=2").is_empty());
        assert!(issues("G1 X#1").is_empty());
        assert!(issues("G1 X[1+2]").is_empty());
        assert!(issues("$X").is_empty());
        assert!(issues("").is_empty());
    }

    #[test]
    fn only_blending_modes_are_removable() {
        let s = |header: &str| Settings {
            g_pre: header.into(),
            ..Settings::default()
        };
        assert!(s("G64 P0.1").has_removable_codes());
        assert!(s("G61.1").has_removable_codes());
        assert!(!s("G17 M3").has_removable_codes());

        // Removing a canned cycle would turn the rest of its line into a move.
        let mut canned = s("G81 X1 Z-1");
        assert!(!canned.has_removable_codes());
        canned.remove_unsupported_codes();
        assert_eq!(canned.g_pre, "G81 X1 Z-1");
        assert_eq!(canned.controller_issues().len(), 1);

        // Lines without anything to remove keep their exact text.
        let mut plain = s("g17  (keep me)|G64 M3");
        plain.remove_unsupported_codes();
        assert_eq!(plain.g_pre, "g17  (keep me)|M3");

        let mut compact = s("G17G64P0.001M3");
        compact.remove_unsupported_codes();
        assert_eq!(compact.g_pre, "G17 M3");
    }

    #[test]
    fn removal_keeps_the_rest_of_the_line() {
        let mut s = Settings {
            g_pre: "G17 G64 P0.001 M3 S3000".into(),
            ..Settings::default()
        };
        s.remove_unsupported_codes();
        assert_eq!(s.g_pre, "G17 M3 S3000");
        assert_eq!(s.g_post, "M5|M2");
        assert!(s.controller_issues().is_empty());

        s.g_pre = "G64 M3 S1|G17 G61.1 P5 M8".into();
        s.remove_unsupported_codes();
        assert_eq!(s.g_pre, "M3 S1|G17 M8");

        // A P word only goes with the code it follows.
        s.g_pre = "M64 P1".into();
        s.remove_unsupported_codes();
        assert_eq!(s.g_pre, "M64 P1");
    }

    #[test]
    fn angle_bounds_are_exclusive() {
        let mut s = Settings {
            angle: 360.0,
            ..Settings::default()
        };
        assert_eq!(s.validate().len(), 1);
        s.angle = -359.9;
        assert!(s.validate().is_empty());
    }

    #[test]
    fn unit_conversion_scales_lengths_not_ratios() {
        let mut s = Settings::default();
        s.set_units(Units::Millimeters);
        assert_abs_diff_eq!(s.y_scale, 50.8);
        assert_abs_diff_eq!(s.v_bit_dia, 12.7);
        assert_abs_diff_eq!(s.feed, 127.0);
        assert_eq!(s.x_scale, 100.0, "percentages are unitless");
        assert_eq!(s.v_bit_angle, 60.0);
        s.set_units(Units::Inches);
        assert_abs_diff_eq!(s.y_scale, 2.0);
        assert_abs_diff_eq!(s.v_bit_dia, 0.5);
    }

    #[test]
    fn setting_same_units_is_a_no_op() {
        let mut s = Settings::default();
        s.set_units(Units::Inches);
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn v_bit_depth_from_geometry() {
        let s = Settings::default();
        // 0.5 diameter at 60 degrees: 0.25 / tan(30).
        assert_abs_diff_eq!(s.max_cut_depth(), -0.433);
    }

    #[test]
    fn depth_limit_narrows_effective_diameter() {
        let s = Settings {
            v_depth_limit: -0.1,
            ..Settings::default()
        };
        assert_abs_diff_eq!(s.effective_bit_dia(), 2.0 * 0.1 * 30f64.to_radians().tan());
        assert_abs_diff_eq!(s.max_cut_depth(), -0.1);
    }

    #[test]
    fn limit_deeper_than_bit_keeps_bit_depth() {
        let s = Settings {
            v_depth_limit: -1.0,
            ..Settings::default()
        };
        assert_abs_diff_eq!(s.max_cut_depth(), -0.433);
    }

    #[test]
    fn ball_bit_geometry() {
        let mut s = Settings {
            bit_shape: BitShape::Ball,
            ..Settings::default()
        };
        assert_abs_diff_eq!(s.effective_bit_dia(), 0.5);
        assert_abs_diff_eq!(s.max_cut_depth(), -0.25);
        s.v_depth_limit = -0.1;
        // Chord of a 0.25-radius circle, 0.1 below the top of the ball.
        assert_abs_diff_eq!(s.effective_bit_dia(), 2.0 * (0.0625f64 - 0.0225).sqrt());
        s.v_depth_limit = -0.5;
        assert_abs_diff_eq!(s.effective_bit_dia(), 0.5);
    }

    #[test]
    fn flat_bit_uses_limit_directly() {
        let s = Settings {
            bit_shape: BitShape::Flat,
            v_depth_limit: -0.75,
            ..Settings::default()
        };
        assert_abs_diff_eq!(s.max_cut_depth(), -0.75);
        assert_abs_diff_eq!(s.effective_bit_dia(), 0.5);
    }

    #[test]
    fn inlay_diameter_comes_from_allowance() {
        let mut s = Settings {
            inlay: true,
            allowance: -0.05,
            ..Settings::default()
        };
        assert_abs_diff_eq!(s.effective_bit_dia(), 2.0 * 0.05 * 30f64.to_radians().tan());
        s.allowance = 0.0;
        assert_abs_diff_eq!(s.effective_bit_dia(), 0.001);
    }

    #[test]
    fn flop_depends_on_transforms_for_text_only() {
        let text = Settings {
            add_box: true,
            mirror: true,
            flip: true,
            ..Settings::default()
        };
        assert!(text.effective_flop(false), "three reversals");
        assert!(!text.effective_flop(true), "cleanup ignores them");
        let image = Settings {
            input_type: InputType::Image,
            ..text
        };
        assert!(!image.effective_flop(false));
    }

    #[test]
    fn settings_round_trip() {
        let original = Settings {
            text: "Héllo\nWörld, a long string of text".into(),
            font_file: "some font.ttf".into(),
            font_dir: PathBuf::from("/a b/c"),
            image_file: PathBuf::from("/x/y z.dxf"),
            input_type: InputType::Image,
            cut_type: CutType::VCarve,
            bit_shape: BitShape::Ball,
            arc_fit: ArcFit::Radius,
            origin: Origin::BotCenter,
            justify: Justify::Right,
            units: Units::Millimeters,
            height_calc: HeightCalc::MaxAll,
            v_check_scope: CheckScope::Char,
            tracer: Tracer::Potrace,
            bmp_turn_policy: TurnPolicy::Left,
            clean_straight: CleanupOps {
                perimeter: false,
                x: true,
                y: true,
                loops: true,
            },
            clean_v_bit: CleanupOps {
                perimeter: true,
                x: false,
                y: true,
                loops: false,
            },
            g_pre: "G17 M3|S1000".into(),
            add_box: true,
            y_scale: 12.5,
            no_comments: false,
            ..Settings::default()
        };
        let text = original.to_settings_lines().join("\n");
        let mut restored = Settings::default();
        let problems = restored.apply_settings_text(&text);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(restored, original);
    }

    #[test]
    fn empty_text_round_trips() {
        let original = Settings {
            text: String::new(),
            ..Settings::default()
        };
        let mut restored = Settings::default();
        restored.apply_settings_text(&original.to_settings_lines().join("\n"));
        assert_eq!(restored.text, "");
    }

    #[test]
    fn reads_legacy_forms() {
        let mut s = Settings::default();
        let problems = s.apply_settings_text(
            "G1 X0\n(fengrave_set plotbox box )\n(fengrave_set arc_fit 1 )\n\
             (fengrave_set STHICK 0.02 )\n(fengrave_set boxsize 5 )\n\
             (fengrave_set TCODE 065 066 )\n(fengrave_set TCODE 067 )\n",
        );
        assert!(problems.is_empty());
        assert!(s.add_box);
        assert_eq!(s.arc_fit, ArcFit::Center);
        assert_abs_diff_eq!(s.box_gap, 0.1);
        assert_eq!(s.text, "ABC");
    }

    #[test]
    fn bad_values_are_reported_and_ignored() {
        let mut s = Settings::default();
        let problems = s.apply_settings_text(
            "(fengrave_set YSCALE abc )\n(fengrave_set nonsense 1 )\n(fengrave_set XSCALE 50 )\n",
        );
        assert_eq!(problems.len(), 2);
        assert_eq!(s.y_scale, 2.0);
        assert_eq!(s.x_scale, 50.0);
    }

    #[test]
    fn enum_parsing_reports_choices() {
        assert_eq!("MM".parse::<Units>(), Ok(Units::Millimeters));
        let err = "cm".parse::<Units>().unwrap_err();
        assert!(err.contains("in, mm"), "{err}");
        for origin in Origin::ALL {
            assert_eq!(origin.name().parse::<Origin>(), Ok(*origin));
        }
    }
}
