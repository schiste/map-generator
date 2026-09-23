use std::collections::HashSet;
use std::fmt::Write;

use geo::{Area, BoundingRect, InteriorPoint};
use geo_types::{LineString, MultiPolygon};

use crate::feature::MapFeature;
use crate::theme::Theme;

/// Affine transform from projected metres to SVG pixels (y axis flipped).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub min_x: f64,
    pub max_y: f64,
    pub scale: f64,
    pub padding: f64,
    pub width: u32,
    pub height: u32,
}

impl Viewport {
    pub fn to_px(self, x: f64, y: f64) -> (f64, f64) {
        (
            self.padding + (x - self.min_x) * self.scale,
            self.padding + (self.max_y - y) * self.scale,
        )
    }
}

/// Everything the writer needs; all geometries are already projected.
#[derive(Debug, Clone, Copy)]
pub struct SvgDocument<'a> {
    pub viewport: Viewport,
    /// Sea: the frame rectangle, or the globe outline on world maps.
    pub water: &'a MultiPolygon<f64>,
    pub context: &'a [MapFeature],
    pub subject: &'a [MapFeature],
    pub lakes: &'a [MapFeature],
    pub labels: bool,
    pub theme: &'a Theme,
    pub title: Option<&'a str>,
    /// Data credit, embedded as `<desc>` (and drawn if `credit` is set).
    pub attribution: Option<&'a str>,
    /// Draw the attribution in the bottom-right corner.
    pub credit: bool,
    /// Decimal places kept for coordinates.
    pub precision: usize,
    /// Emit colours as `var(--mg-<slot>, <colour>)` so a page embedding the
    /// SVG inline can restyle it with CSS custom properties.
    pub css_vars: bool,
}

/// Serialises a map into an SVG document.
///
/// Layers, bottom to top: `#background`, `#water`, `#context`, `#land`,
/// `#lakes`, `#labels`. Colours live in the `<style>` block only.
pub fn write_svg(doc: &SvgDocument) -> String {
    let vp = doc.viewport;
    let p = doc.precision;
    let (w, h) = (vp.width, vp.height);
    let mut out = String::new();
    let mut ids = HashSet::new();

    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">"
    );
    if let Some(title) = doc.title {
        let _ = writeln!(out, "<title>{}</title>", escape(title));
    }
    if let Some(attribution) = doc.attribution {
        let _ = writeln!(
            out,
            "<desc id=\"attribution\">{}</desc>",
            escape(attribution)
        );
    }
    let _ = writeln!(
        out,
        "<style>\n{}</style>",
        stylesheet(doc.theme, doc.css_vars)
    );
    let _ = writeln!(
        out,
        "<rect id=\"background\" class=\"mg-background\" width=\"{w}\" height=\"{h}\"/>"
    );
    let water = path_data(doc.water, vp, p);
    if !water.is_empty() {
        let _ = writeln!(out, "<path id=\"water\" class=\"mg-water\" d=\"{water}\"/>");
    }
    write_layer(
        &mut out,
        &mut ids,
        "context",
        "mg-context",
        "",
        doc.context,
        vp,
        p,
    );
    write_layer(
        &mut out,
        &mut ids,
        "land",
        "mg-land",
        "",
        doc.subject,
        vp,
        p,
    );
    write_layer(
        &mut out, &mut ids, "lakes", "mg-lake", "lake-", doc.lakes, vp, p,
    );
    if doc.labels {
        write_labels(&mut out, doc.subject, vp, doc.theme.label_size);
    }
    if let (true, Some(attribution)) = (doc.credit, doc.attribution) {
        let _ = writeln!(
            out,
            "<text id=\"credit\" class=\"mg-credit\" x=\"{}\" y=\"{}\">{}</text>",
            w.saturating_sub(4),
            h.saturating_sub(4),
            escape(attribution)
        );
    }
    out.push_str("</svg>\n");
    out
}

/// The `<style>` contents: every colour of the map is defined here.
pub fn stylesheet(theme: &Theme, css_vars: bool) -> String {
    let c = |slot: &str| {
        let value = theme
            .colors()
            .iter()
            .find(|(s, _)| *s == slot)
            .map(|(_, c)| c.as_str().to_owned())
            .unwrap_or_default();
        if css_vars {
            format!("var(--mg-{slot},{value})")
        } else {
            value
        }
    };
    let bw = fmt_num(theme.border_width, 3);
    let cbw = fmt_num(theme.context_border_width, 3);
    let ls = fmt_num(theme.label_size, 2);
    format!(
        "path{{stroke-linejoin:round;fill-rule:evenodd}}\n\
         .mg-background{{fill:{bg}}}\n\
         .mg-water{{fill:{water}}}\n\
         .mg-context{{fill:{ctx};stroke:{ctxb};stroke-width:{cbw}}}\n\
         .mg-land{{fill:{land};stroke:{border};stroke-width:{bw}}}\n\
         .mg-lake{{fill:{water};stroke:{lakeb};stroke-width:{cbw}}}\n\
         .mg-label{{fill:{label};font:{ls}px sans-serif;text-anchor:middle;dominant-baseline:central;\
         paint-order:stroke;stroke:{land};stroke-width:2.5px;stroke-linejoin:round}}\n\
         .mg-credit{{fill:{label};font:9px sans-serif;text-anchor:end;opacity:.75}}\n",
        bg = c("background"),
        water = c("water"),
        ctx = c("context-land"),
        ctxb = c("context-border"),
        land = c("land"),
        border = c("border"),
        lakeb = c("lake-border"),
        label = c("label"),
    )
}

#[allow(clippy::too_many_arguments)]
fn write_layer(
    out: &mut String,
    ids: &mut HashSet<String>,
    group: &str,
    layer_class: &str,
    id_prefix: &str,
    features: &[MapFeature],
    vp: Viewport,
    precision: usize,
) {
    let paths: Vec<(&MapFeature, String)> = features
        .iter()
        .map(|f| (f, path_data(&f.geometry, vp, precision)))
        .filter(|(_, d)| !d.is_empty())
        .collect();
    if paths.is_empty() {
        return;
    }
    let _ = writeln!(out, "<g id=\"{group}\">");
    for (f, d) in paths {
        let id = unique_id(ids, &format!("{id_prefix}{}", f.id));
        let _ = writeln!(
            out,
            "<path id=\"{}\" class=\"{layer_class} {}\" data-name=\"{}\" d=\"{d}\"><title>{}</title></path>",
            escape(&id),
            escape(&f.class),
            escape(&f.name),
            escape(&f.name),
        );
    }
    out.push_str("</g>\n");
}

fn write_labels(out: &mut String, features: &[MapFeature], vp: Viewport, size: f64) {
    // Candidates: (area, feature index, x, y, half width, half height) in px.
    let mut candidates = Vec::new();
    for (i, f) in features.iter().enumerate() {
        let Some(poly) = f
            .geometry
            .0
            .iter()
            .max_by(|a, b| a.unsigned_area().total_cmp(&b.unsigned_area()))
        else {
            continue;
        };
        let (Some(pt), Some(bb)) = (poly.interior_point(), poly.bounding_rect()) else {
            continue;
        };
        let (hw, hh) = (0.3 * size * f.name.chars().count() as f64, 0.6 * size);
        // Skip labels that clearly don't fit their region.
        if bb.width() * vp.scale < 2.0 * hw || bb.height() * vp.scale < 2.0 * hh {
            continue;
        }
        let (x, y) = vp.to_px(pt.x(), pt.y());
        candidates.push((poly.unsigned_area(), i, x, y, hw, hh));
    }
    // Largest regions claim space first; a label overlapping one already
    // placed is dropped.
    candidates.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut placed: Vec<(usize, f64, f64, f64, f64)> = Vec::new();
    for &(_, i, x, y, hw, hh) in &candidates {
        let clear = placed
            .iter()
            .all(|&(_, px, py, phw, phh)| (x - px).abs() >= hw + phw || (y - py).abs() >= hh + phh);
        if clear {
            placed.push((i, x, y, hw, hh));
        }
    }
    if placed.is_empty() {
        return;
    }
    placed.sort_by_key(|p| p.0);
    out.push_str("<g id=\"labels\">\n");
    for (i, x, y, _, _) in placed {
        let _ = writeln!(
            out,
            "<text class=\"mg-label\" x=\"{}\" y=\"{}\">{}</text>",
            fmt_num(x, 1),
            fmt_num(y, 1),
            escape(&features[i].name)
        );
    }
    out.push_str("</g>\n");
}

/// Makes a valid, unique XML id: invalid characters become `_`, ids that
/// can't start a name get an `id-` prefix, and repeats get `-2`, `-3`...
fn unique_id(used: &mut HashSet<String>, raw: &str) -> String {
    let mut id: String = raw
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || "_.-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    if !id.starts_with(|c: char| c.is_alphabetic() || c == '_') {
        id.insert_str(0, "id-");
    }
    let mut candidate = id.clone();
    let mut n = 2;
    while !used.insert(candidate.clone()) {
        candidate = format!("{id}-{n}");
        n += 1;
    }
    candidate
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

pub(crate) fn escape(s: &str) -> String {
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

    #[test]
    fn ids_are_valid_and_unique() {
        let mut used = HashSet::new();
        assert_eq!(unique_id(&mut used, "FR-75"), "FR-75");
        assert_eq!(unique_id(&mut used, "FR-75"), "FR-75-2");
        assert_eq!(unique_id(&mut used, "1159106863"), "id-1159106863");
        assert_eq!(unique_id(&mut used, "a b\"c"), "a_b_c");
    }

    #[test]
    fn css_vars_keep_fallbacks() {
        let css = stylesheet(&Theme::default(), true);
        assert!(css.contains(".mg-water{fill:var(--mg-water,#c6ecff)}"));
        let plain = stylesheet(&Theme::default(), false);
        assert!(plain.contains(".mg-water{fill:#c6ecff}"));
    }
}
