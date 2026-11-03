//! Exports the design outline as vector drawings.

use crate::design::Design;
use crate::settings::{CutType, Settings};
use crate::sort::sort_for_v_carve;

/// Writes `design`'s outline as an SVG document, one line per element.
pub fn write_svg(design: &Design, settings: &Settings) -> Vec<String> {
    const DPI: f64 = 100.0;
    let thick = if settings.cut_type == CutType::VCarve {
        0.001
    } else {
        settings.stroke_thickness
    };
    let mut bounds = crate::geom::Bounds::EMPTY;
    for s in &design.segments {
        bounds.include(s.x1, s.y1);
        bounds.include(s.x2, s.y2);
    }
    if design.circle_radius != 0.0 {
        let c = design.circle_center;
        bounds.include(c.x + design.circle_radius, c.y + design.circle_radius);
        bounds.include(c.x - design.circle_radius, c.y - design.circle_radius);
    }
    if bounds.is_empty() {
        bounds.include(0.0, 0.0);
    }
    let (min_x, min_y) = (bounds.min_x - thick / 2.0, bounds.min_y - thick / 2.0);
    let (max_x, max_y) = (bounds.max_x + thick / 2.0, bounds.max_y + thick / 2.0);
    let (width_in, height_in) = (max_x - min_x, max_y - min_y);
    let units = settings.units;

    let mut out = vec![
        r#"<?xml version="1.0" standalone="no"?>"#.to_string(),
        r#"<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN""#.to_string(),
        r#"  "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd">"#.to_string(),
        format!(
            r#"<svg width="{width_in:.6}{units}" height="{height_in:.6}{units}" viewBox="0 0 {:.6} {:.6}""#,
            width_in * DPI,
            height_in * DPI
        ),
        r#"     xmlns="http://www.w3.org/2000/svg" version="1.1">"#.to_string(),
        "  <title>rengrave output</title>".to_string(),
        "  <desc>SVG file created by rengrave</desc>".to_string(),
    ];
    let stroke_width = thick * DPI;
    if design.circle_radius != 0.0 && settings.cut_type == CutType::Engrave {
        out.push(format!(
            r#"  <circle cx="{:.6}" cy="{:.6}" r="{:.6}" fill="none" stroke="blue" stroke-width="{stroke_width:.6}"/>"#,
            (design.circle_center.x - min_x) * DPI,
            (-design.circle_center.y + max_y) * DPI,
            design.circle_radius * DPI
        ));
    }
    for s in &design.segments {
        out.push(format!(
            r#"  <path d="M {:.6} {:.6} L {:.6} {:.6}" fill="none" stroke="blue" stroke-width="{stroke_width:.6}" stroke-linecap="round" stroke-linejoin="round"/>"#,
            (s.x1 - min_x) * DPI,
            (-s.y1 + max_y) * DPI,
            (s.x2 - min_x) * DPI,
            (-s.y2 + max_y) * DPI
        ));
    }
    out.push("</svg>".to_string());
    out
}

/// Writes `design`'s outline as a DXF R12-style drawing, one group code or value per line.
///
/// With `close_loops`, open chains are first stitched into closed, oriented loops.
pub fn write_dxf(design: &Design, settings: &Settings, close_loops: bool) -> Vec<String> {
    let sorted;
    let segments = if close_loops {
        sorted = sort_for_v_carve(&design.segments, settings.accuracy);
        &sorted
    } else {
        &design.segments
    };
    let mut out: Vec<String> = [
        "999",
        "DXF created by rengrave",
        "0",
        "SECTION",
        "2",
        "HEADER",
        "0",
        "ENDSEC",
        "0",
        "SECTION",
        "2",
        "TABLES",
        "0",
        "TABLE",
        "2",
        "LTYPE",
        "70",
        "1",
        "0",
        "LTYPE",
        "2",
        "CONTINUOUS",
        "70",
        "64",
        "3",
        "Solid line",
        "72",
        "65",
        "73",
        "0",
        "40",
        "0.000000",
        "0",
        "ENDTAB",
        "0",
        "TABLE",
        "2",
        "LAYER",
        "70",
        "6",
        "0",
        "LAYER",
        "2",
        "1",
        "70",
        "64",
        "62",
        "7",
        "6",
        "CONTINUOUS",
        "0",
        "LAYER",
        "2",
        "2",
        "70",
        "64",
        "62",
        "7",
        "6",
        "CONTINUOUS",
        "0",
        "ENDTAB",
        "0",
        "TABLE",
        "2",
        "STYLE",
        "70",
        "0",
        "0",
        "ENDTAB",
        "0",
        "ENDSEC",
        "0",
        "SECTION",
        "2",
        "BLOCKS",
        "0",
        "ENDSEC",
        "0",
        "SECTION",
        "2",
        "ENTITIES",
        "  0",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    for s in segments {
        for fixed in [
            "LINE",
            "  5",
            "30",
            "100",
            "AcDbEntity",
            "  8",
            "1",
            " 62",
            "150",
            "100",
            "AcDbLine",
        ] {
            out.push(fixed.to_string());
        }
        for (code, value) in [
            (" 10", s.x1),
            (" 20", s.y1),
            (" 30", 0.0),
            (" 11", s.x2),
            (" 21", s.y2),
            (" 31", 0.0),
        ] {
            out.push(code.to_string());
            out.push(format!("{value:.4}"));
        }
        out.push("  0".to_string());
    }
    out.extend(["ENDSEC", "0", "EOF"].iter().map(|s| s.to_string()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dxf::read_dxf;
    use crate::font::{Font, Stroke};
    use crate::settings::InputType;

    fn design(settings: &Settings) -> Design {
        let font = Font::from_drawing(vec![
            Stroke::new(0.0, 0.0, 0.0, 1.0),
            Stroke::new(0.0, 1.0, 2.0, 1.0),
        ]);
        Design::layout(&font, settings).unwrap()
    }

    fn settings() -> Settings {
        Settings {
            input_type: InputType::Image,
            y_scale: 1.0,
            stroke_thickness: 0.0,
            ..Settings::default()
        }
    }

    #[test]
    fn svg_flips_y_and_sizes_document() {
        let s = settings();
        let svg = write_svg(&design(&s), &s);
        assert!(svg[3].contains(r#"width="2.000000in""#), "{}", svg[3]);
        assert!(svg[3].contains(r#"viewBox="0 0 200.000000 100.000000""#));
        // First segment: (0,0)->(0,1) in design space runs bottom to top in the document.
        assert!(
            svg[7].contains("M 0.000000 100.000000 L 0.000000 0.000000"),
            "{}",
            svg[7]
        );
        assert_eq!(svg.last().unwrap(), "</svg>");
    }

    #[test]
    fn svg_handles_empty_design() {
        let s = settings();
        let mut d = design(&s);
        d.segments.clear();
        assert_eq!(write_svg(&d, &s).last().unwrap(), "</svg>");
    }

    #[test]
    fn svg_includes_circle_when_engraving() {
        let s = settings();
        let mut d = design(&s);
        d.circle_radius = 3.0;
        let svg = write_svg(&d, &s).join("\n");
        assert!(svg.contains("<circle"));
    }

    #[test]
    fn dxf_round_trips_through_reader() {
        let s = settings();
        let d = design(&s);
        let text = write_dxf(&d, &s, false).join("\n");
        let imported = read_dxf(&text, 5.0).unwrap();
        assert_eq!(imported.strokes.len(), 2);
        assert_eq!(imported.strokes[1], Stroke::new(0.0, 1.0, 2.0, 1.0));
    }

    #[test]
    fn dxf_can_close_loops() {
        let s = settings();
        let d = design(&s);
        let text = write_dxf(&d, &s, true).join("\n");
        let imported = read_dxf(&text, 5.0).unwrap();
        assert_eq!(imported.strokes.len(), 3, "closing segment added");
    }
}
