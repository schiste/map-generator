//! Map recipes: a whole map (which regions, every design setting) as a
//! small CSV that a spreadsheet can edit, the playground can import and
//! export, and the API can render (`POST /api/v1/render`, `text/csv`).
//!
//! ```text
//! key,value
//! dataset,countries
//! region,France
//! region,DEU
//! region,Italy
//! title,Three neighbours
//! width,1200
//! labels,true
//! color-water,#c6ecff
//! ```
//!
//! Keys are the API's query parameters (`docs/api.md`), plus `dataset`,
//! `region` (repeated, or `regions` with `;` between values) and
//! `worldview`. Lines starting with `#` are comments. A table with a
//! `country`, `region` or `code` column instead is read as a list of
//! regions with default settings.

use serde_json::Value;

use crate::params::{self, ParamError};
use crate::{RenderSpec, Result, SpecError};

/// Recipe format version, written in exported recipes.
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Recipe {
    /// Dataset id (`ne-admin0`…); `countries` and `subdivisions` are aliases.
    pub dataset: Option<String>,
    /// Region codes or names, in the order given.
    pub regions: Vec<String>,
    pub worldview: Option<String>,
    /// Every other setting, as `(parameter, value)`.
    pub params: Vec<(String, String)>,
}

/// Columns that make a table a list of regions.
const REGION_COLUMNS: [&str; 9] = [
    "country",
    "countries",
    "region",
    "regions",
    "code",
    "iso",
    "iso3",
    "iso2",
    "iso_a3",
];

impl Recipe {
    pub fn parse(text: &str) -> Result<Recipe> {
        let lines: String = text
            .trim_start_matches('\u{feff}')
            .lines()
            .filter(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if lines.is_empty() {
            return Err(SpecError("the recipe is empty".into()));
        }
        let table = mapgen_data::table::parse_table(&lines).map_err(SpecError)?;
        let header: Vec<String> = table.header.iter().map(|h| h.to_lowercase()).collect();
        let is_kv_header = header.len() >= 2
            && ["key", "setting", "parameter", "name"].contains(&header[0].as_str())
            && ["value", "values"].contains(&header[1].as_str());
        // A list of regions: a region column (and possibly others, ignored).
        if !is_kv_header {
            if let Some(col) = header
                .iter()
                .position(|h| REGION_COLUMNS.contains(&h.as_str()))
            {
                let regions = table
                    .rows
                    .iter()
                    .filter_map(|r| table.cell(r, col))
                    .collect();
                return Ok(Recipe {
                    regions,
                    ..Recipe::default()
                });
            }
        }
        // Key/value rows; without a header, the first line is a setting too.
        let mut rows: Vec<Vec<String>> = Vec::new();
        if !is_kv_header {
            rows.push(table.header.clone());
        }
        rows.extend(table.rows.iter().cloned());
        let mut recipe = Recipe::default();
        for row in rows {
            let key = row
                .first()
                .map(|k| k.trim().to_lowercase().replace('_', "-"))
                .unwrap_or_default();
            if key.is_empty() {
                continue;
            }
            let value = row.get(1).map(|v| v.trim().to_owned()).unwrap_or_default();
            match key.as_str() {
                "dataset" | "map" => recipe.dataset = Some(dataset_alias(&value)),
                "region" | "country" => {
                    if !value.is_empty() {
                        recipe.regions.push(value);
                    }
                }
                "regions" | "countries" => recipe.regions.extend(
                    value
                        .split([';', '|'])
                        .map(str::trim)
                        .filter(|v| !v.is_empty())
                        .map(str::to_owned),
                ),
                "worldview" => {
                    recipe.worldview = Some(value.to_ascii_uppercase()).filter(|v| !v.is_empty())
                }
                "mapgen-recipe" | "format" | "recipe" => {}
                _ => recipe.params.push((key, value)),
            }
        }
        // Validate the settings now, with the key as written in the file.
        recipe.spec().map(|_| recipe)
    }

    /// The render spec for the settings (without the regions, which the
    /// caller resolves to codes).
    pub fn spec(&self) -> Result<RenderSpec> {
        let parsed = params::parse(&self.params, &[("format", "a recipe renders SVG")])
            .map_err(|ParamError { param, why }| SpecError(format!("`{param}`: {why}")))?;
        serde_json::from_value(Value::Object(parsed.spec))
            .map_err(|e| SpecError(kebab_fields(&e.to_string())))
    }

    /// A recipe from a render spec (camelCase JSON, as in WASM) and regions.
    pub fn from_spec(
        dataset: Option<String>,
        regions: Vec<String>,
        worldview: Option<String>,
        spec: &serde_json::Map<String, Value>,
    ) -> Recipe {
        let mut spec = spec.clone();
        spec.remove("region");
        spec.remove("regions");
        Recipe {
            dataset,
            regions,
            worldview,
            params: params::to_pairs(&spec, ";").into_iter().collect(),
        }
    }

    /// The recipe as CSV, settings in a stable order.
    pub fn to_csv(&self) -> String {
        let mut out = format!("# map-generator recipe v{VERSION}: https://github.com/schiste/map-generator/blob/main/docs/recipes.md\nkey,value\n");
        let mut row = |k: &str, v: &str| {
            out.push_str(&mapgen_data::join::csv_cell(k));
            out.push(',');
            out.push_str(&mapgen_data::join::csv_cell(v));
            out.push('\n');
        };
        if let Some(d) = &self.dataset {
            row("dataset", d);
        }
        for r in &self.regions {
            row("region", r);
        }
        if let Some(w) = &self.worldview {
            row("worldview", w);
        }
        let mut params = self.params.clone();
        params.sort_by(|a, b| a.0.cmp(&b.0));
        for (k, v) in &params {
            row(k, v);
        }
        out
    }
}

/// `countries` → `ne-admin0`, `subdivisions` → `ne-admin1`.
pub fn dataset_alias(value: &str) -> String {
    match value.trim().to_lowercase().as_str() {
        "countries" | "country" | "world" => "ne-admin0".into(),
        "subdivisions" | "states" | "provinces" | "admin1" => "ne-admin1".into(),
        "mixed" | "countries and subdivisions" => "mixed".into(),
        v => v.to_owned(),
    }
}

/// `unknown field `colourWater`` → `colour-water`, as the recipe spells it.
fn kebab_fields(s: &str) -> String {
    s.split('`')
        .enumerate()
        .map(|(i, part)| {
            if i % 2 == 1 && part.chars().all(|c| c.is_ascii_alphanumeric()) {
                params::kebab(part)
            } else {
                part.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("`")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_key_value_recipes() {
        let r = Recipe::parse(
            "# my map\nkey,value\ndataset,countries\nregion,France\nregion,DEU\nregions,Italy; ESP\ntitle,\"Four, in Europe\"\nwidth,1200\nlabels,true\ncolor-water,#c6ecff\nlanguages,fr;de\n",
        )
        .unwrap();
        assert_eq!(r.dataset.as_deref(), Some("ne-admin0"));
        assert_eq!(r.regions, ["France", "DEU", "Italy", "ESP"]);
        let spec = r.spec().unwrap();
        assert_eq!(spec.width, Some(1200));
        assert!(spec.labels);
        assert_eq!(spec.title.as_deref(), Some("Four, in Europe"));
        assert_eq!(spec.colors["water"], "#c6ecff");
        assert_eq!(spec.languages, ["fr", "de"]);
        let r = Recipe::parse("key,value\nbbox,-10;35;30;60\nparallels,29.5;45.5\n").unwrap();
        let spec = r.spec().unwrap();
        assert_eq!(
            (spec.bbox.as_deref(), spec.parallels),
            (Some("-10,35,30,60"), Some([29.5, 45.5]))
        );
    }

    #[test]
    fn headerless_and_list_recipes() {
        let r = Recipe::parse("dataset,subdivisions\nregion,FRA\ntheme,dark\n").unwrap();
        assert_eq!(
            (r.dataset.as_deref(), r.regions.len()),
            (Some("ne-admin1"), 1)
        );
        assert_eq!(r.spec().unwrap().theme.as_deref(), Some("dark"));
        // A Maphue-style country list.
        let r = Recipe::parse("country,category\nFrance,A\nDEU,B\n").unwrap();
        assert_eq!(
            (r.regions.clone(), r.params.len()),
            (vec!["France".to_string(), "DEU".to_string()], 0)
        );
        let r = Recipe::parse("iso3\nFRA\nITA\n").unwrap();
        assert_eq!(r.regions, ["FRA", "ITA"]);
    }

    #[test]
    fn rejects_bad_settings_with_their_recipe_name() {
        let e = Recipe::parse("key,value\ncolour-water,red\n").unwrap_err();
        assert!(e.0.contains("colour-water"), "{e}");
        let e = Recipe::parse("key,value\nwidth,wide\n").unwrap_err();
        assert!(e.0.contains("`width`"), "{e}");
        assert!(Recipe::parse("# nothing\n").is_err());
    }

    #[test]
    fn round_trips_through_csv() {
        let r = Recipe {
            dataset: Some("ne-admin0".into()),
            regions: vec!["FRA".into(), "DEU".into()],
            worldview: Some("IND".into()),
            params: vec![
                ("width".into(), "900".into()),
                ("title".into(), "A, B".into()),
            ],
        };
        let csv = r.to_csv();
        assert!(csv.contains("key,value\ndataset,ne-admin0\nregion,FRA\nregion,DEU\nworldview,IND\ntitle,\"A, B\"\nwidth,900\n"), "{csv}");
        let back = Recipe::parse(&csv).unwrap();
        assert_eq!(
            (back.dataset, back.regions, back.worldview),
            (r.dataset, r.regions, r.worldview)
        );
    }
}
