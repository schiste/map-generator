//! The datasets the API serves, described by `datasets.toml` in the data
//! directory, with their regions indexed at startup, and the layers every
//! map shares (neighbouring countries, lakes, disputed borders).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use mapgen_core::{MapFeature, MapPlace};
use mapgen_data::join::CrosswalkRow;
use mapgen_spec::{Dataset as Preset, LayerSpec, LoadedLayer, LoadedLines};
use serde::{Deserialize, Serialize};

/// `datasets.toml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub context: ContextConfig,
    #[serde(rename = "dataset", default)]
    pub datasets: Vec<DatasetConfig>,
}

/// Layers drawn around every map (paths relative to the data directory).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextConfig {
    /// Natural Earth Admin-0: neighbouring countries, and country names.
    pub countries: Option<String>,
    pub lakes: Option<String>,
    /// Disputed boundary lines (GeoJSON).
    pub disputed: Option<String>,
    pub disputed_areas: Option<String>,
    /// Capitals: Natural Earth populated places (GeoJSON).
    pub places: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetConfig {
    pub id: String,
    pub title: String,
    /// Property layout: `ne-admin0`, `ne-admin1`, `geoboundaries`, `custom`.
    pub preset: Preset,
    /// One file holding every region (told apart by the preset's region column)...
    pub file: Option<String>,
    /// ...or one file per region: `geoboundaries/{region}-ADM1.gpkg`...
    pub files: Option<String>,
    /// ...or datasets listed before, mixed: the countries of a Natural Earth
    /// Admin-0 dataset and every single subdivision of an Admin-1 one.
    #[serde(default)]
    pub compose: Vec<String>,
    /// Administrative level, e.g. `ADM1`.
    pub level: Option<String>,
    /// More region columns, e.g. `CONTINENT`.
    #[serde(default)]
    pub filters: Vec<String>,
    /// Offer the whole layer as region `world`.
    #[serde(default)]
    pub world: bool,
    /// Accept `worldview=` (Natural Earth point-of-view files `<stem>_<code>`).
    #[serde(default)]
    pub worldviews: bool,
    /// Names in other languages (Natural Earth `name_xx` columns).
    #[serde(default)]
    pub languages: bool,
    /// Used when a file has no `.license.json` sidecar.
    pub credit: Option<String>,
    pub licence: Option<String>,
    pub licence_url: Option<String>,
    #[serde(default)]
    pub share_alike: bool,
    pub release: Option<String>,
    pub boundary_year: Option<String>,
}

/// Licence and version of the data behind a map.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
    pub credit: String,
    pub licence: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub licence_url: Option<String>,
    pub share_alike: bool,
    pub release: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boundary_year: Option<String>,
}

/// A region a map can be made for.
#[derive(Debug, Clone)]
pub struct Region {
    pub code: String,
    pub name: String,
    /// Other ways to write it, lowercase: ISO 3166-1 alpha-2, names in every
    /// Natural Earth language (`de`, `allemagne`, `deutschland`…).
    pub aliases: Vec<String>,
    pub file: PathBuf,
    /// Column compared against `code`; `None` for the whole file.
    pub column: Option<String>,
    /// Property layout of `file` (the dataset's, except in mixed datasets).
    pub preset: Preset,
    /// 0 for regions, 1 for the subdivisions of a mixed dataset: a name
    /// shared by both (Luxembourg) means the country.
    pub level: u8,
    pub provenance: Provenance,
}

pub struct DatasetEntry {
    pub config: DatasetConfig,
    pub regions: BTreeMap<String, Region>,
}

/// A crosswalk hosted in `crosswalks/`.
pub struct Crosswalk {
    pub id: String,
    /// The `.json` metadata, served as is.
    pub meta: serde_json::Value,
    pub table: String,
    pub rows: Vec<CrosswalkRow>,
}

pub struct Registry {
    pub dir: PathBuf,
    pub datasets: BTreeMap<String, DatasetEntry>,
    pub countries: Option<Arc<LoadedLayer>>,
    countries_file: Option<PathBuf>,
    pub lakes: Option<LoadedLayer>,
    pub disputed: Option<LoadedLines>,
    pub disputed_areas: Option<LoadedLayer>,
    pub places: Option<Vec<MapPlace>>,
    /// Point-of-view variants of the countries, loaded on first use.
    worldviews: Mutex<BTreeMap<String, Arc<LoadedLayer>>>,
    pub crosswalks: BTreeMap<String, Crosswalk>,
}

pub type Result<T> = std::result::Result<T, String>;

/// Natural Earth's name languages (`name_xx` columns).
const NE_LANGUAGES: [&str; 26] = [
    "ar", "bn", "de", "el", "en", "es", "fa", "fr", "he", "hi", "hu", "id", "it", "ja", "ko", "nl",
    "pl", "pt", "ru", "sv", "tr", "uk", "ur", "vi", "zh", "zh-Hant",
];

impl Registry {
    pub fn load(dir: &Path) -> Result<Registry> {
        let path = dir.join("datasets.toml");
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let config: Config =
            toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        Registry::from_config(dir, config)
    }

    pub fn from_config(dir: &Path, config: Config) -> Result<Registry> {
        let at = |p: &String| dir.join(p);
        let layer = |p: &Path, preset: Preset| -> Result<LoadedLayer> {
            let spec = LayerSpec::with_dataset(preset);
            let rows = mapgen_data::read_layer(p, &spec.query(), None)
                .map_err(|e| format!("{}: {e}", p.display()))?;
            Ok(LoadedLayer::from_rows(
                rows.into_iter().map(|f| (Some(f.id.clone()), f)).collect(),
                true,
                LoadedLayer::default_credit(&spec),
            ))
        };
        let countries_file = config.context.countries.as_ref().map(at);
        let countries = countries_file
            .as_deref()
            .map(|p| -> Result<LoadedLayer> {
                // With names in every language, for region aliases.
                let spec = LayerSpec {
                    languages: NE_LANGUAGES.iter().map(|l| l.to_string()).collect(),
                    ..LayerSpec::with_dataset(Preset::NeAdmin0)
                };
                let rows = mapgen_data::read_layer(p, &spec.query(), None)
                    .map_err(|e| format!("{}: {e}", p.display()))?;
                Ok(LoadedLayer::from_rows(
                    rows.into_iter().map(|f| (Some(f.id.clone()), f)).collect(),
                    true,
                    LoadedLayer::default_credit(&spec),
                ))
            })
            .transpose()?
            .map(Arc::new);
        let lakes = config
            .context
            .lakes
            .as_ref()
            .map(at)
            .map(|p| layer(&p, Preset::NeLakes))
            .transpose()?;
        let disputed_areas = config
            .context
            .disputed_areas
            .as_ref()
            .map(at)
            .map(|p| layer(&p, Preset::NeDisputedAreas))
            .transpose()?;
        let places = config
            .context
            .places
            .as_ref()
            .map(at)
            .map(|p| {
                let languages: Vec<String> = NE_LANGUAGES.iter().map(|l| l.to_string()).collect();
                mapgen_data::places::read_places(&p, &languages)
                    .map_err(|e| format!("{}: {e}", p.display()))
            })
            .transpose()?;
        let disputed = config
            .context
            .disputed
            .as_ref()
            .map(at)
            .map(|p| -> Result<LoadedLines> {
                let spec = LayerSpec::with_dataset(Preset::NeDisputed);
                let lines = mapgen_data::geojson::read_lines(&p, &spec.query())
                    .map_err(|e| format!("{}: {e}", p.display()))?;
                Ok(LoadedLines::from_lines(
                    lines,
                    Some("Natural Earth (de facto view)".into()),
                ))
            })
            .transpose()?;

        // Territories can share their country's ISO-2 code (Clipperton is
        // `FR`): the code goes to the largest of them, the country itself.
        let mut iso2_owner: BTreeMap<String, (f64, String)> = BTreeMap::new();
        for f in countries.iter().flat_map(|c| c.features()) {
            if let Some(iso2) = &f.country {
                let area = geo::Area::unsigned_area(&f.geometry);
                let owner = iso2_owner
                    .entry(iso2.to_lowercase())
                    .or_insert((area, f.id.clone()));
                if area > owner.0 {
                    *owner = (area, f.id.clone());
                }
            }
        }
        let names: BTreeMap<String, (String, Vec<String>)> = countries
            .as_ref()
            .map(|c| {
                c.features()
                    .map(|f| {
                        let iso2 = f
                            .country
                            .as_ref()
                            .map(|c| c.to_lowercase())
                            .filter(|c| iso2_owner.get(c).is_some_and(|(_, id)| *id == f.id));
                        let mut aliases: Vec<String> = f
                            .names
                            .values()
                            .chain(f.wikidata.as_ref())
                            .map(|a| a.to_lowercase())
                            .chain(iso2)
                            .collect();
                        aliases.sort();
                        aliases.dedup();
                        (f.id.clone(), (f.name.clone(), aliases))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut datasets = BTreeMap::new();
        for d in config.datasets {
            if datasets.contains_key(&d.id) {
                return Err(format!("dataset {:?} is listed twice", d.id));
            }
            let regions = if d.compose.is_empty() {
                index_regions(dir, &d, &names)?
            } else {
                compose_regions(&d, &datasets)?
            };
            datasets.insert(d.id.clone(), DatasetEntry { config: d, regions });
        }
        Ok(Registry {
            dir: dir.to_path_buf(),
            datasets,
            countries,
            countries_file,
            lakes,
            disputed,
            disputed_areas,
            places,
            worldviews: Mutex::new(BTreeMap::new()),
            crosswalks: load_crosswalks(&dir.join("crosswalks"))?,
        })
    }

    /// Neighbouring countries, in Natural Earth's point of view `view`
    /// (`None`: de facto).
    pub fn countries_for(&self, view: Option<&str>) -> Result<Option<Arc<LoadedLayer>>> {
        let (Some(view), Some(file)) = (view, &self.countries_file) else {
            return Ok(self.countries.clone());
        };
        let view = view.to_ascii_uppercase();
        if let Some(l) = self.worldviews.lock().unwrap().get(&view) {
            return Ok(Some(l.clone()));
        }
        let path = mapgen_data::worldview_path(file, &view).map_err(|e| e.to_string())?;
        if !path.exists() {
            return Err(format!("point of view {view} is not hosted"));
        }
        let spec = LayerSpec {
            worldview: Some(view.clone()),
            ..LayerSpec::with_dataset(Preset::NeAdmin0)
        };
        let rows =
            mapgen_data::read_layer(&path, &spec.query(), None).map_err(|e| e.to_string())?;
        let layer = Arc::new(LoadedLayer::from_rows(
            rows.into_iter().map(|f| (Some(f.id.clone()), f)).collect(),
            true,
            LoadedLayer::default_credit(&spec),
        ));
        let mut cache = self.worldviews.lock().unwrap();
        // A handful of views at most in memory.
        if cache.len() >= 4 {
            let first = cache.keys().next().cloned().expect("non-empty");
            cache.remove(&first);
        }
        cache.insert(view, layer.clone());
        Ok(Some(layer))
    }

    /// The features of a region, read from its file.
    pub fn features(
        &self,
        dataset: &DatasetEntry,
        region: &Region,
        languages: &[String],
        worldview: Option<&str>,
    ) -> Result<Vec<MapFeature>> {
        let mut file = region.file.clone();
        // The point of view changes the regions themselves only for datasets
        // with variants (Natural Earth countries); for the others it applies
        // to the neighbouring countries (`countries_for`).
        if let Some(view) = worldview.filter(|_| dataset.config.worldviews) {
            file = mapgen_data::worldview_path(&file, view).map_err(|e| e.to_string())?;
            if !file.exists() {
                return Err(format!(
                    "point of view {} is not hosted",
                    view.to_ascii_uppercase()
                ));
            }
        }
        let query = self.query(dataset, region, languages, worldview);
        let code = region.column.as_ref().map(|_| region.code.as_str());
        mapgen_data::read_layer(&file, &query, code).map_err(|e| e.to_string())
    }

    pub fn layer_spec(
        &self,
        dataset: &DatasetEntry,
        region: &Region,
        languages: &[String],
        worldview: Option<&str>,
    ) -> LayerSpec {
        LayerSpec {
            filter_property: region.column.clone(),
            languages: if dataset.config.languages {
                languages.to_vec()
            } else {
                Vec::new()
            },
            worldview: worldview
                .filter(|_| dataset.config.worldviews)
                .map(str::to_ascii_uppercase),
            attribution: Some(match worldview {
                Some(v) if dataset.config.worldviews => {
                    format!("Natural Earth ({} view)", v.to_ascii_uppercase())
                }
                _ => region.provenance.credit.clone(),
            }),
            ..LayerSpec::with_dataset(region.preset)
        }
    }

    fn query(
        &self,
        dataset: &DatasetEntry,
        region: &Region,
        languages: &[String],
        worldview: Option<&str>,
    ) -> mapgen_data::LayerQuery {
        let mut q = self
            .layer_spec(dataset, region, languages, worldview)
            .query();
        q.filter_column = region.column.clone();
        q
    }
}

/// The regions of a dataset, with names and provenance.
fn index_regions(
    dir: &Path,
    d: &DatasetConfig,
    names: &BTreeMap<String, (String, Vec<String>)>,
) -> Result<BTreeMap<String, Region>> {
    let name = |code: &str| {
        names
            .get(code)
            .map_or_else(|| code.to_owned(), |(n, _)| n.clone())
    };
    let aliases = |code: &str| names.get(code).map(|(_, a)| a.clone()).unwrap_or_default();
    let mut regions = BTreeMap::new();
    match (&d.file, &d.files) {
        (Some(file), None) => {
            let path = dir.join(file);
            if !path.exists() {
                return Err(format!("dataset {}: {} not found", d.id, path.display()));
            }
            let provenance = provenance(&path, d);
            let spec = LayerSpec::with_dataset(d.preset);
            let mut columns: Vec<String> = spec.query().filter_column.into_iter().collect();
            columns.extend(d.filters.iter().cloned());
            for column in columns {
                let mut q = spec.query();
                q.filter_column = Some(column.clone());
                let codes = mapgen_data::list_regions(&path, &q)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                for code in codes {
                    regions.entry(code.clone()).or_insert_with(|| Region {
                        name: name(&code),
                        aliases: aliases(&code),
                        code,
                        file: path.clone(),
                        column: Some(column.clone()),
                        preset: d.preset,
                        level: 0,
                        provenance: provenance.clone(),
                    });
                }
            }
            if d.world {
                regions.insert(
                    "world".into(),
                    Region {
                        code: "world".into(),
                        name: "World".into(),
                        aliases: Vec::new(),
                        file: path.clone(),
                        column: None,
                        preset: d.preset,
                        level: 0,
                        provenance,
                    },
                );
            }
        }
        (None, Some(pattern)) => {
            let (prefix, suffix) = pattern.split_once("{region}").ok_or_else(|| {
                format!("dataset {}: `files` needs a {{region}} placeholder", d.id)
            })?;
            // `geoboundaries/{region}-ADM1.gpkg`: folder `geoboundaries/`,
            // file names `{region}-ADM1.gpkg`.
            let (folder, stem_prefix) = match prefix.rfind('/') {
                Some(i) => (dir.join(&prefix[..i]), prefix[i + 1..].to_owned()),
                None => (dir.to_path_buf(), prefix.to_owned()),
            };
            let entries =
                std::fs::read_dir(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
            for e in entries.flatten() {
                let file = e.file_name().to_string_lossy().into_owned();
                let Some(code) = file
                    .strip_prefix(&stem_prefix)
                    .and_then(|f| f.strip_suffix(suffix))
                else {
                    continue;
                };
                if code.is_empty() || code.contains('/') {
                    continue;
                }
                let path = e.path();
                regions.insert(
                    code.to_owned(),
                    Region {
                        name: name(code),
                        aliases: aliases(code),
                        code: code.to_owned(),
                        provenance: provenance(&path, d),
                        file: path,
                        column: None,
                        preset: d.preset,
                        level: 0,
                    },
                );
            }
        }
        _ => {
            return Err(format!(
                "dataset {}: set exactly one of `file` and `files`",
                d.id
            ))
        }
    }
    if regions.is_empty() {
        return Err(format!("dataset {} has no regions", d.id));
    }
    Ok(regions)
}

/// The regions of a mixed dataset: the countries of its Admin-0 part, and
/// each subdivision of its Admin-1 part as a region of its own.
fn compose_regions(
    d: &DatasetConfig,
    datasets: &BTreeMap<String, DatasetEntry>,
) -> Result<BTreeMap<String, Region>> {
    let mut regions = BTreeMap::new();
    for id in &d.compose {
        let part = datasets
            .get(id)
            .ok_or_else(|| format!("dataset {}: compose {id:?} must be listed before it", d.id))?;
        match part.config.preset {
            Preset::NeAdmin0 => {
                let column = LayerSpec::with_dataset(Preset::NeAdmin0)
                    .query()
                    .filter_column;
                for r in part.regions.values().filter(|r| r.column == column) {
                    regions.entry(r.code.clone()).or_insert_with(|| r.clone());
                }
            }
            Preset::NeAdmin1 => {
                let path = part
                    .regions
                    .values()
                    .next()
                    .map(|r| r.file.clone())
                    .ok_or_else(|| format!("dataset {id} has no file"))?;
                let spec = LayerSpec {
                    languages: NE_LANGUAGES.iter().map(|l| l.to_string()).collect(),
                    ..LayerSpec::with_dataset(Preset::NeAdmin1)
                };
                let mut q = spec.query();
                q.filter_column = None;
                let features = mapgen_data::read_layer(&path, &q, None)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                // Ids are ISO 3166-2 codes, else Natural Earth's adm1_code:
                // the column to read a subdivision back by.
                let mut q = spec.query();
                q.filter_column = Some("iso_3166_2".into());
                let iso: BTreeSet<String> = mapgen_data::list_regions(&path, &q)
                    .map_err(|e| format!("{}: {e}", path.display()))?
                    .into_iter()
                    .collect();
                let provenance = part
                    .regions
                    .values()
                    .next()
                    .map(|r| r.provenance.clone())
                    .unwrap_or_default();
                for f in features {
                    let column = if iso.contains(&f.id) {
                        "iso_3166_2"
                    } else {
                        "adm1_code"
                    };
                    let mut aliases: Vec<String> = f
                        .names
                        .values()
                        .chain(f.wikidata.as_ref())
                        .map(|a| a.to_lowercase())
                        .collect();
                    aliases.sort();
                    aliases.dedup();
                    regions.entry(f.id.clone()).or_insert_with(|| Region {
                        name: f.name.clone(),
                        aliases,
                        file: path.clone(),
                        column: Some(column.into()),
                        preset: Preset::NeAdmin1,
                        level: 1,
                        provenance: provenance.clone(),
                        code: f.id,
                    });
                }
            }
            other => {
                return Err(format!(
                    "dataset {}: compose takes ne-admin0 and ne-admin1 datasets, not {other:?}",
                    d.id
                ))
            }
        }
    }
    if regions.is_empty() {
        return Err(format!("dataset {} has no regions", d.id));
    }
    Ok(regions)
}

/// From the file's `.license.json` sidecar (geoBoundaries), else the
/// dataset's own fields.
fn provenance(path: &Path, d: &DatasetConfig) -> Provenance {
    #[derive(Deserialize)]
    struct Sidecar {
        license: String,
        source: String,
        via: Option<String>,
        license_url: Option<String>,
        year: Option<String>,
        release: Option<String>,
    }
    let sidecar: Option<Sidecar> = std::fs::read_to_string(path.with_extension("license.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok());
    match sidecar {
        Some(s) => {
            let l = s.license.to_ascii_lowercase();
            Provenance {
                credit: match &s.via {
                    Some(via) => format!("{} ({}) via {via}", s.source, s.license),
                    None => format!("{} ({})", s.source, s.license),
                },
                share_alike: ["sharealike", "share-alike", "by-sa", "odbl"]
                    .iter()
                    .any(|t| l.contains(t)),
                licence: s.license,
                licence_url: s.license_url.map(|u| {
                    if u.starts_with("http") {
                        u
                    } else {
                        format!("https://{u}")
                    }
                }),
                release: s.release.or_else(|| d.release.clone()).unwrap_or_default(),
                boundary_year: s.year.or_else(|| d.boundary_year.clone()),
            }
        }
        None => Provenance {
            credit: d.credit.clone().unwrap_or_else(|| match d.preset {
                Preset::NeAdmin0 | Preset::NeAdmin1 => "Natural Earth (de facto view)".into(),
                _ => String::new(),
            }),
            licence: d.licence.clone().unwrap_or_default(),
            licence_url: d.licence_url.clone(),
            share_alike: d.share_alike,
            release: d.release.clone().unwrap_or_default(),
            boundary_year: d.boundary_year.clone(),
        },
    }
}

fn load_crosswalks(dir: &Path) -> Result<BTreeMap<String, Crosswalk>> {
    let mut out = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(out);
    };
    let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    files.sort();
    for meta_path in files
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
    {
        let id = meta_path
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let csv_path = meta_path.with_extension("csv");
        let meta: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(meta_path)
                .map_err(|e| format!("{}: {e}", meta_path.display()))?,
        )
        .map_err(|e| format!("{}: {e}", meta_path.display()))?;
        let table = std::fs::read_to_string(&csv_path)
            .map_err(|e| format!("{}: {e}", csv_path.display()))?;
        let parsed = mapgen_data::table::parse_table(&table)
            .map_err(|e| format!("{}: {e}", csv_path.display()))?;
        let rows = mapgen_data::join::crosswalk_rows(&parsed, "from", "to", Some("weight"))
            .map_err(|e| format!("{}: {e}", csv_path.display()))?;
        out.insert(
            id.clone(),
            Crosswalk {
                id,
                meta,
                table,
                rows,
            },
        );
    }
    Ok(out)
}
