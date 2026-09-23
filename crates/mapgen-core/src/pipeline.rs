use std::collections::BTreeSet;

use geo::{BoundingRect, Contains, MapCoords};
use geo_types::{coord, MultiPolygon, Point, Polygon, Rect};
use rstar::primitives::{GeomWithData, Rectangle};
use rstar::RTree;

use crate::error::{Error, Result};
use crate::feature::{MapFeature, MapLine};
use crate::frame::{anchor as frame_anchor, clusters, Cluster, FrameMode};
use crate::panel::{build_panel, GeoExtent, Panel, PanelSpec, Placement};
use crate::projection::{
    one_sixth_parallels, Albers, EqualEarth, LambertAzimuthalEqualArea, LambertConformalConic,
    MapProjection, Projection, ProjectionChoice,
};
use crate::svg::{write_svg, SvgDocument};
use crate::theme::Theme;

/// Far-away parts smaller than this share of the main landmass get no inset
/// (0.025 %: keeps Mayotte, Saint-Pierre-et-Miquelon or Puerto Rico next to
/// their mainland, leaves out Guam or American Samoa next to the US).
const INSET_MIN_SHARE: f64 = 2.5e-4;

/// The features to draw, in WGS84.
#[derive(Debug, Clone, Default)]
pub struct MapLayers {
    /// The regions the map is about (drawn in `land` colour, labelled).
    pub subject: Vec<MapFeature>,
    /// Neighbouring countries for context (drawn in `context-land` colour).
    pub context: Vec<MapFeature>,
    /// Lakes, drawn in `water` colour above the land.
    pub lakes: Vec<MapFeature>,
    /// Disputed areas (e.g. Natural Earth's), drawn hatched.
    pub disputed_areas: Vec<MapFeature>,
    /// Disputed or claimed boundaries, drawn dashed.
    pub disputed: Vec<MapLine>,
}

impl MapLayers {
    /// Removes from `context` the features being mapped: those whose id is the
    /// region code (e.g. `FRA` when mapping France's subdivisions) or matches a
    /// subject id.
    pub fn exclude_subject_from_context(&mut self, region: Option<&str>) {
        let subject = &self.subject;
        self.context
            .retain(|c| Some(c.id.as_str()) != region && !subject.iter().any(|s| s.id == c.id));
    }
}

/// How region borders are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BorderMode {
    /// A separate `#borders` layer, each border drawn once and styled by kind;
    /// region fills have no stroke.
    #[default]
    Layer,
    /// Each region strokes its own outline (shared borders drawn twice, but
    /// every region is self-contained, e.g. for hover highlighting). Smaller
    /// files; no coast/land-border distinction.
    Regions,
}

/// Whether far-away parts of the mapped area get inset boxes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InsetMode {
    /// With `FrameMode::Auto`, parts left out of the main frame (overseas
    /// territories, Alaska, Hawaii…) are drawn in boxes in the map's corners.
    #[default]
    Auto,
    /// Far-away parts are left out (and reported).
    None,
}

/// End-to-end rendering options.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderOptions {
    /// Output width in pixels; height follows the frame's aspect ratio.
    pub width: u32,
    /// Margin around the map, painted in the background colour.
    pub padding: u32,
    /// Decimal places kept for path coordinates.
    pub precision: usize,
    pub title: Option<String>,
    /// Data credit, e.g. "Natural Earth; IGN (Etalab Open License 2.0) via
    /// geoBoundaries". Always embedded as `<desc>`; drawn when `credit` is set.
    pub attribution: Option<String>,
    pub credit: bool,
    /// Year the boundaries represent (e.g. geoBoundaries'
    /// `boundaryYearRepresented`), written to the SVG and the credit so a
    /// map's boundary version is never implicit.
    pub boundary_year: Option<String>,
    /// Release of the boundary dataset (id, build date, commit…).
    pub source_release: Option<String>,
    pub theme: Theme,
    /// Emit `var(--mg-*, …)` colours for restyling from page CSS.
    pub css_vars: bool,
    pub labels: bool,
    /// Also label in these languages (BCP 47 tags, from `MapFeature::names`):
    /// labels are placed separately per language and emitted in a
    /// `<switch>` on `systemLanguage`, with `name` as the fallback.
    pub languages: Vec<String>,
    pub border_mode: BorderMode,
    /// Place labels of small regions outside them, with a leader line.
    pub label_leaders: bool,
    /// Curve labels along long, thin regions.
    pub label_curved: bool,
    /// Smallest label size, as a fraction of the theme's label size.
    pub label_min_scale: f64,
    /// Simplification tolerance in output pixels; `0.0` disables it.
    pub simplify_px: f64,
    /// Islands and lakes smaller than this many px² are dropped (a subject
    /// region always keeps its largest part).
    pub min_area_px: f64,
    /// Neighbouring-country vertices within this many pixels of the mapped
    /// area's outline are moved onto it (fixes gaps and doubled borders when
    /// the two come from different datasets). `0.0` disables it.
    pub snap_px: f64,
    pub projection: ProjectionChoice,
    pub frame: FrameMode,
    pub insets: InsetMode,
    /// At most this many insets; smaller far-away parts are left out.
    pub max_insets: usize,
    /// Extra room around the framed features, as a fraction of the frame size.
    pub margin: f64,
    /// Override the central meridian.
    pub center_lon: Option<f64>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            width: 1000,
            padding: 0,
            precision: 1,
            title: None,
            attribution: None,
            credit: false,
            boundary_year: None,
            source_release: None,
            theme: Theme::default(),
            css_vars: false,
            labels: false,
            languages: Vec::new(),
            border_mode: BorderMode::Layer,
            label_leaders: true,
            label_curved: true,
            label_min_scale: 0.7,
            simplify_px: 0.5,
            min_area_px: 0.5,
            snap_px: 2.0,
            projection: ProjectionChoice::Auto,
            frame: FrameMode::Auto,
            insets: InsetMode::Auto,
            max_insets: 6,
            margin: 0.04,
            center_lon: None,
        }
    }
}

/// An inset drawn on the map.
#[derive(Debug, Clone, PartialEq)]
pub struct InsetInfo {
    /// Ids of the regions in the inset.
    pub ids: Vec<String>,
    pub projection: String,
}

/// A rendered map plus facts about how it was made.
#[derive(Debug, Clone)]
pub struct Rendered {
    pub svg: String,
    pub width: u32,
    pub height: u32,
    pub projection: MapProjection,
    /// Ids of subject features that appear nowhere (outside the frame and not
    /// in an inset).
    pub outside_frame: Vec<String>,
    pub insets: Vec<InsetInfo>,
}

/// Renders a map. Output is a pure function of the inputs: layers are sorted
/// by id first, so input order never affects the SVG.
pub fn render(layers: &MapLayers, opts: &RenderOptions) -> Result<Rendered> {
    if layers.subject.is_empty() {
        return Err(Error::Empty);
    }
    let subject = sorted(&layers.subject, |f| &f.id);
    let context = sorted(&layers.context, |f| &f.id);
    let lakes = sorted(&layers.lakes, |f| &f.id);
    let disputed_areas = sorted(&layers.disputed_areas, |f| &f.id);
    let disputed = sorted(&layers.disputed, |l| &l.id);

    // Frame planning: the main cluster, plus far-away clusters for insets.
    let polys: Vec<(usize, Polygon<f64>)> = subject
        .iter()
        .enumerate()
        .flat_map(|(i, f)| f.geometry.0.iter().map(move |p| (i, p.clone())))
        .collect();
    let mut main_weight = 0.0;
    let (anchor, inset_clusters) = match opts.frame {
        FrameMode::Auto => {
            let plain: Vec<Polygon<f64>> = polys.iter().map(|(_, p)| p.clone()).collect();
            let all = clusters(&plain);
            let main = all.first().ok_or(Error::Empty)?;
            main_weight = main.weight;
            let anchor = MultiPolygon(main.members.iter().map(|&i| plain[i].clone()).collect());
            let insets: Vec<Cluster> = match opts.insets {
                InsetMode::Auto => all
                    .iter()
                    .skip(1)
                    .filter(|c| c.weight >= main.weight * INSET_MIN_SHARE)
                    .take(opts.max_insets)
                    .cloned()
                    .collect(),
                InsetMode::None => Vec::new(),
            };
            (Some(anchor), insets)
        }
        mode => (frame_anchor(&subject, mode), Vec::new()),
    };
    if anchor.as_ref().is_some_and(|a| a.0.is_empty()) {
        return Err(Error::Empty);
    }
    let projection = choose_projection(anchor.as_ref(), opts, opts.projection, opts.center_lon)?;

    let in_inset: BTreeSet<usize> = inset_clusters
        .iter()
        .flat_map(|c| c.members.iter().copied())
        .collect();
    let main = build_panel(PanelSpec {
        subject: restrict(&subject, &polys, |i| !in_inset.contains(&i)),
        context: &context,
        lakes: &lakes,
        disputed_areas: &disputed_areas,
        disputed: &disputed,
        anchor,
        frame_mode: opts.frame,
        projection: projection.clone(),
        placement: Placement::Canvas {
            width: opts.width,
            padding: opts.padding,
        },
        opts,
        label_size: opts.theme.label_size,
    })?;
    let (width, height) = (f64::from(opts.width), main.viewport.canvas_height);

    let grid = LandGrid::new(&main, width, height);
    let mut panels = vec![main];
    let mut boxes: Vec<Rect<f64>> = Vec::new();
    let mut insets = Vec::new();
    for cluster in &inset_clusters {
        let anchor = MultiPolygon(
            cluster
                .members
                .iter()
                .map(|&i| polys[i].1.clone())
                .collect(),
        );
        // EPSG codes are for the main map; insets pick their own projection.
        let choice = match opts.projection {
            ProjectionChoice::Epsg(_) => ProjectionChoice::Auto,
            c => c,
        };
        let proj = choose_projection(Some(&anchor), opts, choice, None)?;
        let share = if main_weight > 0.0 {
            cluster.weight / main_weight
        } else {
            1.0
        };
        let Some(size) = inset_size(&anchor, &proj, share, width, height) else {
            continue;
        };
        let Some(b) = place_box(size, &grid, &boxes, width, height) else {
            continue;
        };
        let members: BTreeSet<usize> = cluster.members.iter().copied().collect();
        let panel = build_panel(PanelSpec {
            subject: restrict(&subject, &polys, |i| members.contains(&i)),
            context: &context,
            lakes: &lakes,
            disputed_areas: &disputed_areas,
            disputed: &disputed,
            anchor: Some(anchor),
            frame_mode: FrameMode::Auto,
            projection: proj,
            placement: Placement::Box(b),
            opts,
            label_size: 0.9 * opts.theme.label_size,
        })?;
        if panel.subject.is_empty() {
            continue;
        }
        boxes.push(b);
        insets.push(InsetInfo {
            ids: panel.subject.iter().map(|f| f.id.clone()).collect(),
            projection: panel.projection.name(),
        });
        panels.push(panel);
    }

    let shown: BTreeSet<&str> = panels
        .iter()
        .flat_map(|p| p.subject.iter().map(|f| f.id.as_str()))
        .collect();
    let outside_frame: Vec<String> = subject
        .iter()
        .map(|f| f.id.clone())
        .filter(|id| !shown.contains(id.as_str()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let svg = write_svg(&SvgDocument {
        width: opts.width,
        height: height as u32,
        panels: &panels,
        theme: &opts.theme,
        title: opts.title.as_deref(),
        attribution: credit_with_version(opts).as_deref(),
        boundary_year: opts.boundary_year.as_deref(),
        source_release: opts.source_release.as_deref(),
        credit: opts.credit,
        precision: opts.precision,
        css_vars: opts.css_vars,
        region_strokes: opts.border_mode == BorderMode::Regions,
    });
    Ok(Rendered {
        svg,
        width: opts.width,
        height: height as u32,
        projection,
        outside_frame,
        insets,
    })
}

/// The credit, with the boundary year appended when known.
fn credit_with_version(opts: &RenderOptions) -> Option<String> {
    match (&opts.attribution, &opts.boundary_year) {
        (Some(a), Some(y)) => Some(format!("{a}; boundaries as of {y}")),
        (Some(a), None) => Some(a.clone()),
        (None, Some(y)) => Some(format!("Boundaries as of {y}")),
        (None, None) => None,
    }
}

fn sorted<T: Clone>(v: &[T], key: impl Fn(&T) -> &String) -> Vec<T> {
    let mut v = v.to_vec();
    v.sort_by(|a, b| key(a).cmp(key(b)));
    v
}

/// Features restricted to the polygons (by flat index) that `keep` accepts;
/// features left with no polygon are dropped.
fn restrict(
    subject: &[MapFeature],
    polys: &[(usize, Polygon<f64>)],
    keep: impl Fn(usize) -> bool,
) -> Vec<MapFeature> {
    let mut parts: Vec<Vec<Polygon<f64>>> = vec![Vec::new(); subject.len()];
    for (i, (owner, p)) in polys.iter().enumerate() {
        if keep(i) {
            parts[*owner].push(p.clone());
        }
    }
    subject
        .iter()
        .zip(parts)
        .filter(|(_, p)| !p.is_empty())
        .map(|(f, p)| MapFeature {
            geometry: MultiPolygon(p),
            ..f.clone()
        })
        .collect()
}

fn choose_projection(
    anchor: Option<&MultiPolygon<f64>>,
    opts: &RenderOptions,
    choice: ProjectionChoice,
    center_lon: Option<f64>,
) -> Result<MapProjection> {
    let e = anchor.map(GeoExtent::of);
    let (lon0, lat0) = e.map_or((0.0, 0.0), |e| e.center());
    let lon0 = center_lon.unwrap_or(lon0);
    let (south, north) = e.map_or((-60.0, 80.0), |e| (e.south, e.north));
    let span = e.map_or(360.0, |e| e.span);
    let laea = || MapProjection::Laea(LambertAzimuthalEqualArea { lon0, lat0 });
    let albers = |parallels: Option<(f64, f64)>| {
        MapProjection::Albers(Albers {
            lon0,
            lat0,
            parallels: parallels.unwrap_or_else(|| one_sixth_parallels(south, north)),
        })
    };
    Ok(match choice {
        ProjectionChoice::Auto => {
            // Conics suit regions wide in longitude at mid-latitudes; fixed
            // frames (continent presets) keep the azimuthal default.
            let conic = !matches!(opts.frame, FrameMode::BBox(_))
                && span >= 45.0
                && (20.0..=70.0).contains(&lat0.abs());
            if anchor.is_none() || span > 200.0 {
                MapProjection::EqualEarth(EqualEarth { lon0 })
            } else if conic {
                albers(None)
            } else {
                laea()
            }
        }
        ProjectionChoice::Laea => laea(),
        ProjectionChoice::EqualEarth => MapProjection::EqualEarth(EqualEarth { lon0 }),
        ProjectionChoice::Albers { parallels } => albers(parallels),
        ProjectionChoice::Lcc { parallels } => MapProjection::Lcc(LambertConformalConic {
            lon0,
            lat0,
            parallels: parallels.unwrap_or_else(|| one_sixth_parallels(south, north)),
        }),
        #[cfg(feature = "proj")]
        ProjectionChoice::Epsg(code) => {
            MapProjection::Epsg(crate::epsg::EpsgProjection::new(code, lon0)?)
        }
        #[cfg(not(feature = "proj"))]
        ProjectionChoice::Epsg(_) => return Err(Error::ProjUnavailable),
    })
}

/// Pixel size of an inset box, following the inset's aspect ratio (within
/// 1:2.5). The long side is up to a quarter of the map's short side, scaled by
/// the fourth root of the inset's area relative to the main landmass (so
/// Alaska gets a full box, Hawaii or Réunion a smaller one), and at least 55 %.
fn inset_size(
    anchor: &MultiPolygon<f64>,
    proj: &MapProjection,
    share: f64,
    width: f64,
    height: f64,
) -> Option<(f64, f64)> {
    let b = anchor
        .map_coords(|c| {
            let (x, y) = proj.project(c.x, c.y);
            coord! { x: x, y: y }
        })
        .bounding_rect()?;
    if !(b.width() > 0.0 && b.height() > 0.0) {
        return None;
    }
    let long = 0.25 * width.min(height) * (1.6 * share.max(0.0).powf(0.25)).clamp(0.55, 1.0);
    let aspect = (b.width() / b.height()).clamp(0.4, 2.5);
    Some(if aspect >= 1.0 {
        (long, long / aspect)
    } else {
        (long * aspect, long)
    })
}

/// Chooses where an inset box goes: corners first, then along the edges,
/// minimising how much of the main map's land it covers and never
/// overlapping another inset.
fn place_box(
    size: (f64, f64),
    grid: &LandGrid,
    taken: &[Rect<f64>],
    width: f64,
    height: f64,
) -> Option<Rect<f64>> {
    let (w, h) = size;
    let m = 6.0;
    if w + 2.0 * m > width || h + 2.0 * m > height {
        return None;
    }
    let (x0, x1, y0, y1) = (m, width - m - w, m, height - m - h);
    let mut candidates = vec![(x0, y1), (x1, y1), (x0, y0), (x1, y0)];
    let steps = |from: f64, to: f64, step: f64| {
        let n = ((to - from) / step).floor().max(0.0) as usize;
        (0..=n).map(move |k| from + k as f64 * step)
    };
    candidates.extend(steps(x0, x1, w / 2.0).map(|x| (x, y1)));
    candidates.extend(steps(y0, y1, h / 2.0).map(|y| (x0, y1 - (y - y0))));
    candidates.extend(steps(y0, y1, h / 2.0).map(|y| (x1, y1 - (y - y0))));
    candidates.extend(steps(x0, x1, w / 2.0).map(|x| (x, y0)));

    let gap = 4.0;
    candidates
        .into_iter()
        .map(|(x, y)| Rect::new(coord! { x: x, y: y }, coord! { x: x + w, y: y + h }))
        .filter(|r| {
            taken.iter().all(|t| {
                r.max().x + gap <= t.min().x
                    || t.max().x + gap <= r.min().x
                    || r.max().y + gap <= t.min().y
                    || t.max().y + gap <= r.min().y
            })
        })
        .enumerate()
        .min_by(|(ia, a), (ib, b)| grid.land_in(a).cmp(&grid.land_in(b)).then(ia.cmp(ib)))
        .map(|(_, r)| r)
}

/// Coarse raster of where the main map's regions are, in pixels.
struct LandGrid {
    cell: f64,
    cols: usize,
    rows: usize,
    land: Vec<bool>,
}

impl LandGrid {
    fn new(panel: &Panel, width: f64, height: f64) -> LandGrid {
        let cell = 6.0;
        let (cols, rows) = (
            (width / cell).ceil() as usize,
            (height / cell).ceil() as usize,
        );
        let vp = panel.viewport;
        let px: Vec<Polygon<f64>> = panel
            .subject
            .iter()
            .flat_map(|f| f.geometry.0.iter())
            .map(|p| {
                p.map_coords(|c| {
                    let (x, y) = vp.to_px(c.x, c.y);
                    coord! { x: x, y: y }
                })
            })
            .collect();
        let tree: RTree<GeomWithData<Rectangle<[f64; 2]>, usize>> = RTree::bulk_load(
            px.iter()
                .enumerate()
                .filter_map(|(i, p)| p.bounding_rect().map(|r| (i, r)))
                .map(|(i, r)| {
                    GeomWithData::new(
                        Rectangle::from_corners([r.min().x, r.min().y], [r.max().x, r.max().y]),
                        i,
                    )
                })
                .collect(),
        );
        let mut land = vec![false; cols * rows];
        for r in 0..rows {
            for c in 0..cols {
                let (x, y) = ((c as f64 + 0.5) * cell, (r as f64 + 0.5) * cell);
                land[r * cols + c] = tree
                    .locate_all_at_point(&[x, y])
                    .any(|hit| px[hit.data].contains(&Point::new(x, y)));
            }
        }
        LandGrid {
            cell,
            cols,
            rows,
            land,
        }
    }

    fn land_in(&self, r: &Rect<f64>) -> usize {
        let (c0, c1) = (
            (r.min().x / self.cell).floor().max(0.0) as usize,
            ((r.max().x / self.cell).ceil() as usize).min(self.cols),
        );
        let (r0, r1) = (
            (r.min().y / self.cell).floor().max(0.0) as usize,
            ((r.max().y / self.cell).ceil() as usize).min(self.rows),
        );
        (r0..r1)
            .flat_map(|row| (c0..c1).map(move |col| row * self.cols + col))
            .filter(|&i| self.land[i])
            .count()
    }
}
