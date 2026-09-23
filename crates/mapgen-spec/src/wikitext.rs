//! Wikipedia's `{{Choropleth map}}` template as a recipe: paste the wikitext
//! of a map from an article and get the same regions and view as an empty
//! map. Values and colours (`#faa: Mexico`, `400: BR`, `40%: Chile`) are
//! kept per region for colouring tools (Maphue), not drawn. What has no
//! equivalent is said in `notes`.
//!
//! <https://en.wikipedia.org/wiki/Template:Choropleth_map>

use mapgen_core::frame::BBOX_PRESETS;

use crate::recipe::Recipe;
use crate::{Result, SpecError};

/// The template's default map size (a thumbnail), in pixels.
const TEMPLATE_WIDTH: f64 = 250.0;
const TEMPLATE_HEIGHT: f64 = 200.0;
/// Our default width: template sizes set the shape, not the size.
const WIDTH: f64 = 1000.0;

/// Whether `text` is (or contains) a `{{Choropleth map}}` template call.
pub fn is_choropleth(text: &str) -> bool {
    find_template(text).is_some()
}

/// The recipe for the first `{{Choropleth map}}` in `text`.
pub fn choropleth(text: &str) -> Result<Recipe> {
    let body = find_template(text)
        .ok_or_else(|| SpecError("no {{Choropleth map}} template found".into()))?;
    let params = template_params(body);
    let get = |names: &[&str]| {
        params
            .iter()
            .find(|(k, _)| names.contains(&k.as_str()))
            .map(|(_, v)| v.trim())
            .filter(|v| !v.is_empty())
    };
    let mut recipe = Recipe::default();
    let mut notes = Vec::new();

    // Regions: countries make a map of countries; states, provinces and
    // other entities are single subdivisions (the mixed dataset takes both).
    let (list, dataset) = match (
        get(&["countries"]),
        get(&["entities", "states", "provinces"]),
    ) {
        (Some(c), None) => (c, "ne-admin0"),
        (c, Some(e)) => {
            if c.is_some() {
                notes.push("both `countries` and `entities` given: the template reads only one; both are used".into());
            }
            (e, "mixed")
        }
        (None, None) => {
            return Err(SpecError(
                "the template lists no countries or entities".into(),
            ))
        }
    };
    let lists = [
        get(&["countries"]).filter(|_| dataset == "mixed"),
        Some(list),
    ];
    let mut valued = 0;
    for list in lists.into_iter().flatten() {
        for (value, entity) in entities(list) {
            if let Some(v) = value {
                recipe.values.push((entity.clone(), v));
                valued += 1;
            }
            recipe.regions.push(entity);
        }
    }
    recipe.dataset = Some(dataset.into());
    if valued > 0 {
        notes.push(format!(
            "{valued} values or colours kept per region for colouring (Maphue); the map itself is empty"
        ));
    }

    // Shape and view.
    let px = |name: &str| {
        get(&[name])
            .and_then(|v| v.trim_end_matches("px").trim().parse::<f64>().ok())
            .filter(|v| *v > 0.0)
    };
    let (tw, th) = (px("width"), px("height"));
    let (w, h) = (tw.unwrap_or(TEMPLATE_WIDTH), th.unwrap_or(TEMPLATE_HEIGHT));
    if tw.is_some() || th.is_some() {
        let height = (WIDTH * h / w).round().clamp(50.0, 10_000.0);
        recipe.params.push(("height".into(), format!("{height}")));
    }
    if get(&["width"]).is_some_and(|v| v.eq_ignore_ascii_case("full")) {
        notes.push("width `full` is a page layout: the map is 1000 px wide".into());
    }
    let (lat, lon, zoom) = (
        number(get(&["latitude"])),
        number(get(&["longitude"])),
        number(get(&["zoom"])),
    );
    match (lat, lon, zoom) {
        (Some(lat), Some(lon), Some(zoom)) => {
            // The web map shows 256 px per 360° of longitude at zoom 0.
            let deg_per_px = 360.0 / (256.0 * 2f64.powf(zoom));
            let lon_span = w * deg_per_px;
            let lat_span = h * deg_per_px * lat.to_radians().cos().abs().max(0.05);
            if lon_span >= 360.0 {
                recipe.params.push(("frame".into(), "world".into()));
            } else {
                let bbox = [
                    lon - lon_span / 2.0,
                    (lat - lat_span / 2.0).max(-89.0),
                    lon + lon_span / 2.0,
                    (lat + lat_span / 2.0).min(89.0),
                ];
                let bbox: Vec<String> = bbox.iter().map(|v| format!("{:.2}", v)).collect();
                recipe.params.push(("bbox".into(), bbox.join(";")));
            }
        }
        (Some(_), Some(_), None) | (Some(_), None, _) | (None, Some(_), _) => {
            notes.push("`latitude`/`longitude` need all three of latitude, longitude and zoom: the map is framed on the regions".into());
        }
        _ => {}
    }
    if let Some(view) = get(&["view"]) {
        let key = view.to_lowercase().replace([' ', '_'], "-");
        let preset = match key.as_str() {
            "central-america" | "latin-america" => None,
            k => BBOX_PRESETS
                .iter()
                .find(|(name, _)| *name == k)
                .map(|(name, _)| *name),
        };
        if lat.is_some() && lon.is_some() && zoom.is_some() {
            // latitude/longitude/zoom win, as in the template.
        } else if let Some(name) = preset {
            recipe.params.push(("bbox".into(), name.into()));
        } else if !key.eq_ignore_ascii_case("auto") {
            notes.push(format!("view `{view}`: the map is framed on the regions"));
        }
    }
    if zoom.is_some() && (lat.is_none() || lon.is_none()) {
        notes
            .push("`zoom` without latitude and longitude: the map is framed on the regions".into());
    }

    // Text.
    if let Some(caption) = get(&["caption"]).map(plain_text).filter(|c| !c.is_empty()) {
        recipe.params.push(("caption".into(), caption));
    }
    if let Some(alt) = get(&["alt"]).map(plain_text).filter(|a| !a.is_empty()) {
        recipe.params.push(("alt".into(), alt));
    }

    // What has no equivalent in an empty map.
    let colouring: Vec<&str> = ["color", "min-opacity", "logarithmic-scale", "legends"]
        .into_iter()
        .filter(|k| get(&[k]).is_some())
        .collect();
    if !colouring.is_empty() {
        notes.push(format!(
            "{}: colouring settings, for Maphue",
            colouring.join(", ")
        ));
    }
    let layout: Vec<&str> = ["align", "frameless"]
        .into_iter()
        .filter(|k| get(&[k]).is_some())
        .collect();
    if !layout.is_empty() {
        notes.push(format!(
            "{}: page layout, not part of the map",
            layout.join(", ")
        ));
    }
    if let Some(style) = get(&["mapstyle"]) {
        notes.push(format!(
            "mapstyle `{style}`: the base map is Natural Earth's; add region names or neighbour names in the settings"
        ));
    }
    if let Some(source) = get(&["source"]).filter(|s| !s.eq_ignore_ascii_case("ChoroplethMap.map"))
    {
        notes.push(format!(
            "source `{source}`: this recipe uses Natural Earth boundaries (the playground's File mode loads Commons map pages)"
        ));
    }
    let known = [
        "countries",
        "entities",
        "states",
        "provinces",
        "color",
        "min-opacity",
        "logarithmic-scale",
        "view",
        "latitude",
        "longitude",
        "zoom",
        "width",
        "height",
        "align",
        "frameless",
        "mapstyle",
        "alt",
        "caption",
        "legends",
        "source",
    ];
    for (key, _) in &params {
        if !known.contains(&key.as_str()) {
            notes.push(format!("unknown template parameter `{key}` ignored"));
        }
    }
    recipe.notes = notes;
    recipe.spec().map(|_| recipe)
}

/// The text between `{{Choropleth map` and its closing `}}`.
fn find_template(text: &str) -> Option<&str> {
    let mut from = 0;
    while let Some(i) = text[from..].find("{{") {
        let start = from + i + 2;
        let rest = &text[start..];
        let name_end = rest.find(['|', '}']).unwrap_or(rest.len());
        let name = rest[..name_end].trim().replace('_', " ");
        let name = name.strip_prefix("Template:").unwrap_or(&name);
        if name.eq_ignore_ascii_case("choropleth map") {
            // Find the matching `}}`, skipping nested templates.
            let bytes = rest.as_bytes();
            let (mut depth, mut i) = (1usize, 0usize);
            // Braces are ASCII, so byte positions are safe to slice at.
            while i + 1 < bytes.len() {
                if &bytes[i..i + 2] == b"{{" {
                    depth += 1;
                    i += 2;
                } else if &bytes[i..i + 2] == b"}}" {
                    depth -= 1;
                    if depth == 0 {
                        return Some(&rest[name_end..i]);
                    }
                    i += 2;
                } else {
                    i += 1;
                }
            }
            return Some(&rest[name_end..]);
        }
        from = start;
    }
    None
}

/// `| key = value` parameters, split on top-level pipes (not those inside
/// nested templates or links). Keys are lowercase; positional ones dropped.
fn template_params(body: &str) -> Vec<(String, String)> {
    let mut parts = Vec::new();
    let (mut depth, mut current) = (0i32, String::new());
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        let pair = chars.peek().copied();
        match (c, pair) {
            ('{', Some('{')) | ('[', Some('[')) => {
                depth += 1;
                current.push(c);
                current.push(chars.next().unwrap_or(c));
            }
            ('}', Some('}')) | (']', Some(']')) => {
                depth -= 1;
                current.push(c);
                current.push(chars.next().unwrap_or(c));
            }
            ('|', _) if depth == 0 => parts.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    parts.push(current);
    parts
        .into_iter()
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            let key = k.trim().to_lowercase();
            (!key.is_empty()).then(|| (key, v.to_owned()))
        })
        .collect()
}

/// Entities of a list (semicolons or lines), each line optionally led by a
/// value or colour for all its entities: `#faa: Mexico; Brazil`, `40%: Chile`.
fn entities(list: &str) -> Vec<(Option<String>, String)> {
    let mut out = Vec::new();
    for line in list.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (value, rest) = match line.split_once(':') {
            Some((v, rest)) if is_value(v.trim()) => (Some(v.trim().to_owned()), rest),
            _ => (None, line),
        };
        for entity in rest.split(';').map(plain_text).filter(|e| !e.is_empty()) {
            out.push((value.clone(), entity));
        }
    }
    out
}

/// `#faa`, `#D8757E`, `400`, `285.7`, `40%`.
fn is_value(v: &str) -> bool {
    let hex = v.strip_prefix('#').is_some_and(|h| {
        matches!(h.len(), 3 | 4 | 6 | 8) && h.chars().all(|c| c.is_ascii_hexdigit())
    });
    let number = v
        .strip_suffix('%')
        .unwrap_or(v)
        .replace(',', "")
        .parse::<f64>()
        .is_ok();
    hex || number
}

fn number(v: Option<&str>) -> Option<f64> {
    v?.trim().parse::<f64>().ok().filter(|x| x.is_finite())
}

/// Wikitext to plain text: `[[a|b]]` → b, `[[a]]` → a, templates,
/// footnotes and HTML tags dropped, bold and italics marks removed.
fn plain_text(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("[[") {
            let end = r.find("]]").unwrap_or(r.len());
            let link = &r[..end];
            out.push_str(link.rsplit('|').next().unwrap_or(link));
            rest = r.get(end + 2..).unwrap_or("");
        } else if let Some(r) = rest.strip_prefix("{{") {
            // Skip a (possibly nested) template.
            let mut depth = 1;
            let mut i = 0;
            while i < r.len() && depth > 0 {
                if r[i..].starts_with("{{") {
                    depth += 1;
                    i += 2;
                } else if r[i..].starts_with("}}") {
                    depth -= 1;
                    i += 2;
                } else {
                    i += r[i..].chars().next().map_or(1, char::len_utf8);
                }
            }
            rest = &r[i.min(r.len())..];
        } else if rest.starts_with("<ref") {
            // A footnote: dropped with its content.
            let tag_end = rest.find('>').map_or(rest.len(), |e| e + 1);
            rest = if rest[..tag_end].ends_with("/>") {
                &rest[tag_end..]
            } else {
                rest.find("</ref>")
                    .map_or("", |e| &rest[e + "</ref>".len()..])
            };
        } else if rest.starts_with('<') {
            let end = rest.find('>').map_or(rest.len(), |e| e + 1);
            rest = &rest[end..];
        } else {
            let c = rest.chars().next().unwrap_or(' ');
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out.replace("'''", "")
        .replace("''", "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// `New York (state)` → `New York`: a Wikipedia title's disambiguation.
pub fn without_disambiguation(title: &str) -> Option<&str> {
    let t = title.trim();
    let open = t.rfind(" (")?;
    t.ends_with(')')
        .then(|| t[..open].trim())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_list_of_countries() {
        let r = choropleth(
            "{{Choropleth map\n| countries = Brazil; Mexico; Egypt; China; Australia\n}}",
        )
        .unwrap();
        assert_eq!(r.dataset.as_deref(), Some("ne-admin0"));
        assert_eq!(
            r.regions,
            ["Brazil", "Mexico", "Egypt", "China", "Australia"]
        );
        assert!(r.values.is_empty());
        assert!(r.notes.is_empty(), "{:?}", r.notes);
    }

    #[test]
    fn values_colours_and_views() {
        let r = choropleth(
            "Some article text.\n{{Choropleth map\n| view = South America\n| height = 300\n| color = #00f\n| countries =\n400: BR\n300: UY; CL; PE\n#aff: SR; GY; GF\n}}\nMore text.",
        )
        .unwrap();
        assert_eq!(r.regions, ["BR", "UY", "CL", "PE", "SR", "GY", "GF"]);
        assert_eq!(r.values[0], ("BR".to_owned(), "400".to_owned()));
        assert_eq!(r.values[6], ("GF".to_owned(), "#aff".to_owned()));
        let spec = r.spec().unwrap();
        assert_eq!(spec.bbox.as_deref(), Some("south-america"));
        assert_eq!(spec.height, Some(1200), "250×300 → 1000×1200");
        assert!(r.notes.iter().any(|n| n.contains("color: colouring")));
        assert!(r
            .notes
            .iter()
            .any(|n| n.contains("7 values or colours kept")));
    }

    #[test]
    fn entities_titles_and_coordinates() {
        let r = choropleth(
            "{{Choropleth map\n| view = United States\n| zoom = 2\n| latitude = 45\n| longitude = -103\n| states = California; Texas; [[New York (state)|New York]]\n| caption = Three '''big''' states<ref>A source</ref>{{Legend|#f00|Colored}}\n| align = left\n}}",
        )
        .unwrap();
        assert_eq!(r.dataset.as_deref(), Some("mixed"));
        assert_eq!(r.regions, ["California", "Texas", "New York"]);
        let spec = r.spec().unwrap();
        // 250 px at zoom 2: 87.9° of longitude around −103.
        let b: Vec<f64> = spec
            .bbox
            .as_deref()
            .unwrap()
            .split(',')
            .map(|v| v.parse().unwrap())
            .collect();
        assert!(
            (b[0] + 146.95).abs() < 0.01 && (b[2] + 59.05).abs() < 0.01,
            "{b:?}"
        );
        assert!(b[1] < 45.0 && b[3] > 45.0);
        assert_eq!(spec.caption.as_deref(), Some("Three big states"));
        assert!(r.notes.iter().any(|n| n.contains("align: page layout")));
        assert_eq!(without_disambiguation("New York (state)"), Some("New York"));
        assert_eq!(
            without_disambiguation("Nord (French department)"),
            Some("Nord")
        );
        assert_eq!(without_disambiguation("France"), None);
    }

    #[test]
    fn accents_and_other_sources() {
        let r = choropleth(
            "{{Choropleth map\n| source = France Departments.map\n| entities = Finistère; Nord (French department); Pyrénées-Atlantiques\n}}",
        )
        .unwrap();
        assert_eq!(
            r.regions,
            [
                "Finistère",
                "Nord (French department)",
                "Pyrénées-Atlantiques"
            ]
        );
        assert!(r.notes.iter().any(|n| n.contains("France Departments.map")));
    }

    #[test]
    fn not_a_template() {
        assert!(!is_choropleth("key,value\ndataset,countries\n"));
        assert!(is_choropleth("{{ choropleth_map |countries=FR}}"));
        assert!(choropleth("{{Choropleth map | height = 300 }}").is_err());
    }
}
