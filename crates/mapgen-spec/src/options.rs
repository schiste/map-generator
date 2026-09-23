//! Every render option, described for clients that build settings forms
//! (`GET /api/v1/render-options`, `renderOptions()` in WASM): wire name,
//! type, label, help, widget, default, choices, limits, group, order and
//! when it applies. The API's parameter parser types values from this same
//! table, and tests check it against `RenderSpec` and its validation, so the
//! description can't drift from what the API accepts.

use mapgen_core::{RenderOptions, Theme};
use serde::Serialize;
use serde_json::{json, Value};

/// Version of the description's shape; additive changes keep it.
pub const VERSION: u32 = 1;

/// How a value is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Boolean,
    Integer,
    Number,
    String,
    /// Strings, comma-separated in URLs (`;` in recipes).
    List,
    /// Two numbers, comma-separated.
    Pair,
}

/// The control a client should show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Widget {
    Checkbox,
    Select,
    Number,
    Text,
    /// Free text that may be long (captions, descriptions).
    Textarea,
    /// A list of tokens, with `suggestions`.
    Tokens,
    /// `west,south,east,north`, or a preset name from `choicesSource`.
    Bbox,
    /// Two numbers.
    Pair,
}

/// When an option applies (shown or enabled).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Condition {
    /// `{"option": "labels", "equals": true}`
    Equals { option: &'static str, equals: Value },
    /// `{"option": "projection", "in": ["albers", "lcc"]}`
    In {
        option: &'static str,
        r#in: Vec<Value>,
    },
    /// `{"option": "title", "notEmpty": true}`
    NotEmpty {
        option: &'static str,
        #[serde(rename = "notEmpty")]
        not_empty: bool,
    },
    /// `{"anyOf": [...]}`
    AnyOf {
        #[serde(rename = "anyOf")]
        any_of: Vec<Condition>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Choice {
    pub value: Value,
    pub label: &'static str,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderOption {
    /// Query parameter and recipe key (kebab-case).
    pub name: &'static str,
    /// `RenderSpec` member (camelCase), as the WASM API and `POST /render` take it.
    pub key: String,
    /// False for options of the map endpoint and recipes that aren't
    /// `RenderSpec` members (`worldview`).
    pub in_render_spec: bool,
    #[serde(rename = "type")]
    pub kind: Kind,
    pub label: &'static str,
    pub description: &'static str,
    pub widget: Widget,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    /// The only values accepted.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<Choice>,
    /// An endpoint listing the values (authoritative over `choices`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub choices_source: Option<&'static str>,
    /// Common values; others are accepted too.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub suggestions: Vec<Choice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclusive_minimum: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum: Option<f64>,
    /// A sensible step for number inputs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,
    pub group: &'static str,
    pub order: u32,
    /// Belongs in a collapsed "advanced" part of the form.
    pub advanced: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visible_when: Option<Condition>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: &'static str,
    pub label: &'static str,
    pub order: u32,
}

/// A colour slot: `color-<slot>` in URLs, `colors.<key>` in a `RenderSpec`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorSlot {
    pub slot: &'static str,
    pub name: String,
    pub key: String,
    pub label: &'static str,
    pub description: &'static str,
    /// In the default theme; each theme's are at `themesSource`.
    pub default: String,
}

/// A parameter the map endpoint takes that is not a map setting.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Excluded {
    pub name: &'static str,
    pub reason: &'static str,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Description {
    pub version: u32,
    pub groups: Vec<Group>,
    pub options: Vec<RenderOption>,
    pub color_slots: Vec<ColorSlot>,
    /// Theme colours for every slot.
    pub themes_source: &'static str,
    pub excluded: Vec<Excluded>,
}

pub const GROUPS: [Group; 7] = [
    Group {
        id: "map",
        label: "Map",
        order: 10,
    },
    Group {
        id: "labels",
        label: "Labels and places",
        order: 20,
    },
    Group {
        id: "frame",
        label: "Frame and projection",
        order: 30,
    },
    Group {
        id: "borders",
        label: "Borders",
        order: 40,
    },
    Group {
        id: "colors",
        label: "Colours",
        order: 50,
    },
    Group {
        id: "credit",
        label: "Credit",
        order: 60,
    },
    Group {
        id: "output",
        label: "Output",
        order: 70,
    },
];

/// The languages Natural Earth names places in: suggestions for `languages`.
pub const LANGUAGE_SUGGESTIONS: [(&str, &str); 26] = [
    ("ar", "Arabic"),
    ("bn", "Bengali"),
    ("de", "German"),
    ("el", "Greek"),
    ("en", "English"),
    ("es", "Spanish"),
    ("fa", "Persian"),
    ("fr", "French"),
    ("he", "Hebrew"),
    ("hi", "Hindi"),
    ("hu", "Hungarian"),
    ("id", "Indonesian"),
    ("it", "Italian"),
    ("ja", "Japanese"),
    ("ko", "Korean"),
    ("nl", "Dutch"),
    ("pl", "Polish"),
    ("pt", "Portuguese"),
    ("ru", "Russian"),
    ("sv", "Swedish"),
    ("tr", "Turkish"),
    ("uk", "Ukrainian"),
    ("ur", "Urdu"),
    ("vi", "Vietnamese"),
    ("zh-Hans", "Chinese (simplified)"),
    ("zh-Hant", "Chinese (traditional)"),
];

const SLOT_TEXT: [(&str, &str, &str); 11] = [
    (
        "background",
        "Background",
        "Behind everything, outside the map",
    ),
    ("water", "Water", "Seas and oceans"),
    ("land", "Mapped regions", "The regions the map is about"),
    (
        "context-land",
        "Neighbours",
        "Countries around the mapped regions",
    ),
    ("border", "Borders", "Borders between mapped regions"),
    ("outline", "Outline", "The outer edge of the mapped regions"),
    ("coast", "Coast", "Where mapped regions meet water"),
    (
        "context-border",
        "Neighbour borders",
        "Borders between neighbouring countries",
    ),
    ("lake-border", "Lake shores", "Outlines of lakes"),
    (
        "disputed-border",
        "Disputed borders",
        "Disputed and claimed boundaries, dashed",
    ),
    ("label", "Labels", "Names, the title, caption and credit"),
];

/// Parameters of the map endpoint that are not map settings.
pub const EXCLUDED: [Excluded; 5] = [
    Excluded {
        name: "region",
        reason: "part of the path: /maps/{dataset}/{region}.svg",
    },
    Excluded {
        name: "regions",
        reason: "part of the path: /maps/{dataset}/FRA,DEU.svg",
    },
    Excluded {
        name: "format",
        reason: "the path's extension: .svg, .json or .html",
    },
    Excluded {
        name: "release",
        reason: "pins the URL to a dataset release (see /datasets); not a map setting",
    },
    Excluded {
        name: "dissolve",
        reason: "needs a data-unit table, which only the WASM API and the CLI take",
    },
];

fn choices(values: &[(&str, &'static str)]) -> Vec<Choice> {
    values
        .iter()
        .map(|(v, label)| Choice {
            value: json!(v),
            label,
        })
        .collect()
}

fn equals(option: &'static str, value: Value) -> Condition {
    Condition::Equals {
        option,
        equals: value,
    }
}

/// Labels of any kind are drawn: size and placement options apply.
fn any_labels() -> Condition {
    Condition::AnyOf {
        any_of: vec![
            equals("labels", json!(true)),
            equals("context-labels", json!(true)),
            Condition::In {
                option: "capitals",
                r#in: vec![json!("countries"), json!("all")],
            },
        ],
    }
}

/// Every render option. `max_width` is the host's width limit.
pub fn options(max_width: u32) -> Vec<RenderOption> {
    let d = RenderOptions::default();
    let t = Theme::default();
    let o = |name: &'static str, kind: Kind, widget: Widget, group: &'static str, order: u32| {
        RenderOption {
            name,
            key: crate::params::camel(name),
            in_render_spec: true,
            kind,
            label: "",
            description: "",
            widget,
            default: None,
            choices: Vec::new(),
            choices_source: None,
            suggestions: Vec::new(),
            minimum: None,
            exclusive_minimum: None,
            maximum: None,
            step: None,
            max_length: None,
            group,
            order,
            advanced: false,
            visible_when: None,
        }
    };
    use Kind::*;
    use Widget as W;
    let width = |name, label, description, default: f64, order| RenderOption {
        label,
        description,
        default: Some(json!(default)),
        exclusive_minimum: Some(0.0),
        step: Some(0.1),
        advanced: true,
        ..o(name, Number, W::Number, "borders", order)
    };
    vec![
        // Map
        RenderOption {
            label: "Title",
            description: "The document's title; drawn on the map with show-title.",
            max_length: Some(200),
            ..o("title", String, W::Text, "map", 10)
        },
        RenderOption {
            label: "Title on the map",
            description: "Draw the title in a band above the map.",
            default: Some(json!(d.show_title)),
            visible_when: Some(Condition::NotEmpty { option: "title", not_empty: true }),
            ..o("show-title", Boolean, W::Checkbox, "map", 20)
        },
        RenderOption {
            label: "Caption",
            description: "Text drawn under the map, wrapped to its width.",
            max_length: Some(1000),
            ..o("caption", String, W::Textarea, "map", 30)
        },
        RenderOption {
            label: "Description for screen readers",
            description: "What the map shows, for people who can't see it (<desc>).",
            max_length: Some(1000),
            ..o("alt", String, W::Textarea, "map", 40)
        },
        RenderOption {
            label: "Width",
            description: "Width in pixels.",
            default: Some(json!(d.width)),
            minimum: Some(16.0),
            maximum: Some(f64::from(max_width)),
            step: Some(10.0),
            ..o("width", Integer, W::Number, "map", 50)
        },
        RenderOption {
            label: "Height",
            description: "A fixed height in pixels; the frame widens to fill it. Default: fit the map.",
            minimum: Some(50.0),
            maximum: Some(10_000.0),
            step: Some(10.0),
            ..o("height", Integer, W::Number, "map", 60)
        },
        RenderOption {
            label: "Theme",
            description: "Colours of every slot; change single slots with color-<slot>.",
            default: Some(json!("wikimedia")),
            choices: choices(&[("wikimedia", "Wikimedia"), ("light", "Light"), ("dark", "Dark"), ("mono", "Monochrome")]),
            choices_source: Some("/api/v1/themes"),
            ..o("theme", String, W::Select, "map", 70)
        },
        RenderOption {
            label: "Padding",
            description: "Empty space around the map, in pixels.",
            default: Some(json!(d.padding)),
            minimum: Some(0.0),
            step: Some(1.0),
            advanced: true,
            ..o("padding", Integer, W::Number, "map", 80)
        },
        // Labels and places
        RenderOption {
            label: "Region names",
            description: "Label the mapped regions, placed to fit.",
            default: Some(json!(d.labels)),
            ..o("labels", Boolean, W::Checkbox, "labels", 10)
        },
        RenderOption {
            label: "Neighbour names",
            description: "Name the neighbouring countries, where there is room.",
            default: Some(json!(d.context_labels)),
            ..o("context-labels", Boolean, W::Checkbox, "labels", 20)
        },
        RenderOption {
            label: "Capitals",
            description: "National capitals, or also the regional capitals inside the map.",
            default: Some(json!("none")),
            choices: choices(&[("none", "None"), ("countries", "National capitals"), ("all", "National and regional capitals")]),
            ..o("capitals", String, W::Select, "labels", 30)
        },
        RenderOption {
            label: "Label languages",
            description: "Also label in these languages (BCP 47 tags); viewers see their own.",
            suggestions: LANGUAGE_SUGGESTIONS
                .iter()
                .map(|(v, label)| Choice { value: json!(v), label })
                .collect(),
            visible_when: Some(any_labels()),
            ..o("languages", List, W::Tokens, "labels", 40)
        },
        RenderOption {
            label: "Label size",
            description: "Size of region names in pixels; place and neighbour names are a little smaller.",
            default: Some(json!(t.label_size)),
            exclusive_minimum: Some(0.0),
            step: Some(0.5),
            advanced: true,
            visible_when: Some(any_labels()),
            ..o("label-size", Number, W::Number, "labels", 50)
        },
        RenderOption {
            label: "Smallest label size",
            description: "How far labels may shrink to fit, as a share of the label size.",
            default: Some(json!(d.label_min_scale)),
            exclusive_minimum: Some(0.0),
            maximum: Some(1.0),
            step: Some(0.05),
            advanced: true,
            visible_when: Some(any_labels()),
            ..o("label-min-scale", Number, W::Number, "labels", 60)
        },
        RenderOption {
            label: "Leader lines",
            description: "Label small regions outside them, with a line.",
            default: Some(json!(d.label_leaders)),
            advanced: true,
            visible_when: Some(equals("labels", json!(true))),
            ..o("leaders", Boolean, W::Checkbox, "labels", 70)
        },
        RenderOption {
            label: "Curved labels",
            description: "Curve labels along long, thin regions.",
            default: Some(json!(d.label_curved)),
            advanced: true,
            visible_when: Some(any_labels()),
            ..o("curved-labels", Boolean, W::Checkbox, "labels", 80)
        },
        RenderOption {
            label: "Made for",
            description: "Wikimedia Commons (curved labels as rotated letters, which its renderer draws) or web pages (textPath).",
            default: Some(json!("commons")),
            choices: choices(&[("commons", "Wikimedia Commons"), ("web", "Web pages")]),
            advanced: true,
            ..o("target", String, W::Select, "labels", 90)
        },
        // Frame and projection
        RenderOption {
            label: "Frame",
            description: "What the map shows: the regions' main landmass, all of them, or the world.",
            default: Some(json!("auto")),
            choices: choices(&[("auto", "Main landmass"), ("all", "Everything"), ("world", "World")]),
            ..o("frame", String, W::Select, "frame", 10)
        },
        RenderOption {
            label: "Box",
            description: "An area to show instead of the frame: west,south,east,north in degrees, or a preset name.",
            choices_source: Some("/api/v1/bbox-presets"),
            ..o("bbox", String, W::Bbox, "frame", 20)
        },
        RenderOption {
            label: "Projection",
            description: "Automatic picks one suited to the area.",
            default: Some(json!("auto")),
            choices: choices(&[
                ("auto", "Automatic"),
                ("laea", "Lambert azimuthal equal-area"),
                ("equal-earth", "Equal Earth"),
                ("albers", "Albers conic"),
                ("lcc", "Lambert conformal conic"),
            ]),
            ..o("projection", String, W::Select, "frame", 30)
        },
        RenderOption {
            label: "Standard parallels",
            description: "south,north in degrees, for conic projections. Default: from the area.",
            visible_when: Some(Condition::In { option: "projection", r#in: vec![json!("albers"), json!("lcc")] }),
            advanced: true,
            minimum: Some(-89.9),
            maximum: Some(89.9),
            ..o("parallels", Pair, W::Pair, "frame", 40)
        },
        RenderOption {
            label: "Central meridian",
            description: "Longitude at the centre of world maps, in degrees.",
            minimum: Some(-180.0),
            maximum: Some(180.0),
            step: Some(1.0),
            advanced: true,
            ..o("center-lon", Number, W::Number, "frame", 50)
        },
        RenderOption {
            label: "Insets",
            description: "Far-away parts (overseas territories, Alaska…) in corner boxes, or left out.",
            default: Some(json!("auto")),
            choices: choices(&[("auto", "In corner boxes"), ("none", "Left out")]),
            ..o("insets", String, W::Select, "frame", 60)
        },
        RenderOption {
            label: "Most insets",
            description: "At most this many corner boxes.",
            default: Some(json!(d.max_insets)),
            minimum: Some(0.0),
            maximum: Some(20.0),
            step: Some(1.0),
            advanced: true,
            visible_when: Some(equals("insets", json!("auto"))),
            ..o("max-insets", Integer, W::Number, "frame", 70)
        },
        RenderOption {
            label: "Margin",
            description: "Room around the frame, as a share of its size.",
            default: Some(json!(d.margin)),
            minimum: Some(0.0),
            maximum: Some(1.0),
            step: Some(0.01),
            advanced: true,
            ..o("margin", Number, W::Number, "frame", 80)
        },
        // Borders
        RenderOption {
            label: "Point of view",
            description: "A Natural Earth point of view on disputed borders (country code). Default: de facto.",
            choices_source: Some("/api/v1/datasets"),
            in_render_spec: false,
            ..o("worldview", String, W::Select, "borders", 10)
        },
        RenderOption {
            label: "Border drawing",
            description: "Borders drawn once in their own layer, or each region stroking its own outline (for hover effects).",
            default: Some(json!("layer")),
            choices: choices(&[("layer", "Once, by kind"), ("regions", "Around each region")]),
            advanced: true,
            ..o("border-mode", String, W::Select, "borders", 20)
        },
        width("border-width", "Border width", "Borders between mapped regions, in pixels.", t.border_width, 30),
        width("parent-border-width", "Parent border width", "Borders between regions of different parents (régions, states), in pixels.", t.parent_border_width, 40),
        width("outline-width", "Outline width", "The outer edge of the mapped regions, in pixels.", t.outline_width, 50),
        width("context-border-width", "Neighbour border width", "Borders between neighbouring countries, in pixels.", t.context_border_width, 60),
        width("disputed-border-width", "Disputed border width", "Disputed and claimed boundaries, in pixels.", t.disputed_border_width, 70),
        // Credit
        RenderOption {
            label: "Credit on the map",
            description: "Draw the data credit in a corner (it is always in the file).",
            default: Some(json!(d.credit)),
            ..o("credit", Boolean, W::Checkbox, "credit", 10)
        },
        RenderOption {
            label: "Data credit",
            description: "Replaces the credit taken from the data.",
            max_length: Some(500),
            advanced: true,
            ..o("attribution", String, W::Text, "credit", 20)
        },
        RenderOption {
            label: "Boundary year",
            description: "Year the boundaries represent. Default: from the data.",
            max_length: Some(40),
            advanced: true,
            ..o("boundary-year", String, W::Text, "credit", 30)
        },
        RenderOption {
            label: "Source release",
            description: "Release of the boundary data. Default: the hosted one.",
            max_length: Some(200),
            advanced: true,
            ..o("source-release", String, W::Text, "credit", 40)
        },
        // Output
        RenderOption {
            label: "Coordinate decimals",
            description: "Decimal places in path coordinates.",
            default: Some(json!(d.precision)),
            minimum: Some(0.0),
            maximum: Some(6.0),
            step: Some(1.0),
            advanced: true,
            ..o("precision", Integer, W::Number, "output", 10)
        },
        RenderOption {
            label: "Simplification",
            description: "Drop detail smaller than this many pixels.",
            default: Some(json!(d.simplify_px)),
            minimum: Some(0.0),
            step: Some(0.1),
            advanced: true,
            ..o("simplify", Number, W::Number, "output", 20)
        },
        RenderOption {
            label: "Smallest island",
            description: "Drop islands and lakes smaller than this many square pixels.",
            default: Some(json!(d.min_area_px)),
            minimum: Some(0.0),
            step: Some(0.5),
            advanced: true,
            ..o("min-area", Number, W::Number, "output", 30)
        },
        RenderOption {
            label: "Snap",
            description: "Snap neighbours within this many pixels onto the outline.",
            default: Some(json!(d.snap_px)),
            minimum: Some(0.0),
            step: Some(0.5),
            advanced: true,
            ..o("snap", Number, W::Number, "output", 40)
        },
        RenderOption {
            label: "CSS variables",
            description: "Colours as var(--mg-<slot>, …), to restyle from a page.",
            default: Some(json!(d.css_vars)),
            advanced: true,
            ..o("css-vars", Boolean, W::Checkbox, "output", 50)
        },
    ]
}

/// The kind of the option behind a `RenderSpec` member, if there is one.
pub fn kind_of(key: &str) -> Option<Kind> {
    static KINDS: std::sync::OnceLock<Vec<(String, Kind)>> = std::sync::OnceLock::new();
    KINDS
        .get_or_init(|| {
            options(u32::MAX)
                .into_iter()
                .filter(|o| o.in_render_spec)
                .map(|o| (o.key, o.kind))
                .collect()
        })
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, kind)| *kind)
}

/// Colour slots, with the default theme's colours.
pub fn color_slots() -> Vec<ColorSlot> {
    let t = Theme::default();
    t.colors()
        .iter()
        .map(|(slot, color)| {
            let (_, label, description) = SLOT_TEXT
                .iter()
                .find(|(s, _, _)| s == slot)
                .copied()
                .unwrap_or((slot, slot, ""));
            ColorSlot {
                slot,
                name: format!("color-{slot}"),
                key: crate::params::camel(slot),
                label,
                description,
                default: color.as_str().to_owned(),
            }
        })
        .collect()
}

/// The whole description (`GET /api/v1/render-options`).
pub fn describe(max_width: u32) -> Description {
    let mut options = options(max_width);
    let group_order = |g: &str| {
        GROUPS
            .iter()
            .find(|x| x.id == g)
            .map_or(u32::MAX, |x| x.order)
    };
    options.sort_by_key(|o| (group_order(o.group), o.order));
    Description {
        version: VERSION,
        groups: GROUPS.to_vec(),
        options,
        color_slots: color_slots(),
        themes_source: "/api/v1/themes",
        excluded: EXCLUDED.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{parse, PATH_PARAMS};
    use crate::RenderSpec;
    use std::collections::BTreeSet;

    /// `RenderSpec`'s members, as serde lists them for an unknown field.
    fn render_spec_fields() -> BTreeSet<String> {
        let err = serde_json::from_value::<RenderSpec>(json!({ "__unknown__": 1 })).unwrap_err();
        let msg = err.to_string();
        let list = msg
            .split("expected one of")
            .nth(1)
            .expect("serde lists the fields");
        list.split('`')
            .skip(1)
            .step_by(2)
            .map(str::to_owned)
            .collect()
    }

    /// Parses one parameter as the API does and validates the result.
    fn accepts(name: &str, value: &str) -> Result<(), String> {
        let params = parse(&[(name.to_owned(), value.to_owned())], &PATH_PARAMS)
            .map_err(|e| e.to_string())?;
        let spec: RenderSpec =
            serde_json::from_value(Value::Object(params.spec)).map_err(|e| e.to_string())?;
        spec.options().map(|_| ()).map_err(|e| e.0)
    }

    fn wire(v: &Value) -> String {
        match v {
            Value::String(s) => s.clone(),
            Value::Array(items) => items.iter().map(wire).collect::<Vec<_>>().join(","),
            v => v.to_string(),
        }
    }

    #[test]
    fn every_render_spec_member_is_described_exactly_once() {
        let fields = render_spec_fields();
        assert!(fields.len() > 40, "{fields:?}");
        let options = options(4000);
        let mut described = BTreeSet::new();
        for o in options.iter().filter(|o| o.in_render_spec) {
            assert!(described.insert(o.key.clone()), "{} described twice", o.key);
            assert!(
                fields.contains(&o.key),
                "{} is not a RenderSpec member",
                o.key
            );
            assert_eq!(crate::params::camel(o.name), o.key);
        }
        let excluded: BTreeSet<String> = EXCLUDED
            .iter()
            .map(|e| crate::params::camel(e.name))
            .collect();
        for f in &fields {
            let covered = described.contains(f) || excluded.contains(f) || f == "colors";
            assert!(
                covered,
                "RenderSpec member {f} has no description (options.rs)"
            );
            assert!(
                !(described.contains(f) && excluded.contains(f)),
                "{f} both described and excluded"
            );
        }
        let names: BTreeSet<&str> = options.iter().map(|o| o.name).collect();
        assert_eq!(names.len(), options.len(), "duplicate names");
        for o in &options {
            assert!(
                !o.label.is_empty() && !o.description.is_empty(),
                "{}: label and help",
                o.name
            );
            assert!(
                GROUPS.iter().any(|g| g.id == o.group),
                "{}: group {}",
                o.name,
                o.group
            );
        }
    }

    #[test]
    fn types_defaults_and_choices_are_what_the_api_accepts() {
        for o in options(4000).iter().filter(|o| o.in_render_spec) {
            let sample = match o.kind {
                Kind::Boolean => "true".to_owned(),
                Kind::Integer => o.minimum.unwrap_or(1.0).max(1.0).to_string(),
                Kind::Number => o
                    .minimum
                    .or(o.exclusive_minimum.map(|m| m + 0.5))
                    .unwrap_or(1.0)
                    .to_string(),
                Kind::List => "fr,de".to_owned(),
                Kind::Pair => "30,60".to_owned(),
                Kind::String => o.choices.first().map_or("x".to_owned(), |c| wire(&c.value)),
            };
            let sample = if o.name == "bbox" {
                "europe".to_owned()
            } else {
                sample
            };
            accepts(o.name, &sample).unwrap_or_else(|e| panic!("{}={sample}: {e}", o.name));
            // Mistyped values are refused.
            if matches!(
                o.kind,
                Kind::Boolean | Kind::Integer | Kind::Number | Kind::Pair
            ) {
                assert!(
                    accepts(o.name, "nonsense").is_err(),
                    "{}=nonsense accepted",
                    o.name
                );
            }
            if let Some(d) = &o.default {
                accepts(o.name, &wire(d)).unwrap_or_else(|e| panic!("{} default {d}: {e}", o.name));
            }
            for c in &o.choices {
                accepts(o.name, &wire(&c.value))
                    .unwrap_or_else(|e| panic!("{}={}: {e}", o.name, c.value));
            }
            if !o.choices.is_empty() {
                assert!(
                    accepts(o.name, "__nope__").is_err(),
                    "{}: a value outside the choices",
                    o.name
                );
            }
        }
    }

    #[test]
    fn limits_match_the_validation() {
        for o in options(20_000)
            .iter()
            .filter(|o| o.in_render_spec && matches!(o.kind, Kind::Integer | Kind::Number))
        {
            if let Some(min) = o.minimum {
                accepts(o.name, &min.to_string())
                    .unwrap_or_else(|e| panic!("{} at its minimum: {e}", o.name));
            }
            if let Some(max) = o.maximum {
                accepts(o.name, &max.to_string())
                    .unwrap_or_else(|e| panic!("{} at its maximum: {e}", o.name));
            }
            if o.exclusive_minimum.is_some() {
                assert!(
                    accepts(o.name, "0").is_err(),
                    "{}: 0 accepted though exclusive",
                    o.name
                );
            }
        }
        // Limits the validation enforces.
        assert!(accepts("width", "15").is_err() && accepts("width", "20001").is_err());
        assert!(accepts("height", "49").is_err() && accepts("height", "10001").is_err());
        assert!(accepts("margin", "-0.1").is_err());
        assert_eq!(
            options(4000)
                .iter()
                .find(|o| o.name == "width")
                .unwrap()
                .maximum,
            Some(4000.0)
        );
    }

    #[test]
    fn conditions_name_real_options_and_values() {
        let all = options(4000);
        fn check(c: &Condition, all: &[RenderOption]) {
            let target = |name: &str| {
                all.iter()
                    .find(|o| o.name == name)
                    .unwrap_or_else(|| panic!("condition on {name}"))
            };
            let valid = |o: &RenderOption, v: &Value| match o.kind {
                Kind::Boolean => v.is_boolean(),
                _ => o.choices.is_empty() || o.choices.iter().any(|c| &c.value == v),
            };
            match c {
                Condition::Equals { option, equals } => {
                    assert!(valid(target(option), equals), "{option}={equals}")
                }
                Condition::In { option, r#in } => r#in
                    .iter()
                    .for_each(|v| assert!(valid(target(option), v), "{option}={v}")),
                Condition::NotEmpty { option, .. } => assert_eq!(target(option).kind, Kind::String),
                Condition::AnyOf { any_of } => any_of.iter().for_each(|c| check(c, all)),
            }
        }
        for o in &all {
            if let Some(c) = &o.visible_when {
                check(c, &all);
            }
        }
    }

    #[test]
    fn color_slots_are_the_theme_slots() {
        let slots = color_slots();
        assert_eq!(slots.len(), Theme::default().colors().len());
        for s in &slots {
            accepts(&s.name, "#123456").unwrap_or_else(|e| panic!("{}: {e}", s.name));
            assert!(
                !s.label.is_empty() && !s.description.is_empty() && s.label != s.slot,
                "{}",
                s.slot
            );
        }
        assert!(accepts("color-nowhere", "#123456").is_err());
    }

    #[test]
    fn description_is_grouped_and_ordered() {
        let d = describe(4000);
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["version"], 1);
        assert_eq!(v["options"][0]["name"], "title");
        assert_eq!(v["options"][0]["type"], "string");
        let projection = d.options.iter().find(|o| o.name == "parallels").unwrap();
        assert_eq!(
            serde_json::to_value(&projection.visible_when).unwrap(),
            json!({ "option": "projection", "in": ["albers", "lcc"] })
        );
    }
}
