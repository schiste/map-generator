//! Query parameters to a `RenderSpec`: kebab-case names map one to one onto
//! the camelCase fields (`css-vars` → `cssVars`), colour slots are
//! `color-<slot>`, lists are comma-separated. The result goes through the
//! same deserializer as the WASM build, so unknown or misspelled
//! parameters are errors there too.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::error::ApiError;

const BOOLS: [&str; 6] = [
    "credit",
    "labels",
    "dissolve",
    "leaders",
    "curvedLabels",
    "cssVars",
];
const INTEGERS: [&str; 4] = ["width", "padding", "precision", "maxInsets"];
const NUMBERS: [&str; 12] = [
    "borderWidth",
    "parentBorderWidth",
    "outlineWidth",
    "contextBorderWidth",
    "disputedBorderWidth",
    "labelSize",
    "labelMinScale",
    "snap",
    "simplify",
    "minArea",
    "margin",
    "centerLon",
];
/// Set by the path, not the query.
const RESERVED: [(&str, &str); 2] = [
    (
        "region",
        "the region is part of the path: /maps/{dataset}/{region}.svg",
    ),
    (
        "format",
        "the format is the path's extension: .svg, .json or .html",
    ),
];

/// Parameters the server handles itself, next to the render spec.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct MapParams {
    pub spec: Map<String, Value>,
    /// `release=`: the dataset release the URL is pinned to.
    pub release: Option<String>,
    /// `worldview=`: a Natural Earth point of view.
    pub worldview: Option<String>,
}

pub fn camel(kebab: &str) -> String {
    let mut out = String::new();
    let mut upper = false;
    for c in kebab.chars() {
        if c == '-' || c == '_' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

pub fn parse(pairs: &[(String, String)]) -> Result<MapParams, ApiError> {
    let mut out = MapParams::default();
    let mut colors = Map::new();
    let mut seen = std::collections::BTreeSet::new();
    for (key, value) in pairs {
        if !seen.insert(key.as_str()) {
            return Err(ApiError::bad_param(key, "given more than once"));
        }
        if let Some((_, why)) = RESERVED.iter().find(|(k, _)| k == key) {
            return Err(ApiError::bad_param(key, why));
        }
        match key.as_str() {
            "release" => out.release = Some(value.clone()),
            "worldview" => out.worldview = Some(value.to_ascii_uppercase()),
            k if k.starts_with("color-") => {
                colors.insert(camel(&k["color-".len()..]), Value::String(value.clone()));
            }
            k => {
                let field = camel(k);
                let v = typed(&field, value).map_err(|why| ApiError::bad_param(k, &why))?;
                out.spec.insert(field, v);
            }
        }
    }
    if !colors.is_empty() {
        out.spec.insert("colors".into(), Value::Object(colors));
    }
    Ok(out)
}

fn typed(field: &str, value: &str) -> Result<Value, String> {
    let number = |v: &str| {
        v.parse::<f64>()
            .ok()
            .filter(|x| x.is_finite())
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .ok_or_else(|| format!("{v:?} is not a number"))
    };
    if BOOLS.contains(&field) {
        return match value {
            "true" | "1" | "" => Ok(Value::Bool(true)),
            "false" | "0" => Ok(Value::Bool(false)),
            v => Err(format!("{v:?} is not true or false")),
        };
    }
    if INTEGERS.contains(&field) {
        return value
            .parse::<u64>()
            .map(|n| Value::Number(n.into()))
            .map_err(|_| format!("{value:?} is not a whole number"));
    }
    if NUMBERS.contains(&field) {
        return number(value);
    }
    match field {
        "languages" => Ok(Value::Array(
            value
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| Value::String(s.to_owned()))
                .collect(),
        )),
        "parallels" => {
            let parts: Vec<Value> = value
                .split(',')
                .map(|p| number(p.trim()))
                .collect::<Result<_, _>>()?;
            if parts.len() != 2 {
                return Err("expects two numbers: south,north".into());
            }
            Ok(Value::Array(parts))
        }
        _ => Ok(Value::String(value.to_owned())),
    }
}

/// A query string with sorted keys and the server's own parameters
/// included, used as the cache key and the canonical URL.
pub fn canonical(params: &MapParams) -> String {
    let mut pairs: BTreeMap<String, String> = BTreeMap::new();
    for (k, v) in &params.spec {
        match (k.as_str(), v) {
            ("colors", Value::Object(c)) => {
                for (slot, colour) in c {
                    pairs.insert(format!("color-{}", kebab(slot)), plain(colour));
                }
            }
            _ => {
                pairs.insert(kebab(k), plain(v));
            }
        }
    }
    if let Some(w) = &params.worldview {
        pairs.insert("worldview".into(), w.clone());
    }
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

fn kebab(camel: &str) -> String {
    let mut out = String::new();
    for c in camel.chars() {
        if c.is_ascii_uppercase() {
            out.push('-');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn plain(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(plain).collect::<Vec<_>>().join(","),
        v => v.to_string(),
    }
}

/// Percent-encodes everything but unreserved characters (RFC 3986).
pub fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~,".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(s: &[(&str, &str)]) -> Result<MapParams, ApiError> {
        parse(
            &s.iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn maps_query_parameters_onto_the_render_spec() {
        let p = q(&[
            ("width", "800"),
            ("labels", "true"),
            ("css-vars", "1"),
            ("languages", "fr, zh-Hant"),
            ("color-water", "#c6ecff"),
            ("color-context-land", "tan"),
            ("label-min-scale", "0.8"),
            ("title", "2024"),
            ("worldview", "ind"),
        ])
        .unwrap();
        let spec: mapgen_spec::RenderSpec =
            serde_json::from_value(Value::Object(p.spec.clone())).unwrap();
        assert_eq!(spec.width, Some(800));
        assert!(spec.labels && spec.css_vars);
        assert_eq!(spec.languages, ["fr", "zh-Hant"]);
        assert_eq!(spec.colors["contextLand"], "tan");
        assert_eq!(spec.title.as_deref(), Some("2024"));
        assert_eq!(p.worldview.as_deref(), Some("IND"));
        assert_eq!(
            canonical(&p),
            "color-context-land=tan&color-water=%23c6ecff&css-vars=true&label-min-scale=0.8&labels=true&languages=fr,zh-Hant&title=2024&width=800&worldview=IND"
        );
    }

    #[test]
    fn rejects_bad_parameters() {
        assert!(q(&[("width", "wide")]).is_err());
        assert!(q(&[("labels", "yes")]).is_err());
        assert!(q(&[("width", "1"), ("width", "2")]).is_err());
        assert!(q(&[("region", "FRA")]).is_err());
        // Unknown names get through here and fail in the spec deserializer.
        let p = q(&[("colour-water", "red")]).unwrap();
        let e =
            serde_json::from_value::<mapgen_spec::RenderSpec>(Value::Object(p.spec)).unwrap_err();
        assert!(e.to_string().contains("colourWater"), "{e}");
    }
}
