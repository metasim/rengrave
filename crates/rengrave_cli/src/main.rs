// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! Command-line front end for `rengrave_core`: generates G-code, SVG, and DXF from text or images.

use clap::{Args, Parser, Subcommand, ValueEnum};
use rengrave_core::font::{BuiltinFont, Font};
use rengrave_core::{
    ArcFit, BitShape, CheckScope, CleanupBit, CutType, GcodeProgram, InputType, Job, Justify,
    Origin, Progress, Settings, Tracer, Units,
};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Parser)]
#[command(
    name = "rengrave-cli",
    version,
    about = "Generate engraving and V-carve G-code from text and images",
    long_about = "Generate engraving and V-carve G-code from text and images.\n\n\
                  A Rust port of F-Engrave. Options override values from a --settings file, \
                  which may be any G-code file or settings file with `fengrave_set` lines, \
                  including those written by F-Engrave.\n\n\
                  Lengths are in the job's units (--units; inches by default)."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Write the G-code program for a job.
    Gcode(OutputArgs),
    /// Write the G-code program for a secondary cleanup operation of a V-carve.
    Cleanup {
        /// Tool to generate the operation for.
        #[arg(long, value_enum, default_value = "straight")]
        tool: CliCleanupBit,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// Write the design outline as an SVG drawing.
    Svg(OutputArgs),
    /// Write the design outline as a DXF drawing.
    Dxf {
        /// Stitch open chains into closed, oriented loops first.
        #[arg(long)]
        close_loops: bool,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// Write the job's settings as a settings file, which `--settings` can load.
    Settings(OutputArgs),
    /// List the built-in fonts, and the font files in a directory.
    Fonts {
        /// Directory to search.
        dir: Option<PathBuf>,
    },
}

#[derive(Args)]
struct OutputArgs {
    #[command(flatten)]
    job: JobArgs,
    /// Output file; writes to standard output if omitted or "-".
    #[arg(short, long, global = false)]
    output: Option<PathBuf>,
}

#[derive(Clone, Copy, ValueEnum)]
enum CliCleanupBit {
    Straight,
    VBit,
}

#[derive(Clone, Copy, ValueEnum)]
enum CliUnits {
    In,
    Mm,
}

#[derive(Clone, Copy, ValueEnum)]
enum CliMode {
    Engrave,
    VCarve,
}

#[derive(Clone, Copy, ValueEnum)]
enum CliBit {
    VBit,
    Ball,
    Flat,
}

#[derive(Clone, Copy, ValueEnum)]
enum CliJustify {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, ValueEnum)]
enum CliArcFit {
    None,
    Center,
    Radius,
}

#[derive(Clone, Copy, ValueEnum)]
enum CliScope {
    Char,
    All,
}

#[derive(Clone, Copy, ValueEnum)]
enum CliTracer {
    Native,
    Potrace,
}

#[derive(Clone, Copy, ValueEnum)]
enum CliOrigin {
    Default,
    TopLeft,
    TopCenter,
    TopRight,
    MidLeft,
    MidCenter,
    MidRight,
    BotLeft,
    BotCenter,
    BotRight,
    ArcCenter,
}

#[derive(Args)]
struct JobArgs {
    /// Load settings from a file.
    #[arg(short = 'c', long, value_name = "FILE")]
    settings: Option<PathBuf>,
    /// Set any value by its settings-file name; repeatable.
    #[arg(long = "set", value_name = "NAME=VALUE", help_heading = "Advanced")]
    set: Vec<String>,

    /// Text to engrave; "|" starts a new line.
    #[arg(short, long, help_heading = "Input")]
    text: Option<String>,
    /// Font: a .cxf/.ttf/.otf file, or a built-in name ("Ubuntu Light", "Hack").
    #[arg(short, long, help_heading = "Input")]
    font: Option<String>,
    /// DXF drawing or raster image (PNG, BMP, PBM, JPEG, GIF, TIFF) to engrave instead of text.
    #[arg(
        short,
        long,
        value_name = "FILE",
        help_heading = "Input",
        conflicts_with = "text"
    )]
    image: Option<PathBuf>,
    /// Program that traces raster images.
    #[arg(long, value_enum, help_heading = "Input")]
    tracer: Option<CliTracer>,

    /// Linear units for all lengths.
    #[arg(short, long, value_enum, help_heading = "Size and placement")]
    units: Option<CliUnits>,
    /// Text height, or image height. With --image-percent, a percentage of native size.
    #[arg(long, help_heading = "Size and placement")]
    height: Option<f64>,
    /// Interpret --height of an image as a percentage of its native size.
    #[arg(long, help_heading = "Size and placement")]
    image_percent: bool,
    /// Width, as a percentage of proportional.
    #[arg(long, help_heading = "Size and placement")]
    width_percent: Option<f64>,
    /// Rotation, in degrees.
    #[arg(
        long,
        allow_negative_numbers = true,
        help_heading = "Size and placement"
    )]
    angle: Option<f64>,
    /// Place text on a circle of this radius.
    #[arg(long, help_heading = "Size and placement")]
    radius: Option<f64>,
    /// Multi-line alignment.
    #[arg(long, value_enum, help_heading = "Size and placement")]
    justify: Option<CliJustify>,
    /// Location of the work origin.
    #[arg(long, value_enum, help_heading = "Size and placement")]
    origin: Option<CliOrigin>,
    /// Mirror about a vertical line.
    #[arg(long, help_heading = "Size and placement")]
    mirror: bool,
    /// Mirror about a horizontal line.
    #[arg(long, help_heading = "Size and placement")]
    flip: bool,
    /// Add a box around the design (or a circle around engraved text on a circle).
    #[arg(long, help_heading = "Size and placement")]
    add_box: bool,

    /// Cut type.
    #[arg(short, long, value_enum, help_heading = "Machine")]
    mode: Option<CliMode>,
    /// Feed rate, in units per minute.
    #[arg(long, help_heading = "Machine")]
    feed: Option<f64>,
    /// Plunge rate, in units per minute; zero uses the feed rate.
    #[arg(long, help_heading = "Machine")]
    plunge: Option<f64>,
    /// Height for rapid moves.
    #[arg(long, allow_negative_numbers = true, help_heading = "Machine")]
    z_safe: Option<f64>,
    /// Engraving depth (negative).
    #[arg(long, allow_negative_numbers = true, help_heading = "Machine")]
    depth: Option<f64>,
    /// Arc output format.
    #[arg(long, value_enum, help_heading = "Machine")]
    arc_fit: Option<CliArcFit>,
    /// Include comments, and recoverable settings, in the G-code.
    #[arg(long, help_heading = "Machine")]
    comments: bool,

    /// Bit shape.
    #[arg(long, value_enum, help_heading = "V-carve")]
    bit: Option<CliBit>,
    /// Included angle of a V-bit, in degrees.
    #[arg(long, help_heading = "V-carve")]
    bit_angle: Option<f64>,
    /// Bit diameter.
    #[arg(long, help_heading = "V-carve")]
    bit_dia: Option<f64>,
    /// Deepest allowed cut (negative).
    #[arg(long, allow_negative_numbers = true, help_heading = "V-carve")]
    depth_limit: Option<f64>,
    /// Distance between samples along the outline.
    #[arg(long, help_heading = "V-carve")]
    step: Option<f64>,
    /// Stock left for the finish pass; greater than zero enables multi-pass cutting.
    #[arg(long, help_heading = "V-carve")]
    finish_stock: Option<f64>,
    /// Maximum depth per roughing pass (negative).
    #[arg(long, allow_negative_numbers = true, help_heading = "V-carve")]
    pass_depth: Option<f64>,
    /// Which outlines limit the carve at each point.
    #[arg(long, value_enum, help_heading = "V-carve")]
    check: Option<CliScope>,
    /// Cut outside the outlines rather than inside.
    #[arg(long, help_heading = "V-carve")]
    flop: bool,
    /// Cut a prismatic plug for inlay work.
    #[arg(long, help_heading = "V-carve")]
    inlay: bool,

    /// Print progress to standard error.
    #[arg(short, long)]
    verbose: bool,
}

impl JobArgs {
    /// Combines defaults, the settings file, and the command line.
    fn settings(&self) -> Result<Settings, String> {
        let mut s = match self.units {
            Some(CliUnits::Mm) => Settings::with_units(Units::Millimeters),
            _ => Settings::default(),
        };
        if let Some(path) = &self.settings {
            let text =
                std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
            for problem in s.apply_settings_text(&text) {
                eprintln!("warning: {}: {problem}", path.display());
            }
            if let Some(units) = self.units {
                s.set_units(match units {
                    CliUnits::In => Units::Inches,
                    CliUnits::Mm => Units::Millimeters,
                });
            }
        }
        for pair in &self.set {
            let (name, value) = pair
                .split_once('=')
                .ok_or_else(|| format!("--set expects NAME=VALUE, got {pair:?}"))?;
            s.apply_setting(name.trim(), value.trim())
                .map_err(|e| format!("--set {name}: {e}"))?;
        }

        if let Some(text) = &self.text {
            s.text = text.replace('|', "\n");
            s.input_type = InputType::Text;
        }
        if let Some(font) = &self.font {
            let path = Path::new(font);
            if BuiltinFont::from_name(font).is_none() && !font.is_empty() {
                s.font_dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
                s.font_file = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .ok_or_else(|| format!("invalid font path {font:?}"))?;
            } else {
                s.font_file.clone_from(font);
            }
        }
        if let Some(image) = &self.image {
            s.image_file.clone_from(image);
            s.input_type = InputType::Image;
        }
        if let Some(t) = self.tracer {
            s.tracer = match t {
                CliTracer::Native => Tracer::Native,
                CliTracer::Potrace => Tracer::Potrace,
            };
        }
        if let Some(v) = self.height {
            s.y_scale = v;
        }
        s.use_image_size |= self.image_percent;
        if let Some(v) = self.width_percent {
            s.x_scale = v;
        }
        if let Some(v) = self.angle {
            s.angle = v;
        }
        if let Some(v) = self.radius {
            s.text_radius = v;
        }
        if let Some(v) = self.justify {
            s.justify = match v {
                CliJustify::Left => Justify::Left,
                CliJustify::Center => Justify::Center,
                CliJustify::Right => Justify::Right,
            };
        }
        if let Some(v) = self.origin {
            s.origin = match v {
                CliOrigin::Default => Origin::Default,
                CliOrigin::TopLeft => Origin::TopLeft,
                CliOrigin::TopCenter => Origin::TopCenter,
                CliOrigin::TopRight => Origin::TopRight,
                CliOrigin::MidLeft => Origin::MidLeft,
                CliOrigin::MidCenter => Origin::MidCenter,
                CliOrigin::MidRight => Origin::MidRight,
                CliOrigin::BotLeft => Origin::BotLeft,
                CliOrigin::BotCenter => Origin::BotCenter,
                CliOrigin::BotRight => Origin::BotRight,
                CliOrigin::ArcCenter => Origin::ArcCenter,
            };
        }
        s.mirror |= self.mirror;
        s.flip |= self.flip;
        s.add_box |= self.add_box;
        if let Some(v) = self.mode {
            s.cut_type = match v {
                CliMode::Engrave => CutType::Engrave,
                CliMode::VCarve => CutType::VCarve,
            };
        }
        if let Some(v) = self.feed {
            s.feed = v;
        }
        if let Some(v) = self.plunge {
            s.plunge = v;
        }
        if let Some(v) = self.z_safe {
            s.z_safe = v;
        }
        if let Some(v) = self.depth {
            s.z_cut = v;
        }
        if let Some(v) = self.arc_fit {
            s.arc_fit = match v {
                CliArcFit::None => ArcFit::None,
                CliArcFit::Center => ArcFit::Center,
                CliArcFit::Radius => ArcFit::Radius,
            };
        }
        if self.comments {
            s.no_comments = false;
        }
        if let Some(v) = self.bit {
            s.bit_shape = match v {
                CliBit::VBit => BitShape::VBit,
                CliBit::Ball => BitShape::Ball,
                CliBit::Flat => BitShape::Flat,
            };
        }
        if let Some(v) = self.bit_angle {
            s.v_bit_angle = v;
        }
        if let Some(v) = self.bit_dia {
            s.v_bit_dia = v;
        }
        if let Some(v) = self.depth_limit {
            s.v_depth_limit = v;
        }
        if let Some(v) = self.step {
            s.v_step_len = v;
        }
        if let Some(v) = self.finish_stock {
            s.v_rough_stock = v;
        }
        if let Some(v) = self.pass_depth {
            s.v_max_cut = v;
        }
        if let Some(v) = self.check {
            s.v_check_scope = match v {
                CliScope::Char => CheckScope::Char,
                CliScope::All => CheckScope::All,
            };
        }
        s.v_flop |= self.flop;
        s.inlay |= self.inlay;
        Ok(s)
    }

    fn job(&self) -> Result<Job, String> {
        let settings = self.settings()?;
        settings.ensure_valid().map_err(|e| e.to_string())?;
        let job = Job::load(settings).map_err(|e| e.to_string())?;
        for warning in job.warnings.iter().chain(&job.settings.controller_issues()) {
            eprintln!("warning: {warning}");
        }
        Ok(job)
    }

    /// A progress handle that prints whole percentages to standard error when verbose.
    fn progress(&self) -> Progress {
        if !self.verbose {
            return Progress::none();
        }
        let last = AtomicU32::new(u32::MAX);
        let is_tty = std::io::stderr().is_terminal();
        Progress::with_callback(move |fraction| {
            let percent = (fraction * 100.0) as u32;
            if last.swap(percent, Ordering::Relaxed) != percent {
                if is_tty {
                    eprint!("\rcalculating: {percent:3}%");
                    if percent == 100 {
                        eprintln!();
                    }
                } else if percent.is_multiple_of(10) {
                    eprintln!("calculating: {percent}%");
                }
            }
        })
    }
}

impl OutputArgs {
    fn write(&self, lines: &[String]) -> Result<(), String> {
        let path = self.output.as_deref();
        let mut text = lines.join("\n");
        text.push('\n');
        match path {
            Some(p) if p != Path::new("-") => {
                std::fs::write(p, text).map_err(|e| format!("{}: {e}", p.display()))
            }
            _ => std::io::stdout()
                .write_all(text.as_bytes())
                .map_err(|e| format!("standard output: {e}")),
        }
    }
}

impl JobArgs {
    /// Warns about characters that the font could not supply.
    fn warn_missing(job: &Job) {
        if let Ok(design) = job.layout()
            && !design.missing_chars.is_empty()
        {
            let list: String = design.missing_chars.iter().collect();
            eprintln!("warning: characters not in font: {list}");
        }
    }
}

impl Cli {
    fn describe_font(label: &str, font: &Font) {
        println!("{label}  ({} characters)", font.glyphs.len());
    }

    fn run(self) -> Result<(), String> {
        match self.command {
            Command::Gcode(out) => {
                let job = out.job.job()?;
                JobArgs::warn_missing(&job);
                let program = job
                    .generate_gcode(&out.job.progress())
                    .map_err(|e| e.to_string())?;
                out.write(&program)
            }
            Command::Cleanup { tool, output } => {
                let job = output.job.job()?;
                let bit = match tool {
                    CliCleanupBit::Straight => CleanupBit::Straight,
                    CliCleanupBit::VBit => CleanupBit::VBit,
                };
                let program = job
                    .generate_cleanup_gcode(bit, &output.job.progress())
                    .map_err(|e| e.to_string())?;
                output.write(&program)
            }
            Command::Svg(out) => {
                let job = out.job.job()?;
                JobArgs::warn_missing(&job);
                let design = job.layout().map_err(|e| e.to_string())?;
                out.write(&design.to_svg(&job.settings))
            }
            Command::Dxf {
                close_loops,
                output,
            } => {
                let job = output.job.job()?;
                JobArgs::warn_missing(&job);
                let design = job.layout().map_err(|e| e.to_string())?;
                output.write(&design.to_dxf(&job.settings, close_loops))
            }
            Command::Settings(out) => {
                let settings = out.job.settings()?;
                out.write(&GcodeProgram::settings_file(&settings))
            }
            Command::Fonts { dir } => {
                for builtin in BuiltinFont::ALL {
                    let font = builtin.load(5.0, false).map_err(|e| e.to_string())?;
                    Self::describe_font(&format!("(built-in) {}", builtin.name()), &font);
                }
                if let Some(dir) = dir {
                    for path in Font::list_files(&dir).map_err(|e| e.to_string())? {
                        match Font::load_file(&path, 5.0, false) {
                            Ok(font) => Self::describe_font(&path.display().to_string(), &font),
                            Err(e) => eprintln!("warning: {e}"),
                        }
                    }
                }
                Ok(())
            }
        }
    }
}

fn main() -> ExitCode {
    match Cli::parse().run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn job_args(args: &[&str]) -> JobArgs {
        let mut full = vec!["rengrave-cli", "gcode"];
        full.extend_from_slice(args);
        match Cli::parse_from(full).command {
            Command::Gcode(out) => out.job,
            _ => unreachable!(),
        }
    }

    #[test]
    fn command_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn defaults_apply_without_options() {
        assert_eq!(job_args(&[]).settings().unwrap(), Settings::default());
    }

    #[test]
    fn metric_flag_uses_converted_defaults() {
        let s = job_args(&["--units", "mm"]).settings().unwrap();
        assert_eq!(s.units, Units::Millimeters);
        assert_eq!(s.y_scale, 50.8);
    }

    #[test]
    fn options_map_to_settings() {
        let s = job_args(&[
            "-t",
            "a|b",
            "--height",
            "3",
            "--mode",
            "v-carve",
            "--bit",
            "ball",
            "--bit-dia",
            "0.1",
            "--angle",
            "-15",
            "--origin",
            "mid-center",
            "--justify",
            "center",
            "--mirror",
            "--depth-limit",
            "-0.2",
            "--arc-fit",
            "radius",
            "--finish-stock",
            "0.01",
            "--pass-depth",
            "-0.05",
            "--check",
            "char",
            "--z-safe",
            "0.5",
            "--comments",
        ])
        .settings()
        .unwrap();
        assert_eq!(s.text, "a\nb");
        assert_eq!(s.y_scale, 3.0);
        assert_eq!(s.cut_type, CutType::VCarve);
        assert_eq!(s.bit_shape, BitShape::Ball);
        assert_eq!(s.angle, -15.0);
        assert_eq!(s.origin, Origin::MidCenter);
        assert_eq!(s.justify, Justify::Center);
        assert!(s.mirror);
        assert_eq!(s.v_depth_limit, -0.2);
        assert_eq!(s.arc_fit, ArcFit::Radius);
        assert_eq!(s.v_rough_stock, 0.01);
        assert_eq!(s.v_max_cut, -0.05);
        assert_eq!(s.v_check_scope, CheckScope::Char);
        assert_eq!(s.z_safe, 0.5);
        assert!(!s.no_comments);
    }

    #[test]
    fn font_option_splits_directory_or_names_builtin() {
        let s = job_args(&["-f", "/fonts/x.ttf"]).settings().unwrap();
        assert_eq!(s.font_dir, PathBuf::from("/fonts"));
        assert_eq!(s.font_file, "x.ttf");
        let b = job_args(&["-f", "hack"]).settings().unwrap();
        assert_eq!(b.font_file, "hack");
    }

    #[test]
    fn image_option_switches_input() {
        let s = job_args(&["-i", "x.dxf"]).settings().unwrap();
        assert_eq!(s.input_type, InputType::Image);
    }

    #[test]
    fn generic_set_option_and_errors() {
        let s = job_args(&["--set", "YSCALE=7"]).settings().unwrap();
        assert_eq!(s.y_scale, 7.0);
        assert!(job_args(&["--set", "nonsense"]).settings().is_err());
        assert!(job_args(&["--set", "nonsense=1"]).settings().is_err());
    }

    #[test]
    fn missing_settings_file_is_an_error() {
        assert!(job_args(&["-c", "/nonexistent"]).settings().is_err());
    }

    #[test]
    fn invalid_values_are_rejected_before_loading() {
        let err = job_args(&["--height=-1", "--z-safe", "1"])
            .job()
            .unwrap_err();
        assert!(err.contains("y_scale"), "{err}");
    }
}
