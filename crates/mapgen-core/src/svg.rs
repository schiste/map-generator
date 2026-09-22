use std::fmt::Write;

use geo_types::{LineString, MultiPolygon};

use crate::feature::MapFeature;

/// Presentation options for the SVG writer.
///
/// The defaults follow the Wikimedia Commons location-map palette.
#[derive(Debug, Clone, PartialEq)]
pub struct SvgOptions {
    /// Output width in pixels; height is derived from the projected aspect ratio.
    pub width: u32,
    /// Blank margin around the drawing, in pixels.
    pub padding: u32,
    /// Decimal places kept for path coordinates.
    pub precision: usize,
    pub title: Option<String>,
    pub fill: String,
    pub stroke: String,
    pub stroke_width: f64,
}

impl Default for SvgOptions {
    fn default() -> Self {
        Self {
            width: 1000,
            padding: 10,
            precision: 1,
            title: None,
            fill: "#fefee9".into(),
            stroke: "#646464".into(),
            stroke_width: 0.5,
        }
    }
}

/// Affine transform from projected metres to SVG pixels (y axis flipped).
#[derive(Debug, Clone, Copy)]
pub struct Viewport {
    pub min_x: f64,
    pub max_y: f64,
    pub scale: f64,
    pub padding: f64,
    pub width: u32,
    pub height: u32,
}

impl Viewport {
    fn to_px(self, x: f64, y: f64) -> (f64, f64) {
        (
            self.padding + (x - self.min_x) * self.scale,
            self.padding + (self.max_y - y) * self.scale,
        )
    }
}

/// Serialises already-projected features into an SVG document.
pub fn write_svg(features: &[MapFeature], viewport: Viewport, opts: &SvgOptions) -> String {
    let mut out = String::new();
    let (w, h) = (viewport.width, viewport.height);
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">"
    );
    if let Some(title) = &opts.title {
        let _ = writeln!(out, "<title>{}</title>", escape(title));
    }
    let _ = writeln!(
        out,
        "<style>path{{fill:{};stroke:{};stroke-width:{};stroke-linejoin:round;fill-rule:evenodd}}</style>",
        escape(&opts.fill),
        escape(&opts.stroke),
        fmt_num(opts.stroke_width, 3)
    );
    out.push_str("<g id=\"features\">\n");
    for f in features {
        let d = path_data(&f.geometry, viewport, opts.precision);
        if d.is_empty() {
            continue;
        }
        let _ = writeln!(
            out,
            "<path id=\"{}\" class=\"{}\" data-name=\"{}\" d=\"{}\"><title>{}</title></path>",
            escape(&f.id),
            escape(&f.class),
            escape(&f.name),
            d,
            escape(&f.name)
        );
    }
    out.push_str("</g>\n</svg>\n");
    out
}

fn path_data(geometry: &MultiPolygon<f64>, vp: Viewport, precision: usize) -> String {
    let mut d = String::new();
    for poly in geometry {
        ring_data(&mut d, poly.exterior(), vp, precision);
        for hole in poly.interiors() {
            ring_data(&mut d, hole, vp, precision);
        }
    }
    d
}

fn ring_data(d: &mut String, ring: &LineString<f64>, vp: Viewport, precision: usize) {
    // Round first, then drop consecutive duplicates that rounding created.
    let mut pts: Vec<(String, String)> = Vec::with_capacity(ring.0.len());
    for c in ring.0.iter() {
        let (x, y) = vp.to_px(c.x, c.y);
        let p = (fmt_num(x, precision), fmt_num(y, precision));
        if pts.last() != Some(&p) {
            pts.push(p);
        }
    }
    if pts.len() > 1 && pts.first() == pts.last() {
        pts.pop();
    }
    if pts.len() < 3 {
        return;
    }
    for (i, (x, y)) in pts.iter().enumerate() {
        let _ = write!(d, "{}{x} {y}", if i == 0 { "M" } else { "L" });
    }
    d.push('Z');
}

/// Fixed-precision number formatting with trailing zeros and `-0` removed,
/// so output is stable across platforms and compact.
pub fn fmt_num(v: f64, precision: usize) -> String {
    let mut s = format!("{v:.precision$}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" {
        s = "0".into();
    }
    s
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_numbers_compactly() {
        assert_eq!(fmt_num(1.50, 2), "1.5");
        assert_eq!(fmt_num(2.0, 1), "2");
        assert_eq!(fmt_num(-0.04, 1), "0");
        assert_eq!(fmt_num(12.345, 1), "12.3");
    }

    #[test]
    fn escapes_xml() {
        assert_eq!(escape("A&B <\"x\">"), "A&amp;B &lt;&quot;x&quot;&gt;");
    }
}
