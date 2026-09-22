//! Colours and stroke widths. Every colour ends up in a single `<style>` block
//! at the top of the SVG, so a map can be recoloured by editing a few lines.

use crate::error::{Error, Result};

/// A CSS colour value (`#c6ecff`, `rgb(1 2 3)`, `steelblue`, `none`...).
///
/// Only characters that are harmless inside a `<style>` block are accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Color(String);

impl Color {
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        let ok = !s.is_empty()
            && s.len() <= 64
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "#(),.% -".contains(c));
        if ok {
            Ok(Color(s.to_ascii_lowercase()))
        } else {
            Err(Error::InvalidColor(s.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::str::FromStr for Color {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        Color::parse(s)
    }
}

impl std::fmt::Display for Color {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    /// Canvas behind everything; visible in the padding, around a world map's
    /// outline, or through `water: none`.
    pub background: Color,
    /// Sea (the map frame or the globe outline) and lakes.
    pub water: Color,
    /// The regions being mapped.
    pub land: Color,
    /// Neighbouring countries drawn for context.
    pub context_land: Color,
    /// Borders between the mapped regions.
    pub border: Color,
    pub context_border: Color,
    pub lake_border: Color,
    pub label: Color,
    pub border_width: f64,
    pub context_border_width: f64,
    pub label_size: f64,
}

/// Names of the colour slots, as used for CSS custom properties (`--mg-<name>`).
pub const COLOR_SLOTS: [&str; 8] = [
    "background",
    "water",
    "land",
    "context-land",
    "border",
    "context-border",
    "lake-border",
    "label",
];

impl Theme {
    pub const NAMES: [&'static str; 4] = ["wikimedia", "light", "dark", "mono"];

    pub fn builtin(name: &str) -> Option<Theme> {
        let t = |bg, water, land, ctx, border, ctx_border, lake, label| Theme {
            background: Color(String::from(bg)),
            water: Color(String::from(water)),
            land: Color(String::from(land)),
            context_land: Color(String::from(ctx)),
            border: Color(String::from(border)),
            context_border: Color(String::from(ctx_border)),
            lake_border: Color(String::from(lake)),
            label: Color(String::from(label)),
            border_width: 0.5,
            context_border_width: 0.4,
            label_size: 11.0,
        };
        Some(match name {
            // Wikimedia Commons location-map conventions.
            "wikimedia" => t(
                "#ffffff", "#c6ecff", "#fefee9", "#f6e1b9", "#646464", "#a08070", "#0978ab",
                "#333333",
            ),
            "light" => t(
                "#ffffff", "#e8eef3", "#ffffff", "#f3f3f1", "#8c96a0", "#d2d6da", "#b8c7d3",
                "#3c4650",
            ),
            "dark" => t(
                "#0d1117", "#0d1b2a", "#2b3445", "#1b2230", "#0d1117", "#2a3342", "#1f3b57",
                "#e6edf3",
            ),
            "mono" => t(
                "#ffffff", "#ffffff", "#ffffff", "#eeeeee", "#000000", "#999999", "#000000",
                "#000000",
            ),
            _ => return None,
        })
    }

    /// `(slot name, colour)` pairs in [`COLOR_SLOTS`] order.
    pub fn colors(&self) -> [(&'static str, &Color); 8] {
        [
            ("background", &self.background),
            ("water", &self.water),
            ("land", &self.land),
            ("context-land", &self.context_land),
            ("border", &self.border),
            ("context-border", &self.context_border),
            ("lake-border", &self.lake_border),
            ("label", &self.label),
        ]
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme::builtin("wikimedia").expect("built-in theme")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_css_colors() {
        for c in [
            "#C6ECFF",
            "steelblue",
            "rgb(10, 20, 30)",
            "hsl(200 50% 40%)",
            "none",
        ] {
            assert!(Color::parse(c).is_ok(), "{c}");
        }
        assert_eq!(Color::parse("#C6ECFF").unwrap().as_str(), "#c6ecff");
    }

    #[test]
    fn rejects_style_injection() {
        for c in ["red;}</style><script>", "url(x)\"", "", "a{b}"] {
            assert!(Color::parse(c).is_err(), "{c}");
        }
    }

    #[test]
    fn all_builtins_exist() {
        for n in Theme::NAMES {
            assert!(Theme::builtin(n).is_some());
        }
    }
}
