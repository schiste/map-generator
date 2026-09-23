use std::collections::HashSet;
use std::fmt::Write;

use geo_types::{LineString, MultiPolygon};

use crate::feature::MapFeature;
use crate::labels::{Label, LabelShape};
use crate::panel::{BorderKind, Panel};
use crate::theme::Theme;

/// Affine transform from projected metres to SVG pixels (y axis flipped).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub min_x: f64,
    pub max_y: f64,
    pub scale: f64,
    /// Pixel position of the frame's top-left corner.
    pub left: f64,
    pub top: f64,
    /// Height of the whole canvas (main panel only).
    pub canvas_height: f64,
}

impl Viewport {
    pub fn to_px(self, x: f64, y: f64) -> (f64, f64) {
        (
            self.left + (x - self.min_x) * self.scale,
            self.top + (self.max_y - y) * self.scale,
        )
    }
}

/// Everything the writer needs: the canvas and its panels (main map first,
/// then insets), all geometry already projected.
#[derive(Debug, Clone, Copy)]
pub struct SvgDocument<'a> {
    pub width: u32,
    pub height: u32,
    pub panels: &'a [Panel],
    pub theme: &'a Theme,
    pub title: Option<&'a str>,
    pub attribution: Option<&'a str>,
    /// Boundary version, as `data-boundary-year` / `data-source-release`
    /// on the root element.
    pub boundary_year: Option<&'a str>,
    pub source_release: Option<&'a str>,
    /// Draw the attribution in the bottom-right corner.
    pub credit: bool,
    /// Decimal places kept for coordinates.
    pub precision: usize,
    /// Emit colours as `var(--mg-<slot>, <colour>)` so a page embedding the
    /// SVG inline can restyle it with CSS custom properties.
    pub css_vars: bool,
    /// Region fills stroke their own outlines (`BorderMode::Regions`).
    pub region_strokes: bool,
}

/// Serialises a map into an SVG document.
///
/// Main-map layers, bottom to top: `#background`, `#water`, `#context`,
/// `#context-borders`, `#land`, `#lakes`, `#borders`, `#labels`; then one
/// `g.mg-inset` per inset with the same layers (as classes). Colours live in
/// the `<style>` block only; region fills have no stroke, and every border is
/// drawn once, in `#borders`, by kind.
pub fn write_svg(doc: &SvgDocument) -> String {
    let (w, h) = (doc.width, doc.height);
    let mut out = String::new();
    let mut ids = HashSet::new();

    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let version: String = [
        ("data-boundary-year", doc.boundary_year),
        ("data-source-release", doc.source_release),
    ]
    .iter()
    .filter_map(|(k, v)| v.map(|v| format!(" {k}=\"{}\"", escape(v))))
    .collect();
    let _ = writeln!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\"{version}>"
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
        stylesheet(doc.theme, doc.css_vars, doc.region_strokes)
    );
    let _ = writeln!(
        out,
        "<rect id=\"background\" class=\"mg-background\" width=\"{w}\" height=\"{h}\"/>"
    );
    for (i, panel) in doc.panels.iter().enumerate() {
        write_panel(&mut out, &mut ids, panel, i, doc);
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

fn write_panel(
    out: &mut String,
    ids: &mut HashSet<String>,
    panel: &Panel,
    index: usize,
    doc: &SvgDocument,
) {
    let vp = panel.viewport;
    let p = doc.precision;
    // The main map's layers have ids; insets use classes (ids must be unique).
    let main = index == 0;
    let group = |name: &str| {
        if main {
            format!("id=\"{name}\"")
        } else {
            format!("class=\"mg-{name}\"")
        }
    };
    if !main {
        let _ = writeln!(out, "<g id=\"inset-{index}\" class=\"mg-inset\">");
    }
    let water = path_data(&panel.water, vp, p);
    if !water.is_empty() {
        let id = if main { "id=\"water\" " } else { "" };
        let _ = writeln!(out, "<path {id}class=\"mg-water\" d=\"{water}\"/>");
    }
    write_layer(
        out,
        ids,
        &group("context"),
        "mg-context",
        "",
        &panel.context,
        vp,
        p,
    );
    write_borders(
        out,
        &group("context-borders"),
        panel,
        |k| k == BorderKind::Context,
        vp,
        p,
    );
    write_layer(
        out,
        ids,
        &group("land"),
        "mg-land",
        "",
        &panel.subject,
        vp,
        p,
    );
    write_layer(
        out,
        ids,
        &group("lakes"),
        "mg-lake",
        "lake-",
        &panel.lakes,
        vp,
        p,
    );
    write_borders(
        out,
        &group("borders"),
        panel,
        |k| k != BorderKind::Context,
        vp,
        p,
    );
    write_labels(
        out,
        ids,
        &group("labels"),
        &panel.labels,
        &panel.subject,
        doc.theme.label_size,
    );
    if let Some(b) = panel.inset_box {
        let _ = writeln!(
            out,
            "<rect class=\"mg-inset-frame\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/>",
            fmt_num(b.min().x, 1),
            fmt_num(b.min().y, 1),
            fmt_num(b.width(), 1),
            fmt_num(b.height(), 1)
        );
    }
    if !main {
        out.push_str("</g>\n");
    }
}

/// The `<style>` contents: every colour of the map is defined here.
/// With `region_strokes`, region fills carry their own stroke (see
/// `BorderMode::Regions`).
pub fn stylesheet(theme: &Theme, css_vars: bool, region_strokes: bool) -> String {
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
    let n = |v: f64| fmt_num(v, 3);
    format!(
        "path{{fill-rule:evenodd}}\n\
         .mg-background{{fill:{bg}}}\n\
         .mg-water{{fill:{water}}}\n\
         .mg-context{{fill:{ctx}{ctx_stroke}}}\n\
         .mg-land{{fill:{land}{land_stroke}}}\n\
         .mg-lake{{fill:{water};stroke:{lakeb};stroke-width:{cbw}}}\n\
         .mg-border{{fill:none;stroke-linejoin:round;stroke-linecap:round}}\n\
         .mg-border-context{{stroke:{ctxb};stroke-width:{cbw}}}\n\
         .mg-border-internal{{stroke:{border};stroke-width:{bw}}}\n\
         .mg-border-parent{{stroke:{border};stroke-width:{pbw}}}\n\
         .mg-border-outline{{stroke:{outline};stroke-width:{ow}}}\n\
         .mg-border-coast{{stroke:{coast}}}\n\
         .mg-border-disputed{{stroke:{disputed};stroke-width:{dw};stroke-dasharray:{dash1} {dash2}}}\n\
         .mg-label{{fill:{label};font:{ls}px sans-serif;text-anchor:middle;dominant-baseline:central;\
         paint-order:stroke;stroke:{land};stroke-width:2.5px;stroke-linejoin:round}}\n\
         .mg-leader{{fill:none;stroke:{label};stroke-width:0.6}}\n\
         .mg-inset-frame{{fill:none;stroke:{outline};stroke-width:1}}\n\
         .mg-credit{{fill:{label};font:9px sans-serif;text-anchor:end;opacity:.75}}\n",
        bg = c("background"),
        water = c("water"),
        ctx = c("context-land"),
        ctxb = c("context-border"),
        land = c("land"),
        border = c("border"),
        outline = c("outline"),
        coast = c("coast"),
        ctx_stroke = if region_strokes {
            format!(";stroke:{};stroke-width:{};stroke-linejoin:round", c("context-border"), n(theme.context_border_width))
        } else {
            String::new()
        },
        land_stroke = if region_strokes {
            format!(";stroke:{};stroke-width:{};stroke-linejoin:round", c("border"), n(theme.border_width))
        } else {
            String::new()
        },
        lakeb = c("lake-border"),
        disputed = c("disputed-border"),
        label = c("label"),
        bw = n(theme.border_width),
        pbw = n(theme.parent_border_width),
        ow = n(theme.outline_width),
        cbw = n(theme.context_border_width),
        dw = n(theme.disputed_border_width),
        dash1 = n(4.0 * theme.disputed_border_width),
        dash2 = n(2.5 * theme.disputed_border_width),
        ls = fmt_num(theme.label_size, 2),
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
    let _ = writeln!(out, "<g {group}>");
    for (f, d) in paths {
        let id = unique_id(ids, &format!("{id_prefix}{}", f.id));
        let _ = writeln!(out, "{}", path_element(&id, layer_class, f, &d));
    }
    out.push_str("</g>\n");
}

/// A region's `<path>`: `id` (XML-safe, unique), classes (layer, feature
/// class and lowercase ISO-2 country, which Maphue uses to colour by
/// country), `data-name`, `data-code` (the raw code, since `id` may have been
/// sanitised or de-duplicated), `data-parent`, and a `<title>` tooltip that
/// names the parent ("Lancaster, Nebraska").
fn path_element(id: &str, layer_class: &str, f: &MapFeature, d: &str) -> String {
    let mut classes = vec![layer_class];
    if !f.class.is_empty() {
        classes.push(&f.class);
    }
    if let Some(c) = f.country.as_deref().filter(|c| *c != f.class) {
        classes.push(c);
    }
    let parent = f
        .parent
        .as_deref()
        .map(|p| format!(" data-parent=\"{}\"", escape(p)))
        .unwrap_or_default();
    let title = match &f.parent_name {
        Some(p) if *p != f.name => format!("{}, {p}", f.name),
        _ => f.name.clone(),
    };
    format!(
        "<path id=\"{}\" class=\"{}\" data-name=\"{}\" data-code=\"{}\"{parent} d=\"{d}\"><title>{}</title></path>",
        escape(id),
        escape(&classes.join(" ")),
        escape(&f.name),
        escape(&f.id),
        escape(&title),
    )
}

/// One `<path>` per border kind, each border segment drawn once.
fn write_borders(
    out: &mut String,
    group: &str,
    panel: &Panel,
    select: impl Fn(BorderKind) -> bool,
    vp: Viewport,
    precision: usize,
) {
    let paths: Vec<(BorderKind, String)> = panel
        .borders
        .iter()
        .filter(|(k, _)| select(*k))
        .map(|(k, lines)| {
            let mut d = String::new();
            for l in lines {
                line_data(&mut d, l, vp, precision);
            }
            (*k, d)
        })
        .filter(|(_, d)| !d.is_empty())
        .collect();
    if paths.is_empty() {
        return;
    }
    let _ = writeln!(out, "<g {group}>");
    for (k, d) in paths {
        let _ = writeln!(out, "<path class=\"mg-border {}\" d=\"{d}\"/>", k.class());
    }
    out.push_str("</g>\n");
}

fn write_labels(
    out: &mut String,
    ids: &mut HashSet<String>,
    group: &str,
    labels: &[Label],
    subject: &[MapFeature],
    nominal: f64,
) {
    if labels.is_empty() {
        return;
    }
    let _ = writeln!(out, "<g {group}>");
    for l in labels {
        // The stylesheet sets the nominal size; shrunk labels override it.
        let style = if (l.size - nominal).abs() > 1e-9 {
            format!(" style=\"font-size:{}px\"", fmt_num(l.size, 2))
        } else {
            String::new()
        };
        let text = escape(&l.text);
        match &l.shape {
            LabelShape::Straight => {
                let _ = writeln!(
                    out,
                    "<text class=\"mg-label\" x=\"{}\" y=\"{}\"{style}>{text}</text>",
                    fmt_num(l.x, 1),
                    fmt_num(l.y, 1)
                );
            }
            LabelShape::Leader { anchor, end } => {
                let _ = writeln!(
                    out,
                    "<path class=\"mg-leader\" d=\"M{} {}L{} {}\"/>",
                    fmt_num(anchor.0, 1),
                    fmt_num(anchor.1, 1),
                    fmt_num(end.0, 1),
                    fmt_num(end.1, 1)
                );
                let _ = writeln!(
                    out,
                    "<text class=\"mg-label\" x=\"{}\" y=\"{}\"{style}>{text}</text>",
                    fmt_num(l.x, 1),
                    fmt_num(l.y, 1)
                );
            }
            LabelShape::Curved { path } => {
                let base = subject.get(l.feature).map_or("label", |f| f.id.as_str());
                let id = unique_id(ids, &format!("label-path-{base}"));
                let mut d = String::new();
                for (i, (x, y)) in path.iter().enumerate() {
                    let _ = write!(
                        d,
                        "{}{} {}",
                        if i == 0 { "M" } else { "L" },
                        fmt_num(*x, 1),
                        fmt_num(*y, 1)
                    );
                }
                let _ = writeln!(
                    out,
                    "<path id=\"{}\" fill=\"none\" d=\"{d}\"/>",
                    escape(&id)
                );
                let _ = writeln!(
                    out,
                    "<text class=\"mg-label\"{style}><textPath href=\"#{}\" startOffset=\"50%\">{text}</textPath></text>",
                    escape(&id)
                );
            }
        }
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

fn line_data(d: &mut String, line: &LineString<f64>, vp: Viewport, precision: usize) {
    let mut pts: Vec<(String, String)> = Vec::with_capacity(line.0.len());
    for c in line.0.iter() {
        let (x, y) = vp.to_px(c.x, c.y);
        let p = (fmt_num(x, precision), fmt_num(y, precision));
        if pts.last() != Some(&p) {
            pts.push(p);
        }
    }
    if pts.len() < 2 {
        return;
    }
    for (i, (x, y)) in pts.iter().enumerate() {
        let _ = write!(d, "{}{x} {y}", if i == 0 { "M" } else { "L" });
    }
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

    fn lancaster() -> MapFeature {
        MapFeature {
            id: "US-31109".into(),
            name: "Lancaster".into(),
            class: "subdivision".into(),
            parent: Some("US-31".into()),
            parent_name: Some("Nebraska".into()),
            country: Some("us".into()),
            ..MapFeature::default()
        }
    }

    #[test]
    fn paths_carry_codes_parent_and_country() {
        let p = path_element("US-31109", "mg-land", &lancaster(), "M0 0Z");
        assert_eq!(
            p,
            "<path id=\"US-31109\" class=\"mg-land subdivision us\" data-name=\"Lancaster\" \
             data-code=\"US-31109\" data-parent=\"US-31\" d=\"M0 0Z\"><title>Lancaster, Nebraska</title></path>"
        );
        let bare = MapFeature {
            id: "x".into(),
            name: "X".into(),
            ..MapFeature::default()
        };
        assert_eq!(
            path_element("id-x", "mg-land", &bare, "M0 0Z"),
            "<path id=\"id-x\" class=\"mg-land\" data-name=\"X\" data-code=\"x\" d=\"M0 0Z\"><title>X</title></path>"
        );
    }

    /// Maphue finds a country's shapes with
    /// `[class~="fr"], [id="fr"], [id="FR"]`; the ISO-2 class must be a
    /// whitespace-separated token of `class`.
    #[test]
    fn maphue_finds_country_shapes() {
        let p = path_element(
            "FR-75",
            "mg-land",
            &MapFeature {
                id: "FR-75".into(),
                name: "Paris".into(),
                class: "subdivision".into(),
                country: Some("fr".into()),
                ..MapFeature::default()
            },
            "M0 0Z",
        );
        let class = p
            .split("class=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        assert!(class.split_whitespace().any(|t| t == "fr"), "{class}");
        assert!(!class.split_whitespace().any(|t| t == "FR-75"));
    }

    #[test]
    fn css_vars_keep_fallbacks() {
        let css = stylesheet(&Theme::default(), true, false);
        assert!(css.contains(".mg-water{fill:var(--mg-water,#c6ecff)}"));
        let plain = stylesheet(&Theme::default(), false, false);
        assert!(plain.contains(".mg-land{fill:#fefee9}"));
        assert!(plain.contains(".mg-border-coast{stroke:#0978ab}"));
        let stroked = stylesheet(&Theme::default(), false, true);
        assert!(stroked.contains(
            ".mg-land{fill:#fefee9;stroke:#646464;stroke-width:0.5;stroke-linejoin:round}"
        ));
        assert!(plain.contains(".mg-water{fill:#c6ecff}"));
    }
}
