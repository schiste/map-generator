//! One map panel (the main map or an inset): projection, shared-border
//! simplification, snapping, clipping, border classification and labels.

use std::collections::HashSet;

use geo::Contains;
use geo::{BooleanOps, BoundingRect, Closest, ClosestPoint, CoordsIter, MapCoords, SimplifyVw};
use geo_types::{
    coord, Coord, Line, LineString, MultiLineString, MultiPolygon, Point, Polygon, Rect,
};
use rstar::primitives::{GeomWithData, Rectangle};
use rstar::RTree;

use crate::antimeridian::{
    canonicalize_antimeridian, covering_arc, split_at_seam, split_lines_at_seam, wrap_longitude,
};
use crate::error::{Error, Result};
use crate::feature::{MapFeature, MapLine};
use crate::frame::{clip_to_rect, FrameMode};
use crate::labels::{place_labels, Label, LabelOptions};
use crate::pipeline::{BorderMode, RenderOptions};
use crate::projection::{MapProjection, Projection};
use crate::simplify::{vw_epsilon, BorderArc, Topology};
use crate::svg::Viewport;

/// What a border line separates, which decides how it is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BorderKind {
    /// Between or around neighbouring (context) countries.
    Context,
    /// Between two mapped regions with the same (or no) parent.
    Internal,
    /// Between mapped regions with different parents (e.g. two régions).
    Parent,
    /// The outer edge of the mapped area, when there is no neighbouring
    /// layer to tell coasts from land borders.
    Outline,
    /// Outer edge along the sea.
    Coast,
    /// Outer edge along a neighbouring country (a national border).
    External,
    /// Disputed or claimed boundary lines.
    Disputed,
}

impl BorderKind {
    pub fn class(self) -> &'static str {
        match self {
            BorderKind::Context => "mg-border-context",
            BorderKind::Internal => "mg-border-internal",
            BorderKind::Parent => "mg-border-parent",
            BorderKind::Outline => "mg-border-outline",
            BorderKind::Coast => "mg-border-outline mg-border-coast",
            BorderKind::External => "mg-border-outline mg-border-external",
            BorderKind::Disputed => "mg-border-disputed",
        }
    }
}

/// A finished panel: projected, simplified and clipped geometry.
#[derive(Debug, Clone)]
pub struct Panel {
    pub viewport: Viewport,
    pub water: MultiPolygon<f64>,
    pub context: Vec<MapFeature>,
    pub subject: Vec<MapFeature>,
    pub lakes: Vec<MapFeature>,
    /// Disputed areas (hatched).
    pub disputed_areas: Vec<MapFeature>,
    pub borders: Vec<(BorderKind, MultiLineString<f64>)>,
    /// In pixels.
    pub labels: Vec<Label>,
    /// Labels per extra language, placed independently of each other.
    pub translations: Vec<(String, Vec<Label>)>,
    /// For insets: the box, in pixels.
    pub inset_box: Option<Rect<f64>>,
    pub projection: MapProjection,
    /// Ids of subject features that ended up outside the frame.
    pub outside_frame: Vec<String>,
}

/// Where a panel goes on the canvas.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Placement {
    /// The main map: this width, with padding; the height follows.
    Canvas { width: u32, padding: u32 },
    /// An inset drawn inside this pixel box.
    Box(Rect<f64>),
}

pub(crate) struct PanelSpec<'a> {
    pub subject: Vec<MapFeature>,
    pub context: &'a [MapFeature],
    pub lakes: &'a [MapFeature],
    pub disputed_areas: &'a [MapFeature],
    pub disputed: &'a [MapLine],
    /// Lon/lat geometry whose extent is the frame; `None` for the whole globe.
    pub anchor: Option<MultiPolygon<f64>>,
    pub frame_mode: FrameMode,
    pub projection: MapProjection,
    pub placement: Placement,
    pub opts: &'a RenderOptions,
    pub label_size: f64,
}

/// Lon/lat extent that is aware of the antimeridian.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GeoExtent {
    pub west: f64,
    pub span: f64,
    pub south: f64,
    pub north: f64,
}

impl GeoExtent {
    pub fn of(mp: &MultiPolygon<f64>) -> GeoExtent {
        let lons: Vec<f64> = mp.exterior_coords_iter().map(|c| c.x).collect();
        let (west, span) = covering_arc(&lons).unwrap_or((0.0, 0.0));
        let (south, north) = mp
            .exterior_coords_iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(s, n), c| {
                (s.min(c.y), n.max(c.y))
            });
        GeoExtent {
            west,
            span,
            south,
            north,
        }
    }

    pub fn center(&self) -> (f64, f64) {
        (
            wrap_longitude(self.west + self.span / 2.0),
            (self.south + self.north) / 2.0,
        )
    }

    /// Cheap pre-filter: could a geometry with bounding box `b` be visible in a
    /// frame around this extent?
    fn near(&self, b: Rect<f64>) -> bool {
        let pad_lon = self.span * 0.5 + 5.0;
        let pad_lat = (self.north - self.south) * 0.5 + 5.0;
        if b.max().y < self.south - pad_lat || b.min().y > self.north + pad_lat {
            return false;
        }
        let (w, len) = (self.west - pad_lon, self.span + 2.0 * pad_lon);
        len >= 360.0
            || [-360.0, 0.0, 360.0, 720.0]
                .iter()
                .any(|s| b.min().x + s <= w + len && b.max().x + s >= w)
    }
}

pub(crate) fn build_panel(spec: PanelSpec) -> Result<Panel> {
    let PanelSpec {
        mut subject,
        context,
        lakes,
        disputed_areas,
        disputed,
        anchor,
        frame_mode,
        projection,
        placement,
        opts,
        label_size,
    } = spec;

    // 1. Keep only what can be visible.
    let extent = anchor.as_ref().map(GeoExtent::of);
    let near = |g: Option<Rect<f64>>| match (extent, g) {
        (Some(e), Some(b)) => e.near(b),
        (None, Some(_)) => true,
        (_, None) => false,
    };
    let mut context: Vec<MapFeature> = context
        .iter()
        .filter(|f| near(f.geometry.bounding_rect()))
        .cloned()
        .collect();
    let mut lakes: Vec<MapFeature> = lakes
        .iter()
        .filter(|f| near(f.geometry.bounding_rect()))
        .cloned()
        .collect();
    let mut disputed_areas: Vec<MapFeature> = disputed_areas
        .iter()
        .filter(|f| near(f.geometry.bounding_rect()))
        .cloned()
        .collect();
    let disputed: Vec<MapLine> = disputed
        .iter()
        .filter(|l| near(l.geometry.bounding_rect()))
        .cloned()
        .collect();

    // 2. Project (canonicalising ±180°, or cutting along the seam).
    let lon0 = projection.lon0();
    let seam_at_180 = projection.has_seam() && wrap_longitude(lon0) == 0.0;
    let prepare = |g: &MultiPolygon<f64>| -> MultiPolygon<f64> {
        let g = if seam_at_180 {
            g.clone()
        } else {
            canonicalize_antimeridian(g)
        };
        let g = if projection.has_seam() {
            split_at_seam(&g, lon0)
        } else {
            g
        };
        drop_non_finite(g.map_coords(|c| {
            let (x, y) = projection.project(c.x, c.y);
            coord! { x: x, y: y }
        }))
    };
    // Projected positions of vertices on ±180°: datasets cut islands there
    // (Taveuni, Chukotka), and a border segment between two of them is such a
    // cut, never a real border.
    let mut seam: HashSet<(u64, u64)> = HashSet::new();
    for f in subject.iter().chain(context.iter()) {
        for c in f.geometry.coords_iter().filter(|c| c.x.abs() == 180.0) {
            let lon = if seam_at_180 { c.x } else { -180.0 };
            let (x, y) = projection.project(lon, c.y);
            seam.insert(key(x, y));
        }
    }
    for f in subject
        .iter_mut()
        .chain(context.iter_mut())
        .chain(lakes.iter_mut())
        .chain(disputed_areas.iter_mut())
    {
        f.geometry = prepare(&f.geometry);
    }
    let disputed: Vec<MultiLineString<f64>> = disputed
        .iter()
        .map(|l| {
            let g = if projection.has_seam() {
                split_lines_at_seam(&l.geometry, lon0)
            } else {
                l.geometry.clone()
            };
            g.map_coords(|c| {
                let (x, y) = projection.project(c.x, c.y);
                coord! { x: x, y: y }
            })
        })
        .collect();

    // 3. Frame and viewport.
    let (frame, water) = match &anchor {
        Some(a) => {
            let b = prepare(a).bounding_rect().ok_or(Error::Empty)?;
            let m = match (frame_mode, placement) {
                (FrameMode::BBox(_), Placement::Canvas { .. }) => 0.0,
                (_, Placement::Box(_)) => 0.1 * b.width().max(b.height()),
                _ => opts.margin.max(0.0) * b.width().max(b.height()),
            };
            let r = Rect::new(
                coord! { x: b.min().x - m, y: b.min().y - m },
                coord! { x: b.max().x + m, y: b.max().y + m },
            );
            (Some(r), MultiPolygon(vec![r.to_polygon()]))
        }
        None => {
            let sphere = sphere_outline(&projection);
            (None, MultiPolygon(vec![sphere]))
        }
    };
    let extent_rect = match frame {
        Some(r) => r,
        None => water.bounding_rect().ok_or(Error::Empty)?,
    };
    let viewport = fit(extent_rect, placement)?;
    let eps = if opts.simplify_px > 0.0 {
        vw_epsilon(opts.simplify_px, viewport.scale)
    } else {
        0.0
    };
    let min_area = opts.min_area_px.max(0.0) / (viewport.scale * viewport.scale);

    // 4. Shared-border topology per layer; snap neighbours onto the subject.
    let geoms = |v: &[MapFeature]| v.iter().map(|f| f.geometry.clone()).collect::<Vec<_>>();
    let mut subject_topo = Topology::build(&geoms(&subject));
    subject_topo.simplify(eps);
    let mut context_topo = Topology::build(&geoms(&context));
    context_topo.simplify(eps);
    if opts.snap_px > 0.0 && !context.is_empty() {
        snap_onto(
            &mut context_topo,
            &subject_topo.outline_segments(),
            opts.snap_px / viewport.scale,
        );
    }
    let mut lakes_topo = Topology::build(&geoms(&lakes));
    lakes_topo.simplify(eps);
    let mut disputed_topo = Topology::build(&geoms(&disputed_areas));
    disputed_topo.simplify(eps);

    let (subject_geoms, subject_arcs) = subject_topo.finish(min_area, |_| true);
    let (context_geoms, context_arcs) = context_topo.finish(min_area, |_| false);
    let (lake_geoms, _) = lakes_topo.finish(min_area, |_| false);
    let (disputed_geoms, _) = disputed_topo.finish(min_area, |_| false);

    // 5. Clip to the frame.
    let clip = |g: MultiPolygon<f64>| match frame {
        Some(r) => clip_to_rect(&g, r),
        None => g,
    };
    for (f, g) in subject.iter_mut().zip(subject_geoms) {
        f.geometry = clip(g);
    }
    for (f, g) in context.iter_mut().zip(context_geoms) {
        f.geometry = clip(g);
    }
    for (f, g) in lakes.iter_mut().zip(lake_geoms) {
        f.geometry = clip(g);
    }
    for (f, g) in disputed_areas.iter_mut().zip(disputed_geoms) {
        f.geometry = clip(g);
    }

    // 6. Borders, each drawn once (unless regions stroke their own outlines).
    let mut by_kind: std::collections::BTreeMap<BorderKind, Vec<LineString<f64>>> =
        Default::default();
    let border_layer = opts.border_mode == BorderMode::Layer;
    if border_layer {
        for arc in context_arcs {
            by_kind
                .entry(BorderKind::Context)
                .or_default()
                .extend(without_seam(arc.coords, &seam));
        }
        let sides = (!context.is_empty()).then(|| SideTest::new(&context, 1.5 / viewport.scale));
        for arc in subject_arcs {
            let kind = classify(&arc, &subject);
            let runs = without_seam(arc.coords, &seam);
            match (kind, &sides, arc.interior_left) {
                (BorderKind::Outline, Some(sides), Some(left)) => {
                    for run in runs {
                        for (k, part) in sides.split(run, left) {
                            by_kind.entry(k).or_default().push(part);
                        }
                    }
                }
                _ => by_kind.entry(kind).or_default().extend(runs),
            }
        }
    }
    for d in disputed {
        for l in d {
            let l = if eps > 0.0 { l.simplify_vw(&eps) } else { l };
            by_kind.entry(BorderKind::Disputed).or_default().push(l);
        }
    }
    let borders = by_kind
        .into_iter()
        .map(|(k, lines)| (k, clip_lines(MultiLineString(lines), frame)))
        .filter(|(_, l)| !l.0.is_empty())
        .collect();

    let outside_frame = subject
        .iter()
        .filter(|f| f.geometry.0.is_empty())
        .map(|f| f.id.clone())
        .collect();
    subject.retain(|f| !f.geometry.0.is_empty());
    context.retain(|f| !f.geometry.0.is_empty());
    lakes.retain(|f| !f.geometry.0.is_empty());
    disputed_areas.retain(|f| !f.geometry.0.is_empty());

    // 7. Labels, in pixels.
    let (labels, translations) = if opts.labels {
        let px: Vec<MultiPolygon<f64>> = subject
            .iter()
            .map(|f| {
                f.geometry.map_coords(|c| {
                    let (x, y) = viewport.to_px(c.x, c.y);
                    coord! { x: x, y: y }
                })
            })
            .collect();
        let regions_in = |lang: Option<&str>| -> Vec<(&str, &MultiPolygon<f64>)> {
            subject
                .iter()
                .zip(&px)
                .map(|(f, g)| {
                    let name = lang.and_then(|l| f.names.get(l)).unwrap_or(&f.name);
                    (name.as_str(), g)
                })
                .collect()
        };
        let bounds = match placement {
            Placement::Canvas { width, .. } => [0.0, 0.0, f64::from(width), viewport.canvas_height],
            Placement::Box(b) => [b.min().x, b.min().y, b.max().x, b.max().y],
        };
        let label_opts = LabelOptions {
            size: label_size,
            min_scale: opts.label_min_scale,
            leaders: opts.label_leaders,
            curved: opts.label_curved,
        };
        let translations = opts
            .languages
            .iter()
            .map(|l| {
                let placed = place_labels(&regions_in(Some(l)), bounds, &label_opts);
                (l.clone(), placed)
            })
            .collect();
        (
            place_labels(&regions_in(None), bounds, &label_opts),
            translations,
        )
    } else {
        (Vec::new(), Vec::new())
    };

    Ok(Panel {
        viewport,
        water,
        context,
        subject,
        lakes,
        disputed_areas,
        borders,
        labels,
        translations,
        inset_box: match placement {
            Placement::Box(b) => Some(b),
            Placement::Canvas { .. } => None,
        },
        projection,
        outside_frame,
    })
}

/// Tells coasts from land borders: the point just outside an outline segment
/// lies in a neighbouring country for a land border, in the sea for a coast.
struct SideTest {
    polys: Vec<Polygon<f64>>,
    tree: RTree<GeomWithData<Rectangle<[f64; 2]>, usize>>,
    /// Probe distance from the segment, in projected units.
    delta: f64,
}

impl SideTest {
    fn new(context: &[MapFeature], delta: f64) -> SideTest {
        let polys: Vec<Polygon<f64>> = context
            .iter()
            .flat_map(|f| f.geometry.0.iter().cloned())
            .collect();
        let tree = RTree::bulk_load(
            polys
                .iter()
                .enumerate()
                .filter_map(|(i, p)| p.bounding_rect().map(|b| (i, b)))
                .map(|(i, b)| {
                    GeomWithData::new(
                        Rectangle::from_corners([b.min().x, b.min().y], [b.max().x, b.max().y]),
                        i,
                    )
                })
                .collect(),
        );
        SideTest { polys, tree, delta }
    }

    fn land_at(&self, x: f64, y: f64) -> bool {
        let p = Point::new(x, y);
        self.tree
            .locate_all_at_point(&[x, y])
            .any(|hit| self.polys[hit.data].contains(&p))
    }

    /// Splits an outline run into coast and land-border runs.
    fn split(
        &self,
        line: LineString<f64>,
        interior_left: bool,
    ) -> Vec<(BorderKind, LineString<f64>)> {
        let mut out: Vec<(BorderKind, LineString<f64>)> = Vec::new();
        for w in line.0.windows(2) {
            let (a, b) = (w[0], w[1]);
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let len = (dx * dx + dy * dy).sqrt();
            if len == 0.0 {
                continue;
            }
            // Outward normal: right of the walk when the interior is on the left.
            let (nx, ny) = if interior_left {
                (dy / len, -dx / len)
            } else {
                (-dy / len, dx / len)
            };
            let (mx, my) = ((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
            let kind = if self.land_at(mx + self.delta * nx, my + self.delta * ny) {
                BorderKind::External
            } else {
                BorderKind::Coast
            };
            match out.last_mut() {
                Some((k, run)) if *k == kind => run.0.push(b),
                _ => out.push((kind, LineString(vec![a, b]))),
            }
        }
        out
    }
}

fn key(x: f64, y: f64) -> (u64, u64) {
    ((x + 0.0).to_bits(), (y + 0.0).to_bits())
}

/// Splits an arc into the runs that don't lie along the antimeridian (a
/// segment whose two ends both came from ±180° vertices).
fn without_seam(coords: Vec<Coord<f64>>, seam: &HashSet<(u64, u64)>) -> Vec<LineString<f64>> {
    if seam.is_empty() {
        return vec![LineString(coords)];
    }
    let on = |c: &Coord<f64>| seam.contains(&key(c.x, c.y));
    let mut out = Vec::new();
    let mut run: Vec<Coord<f64>> = Vec::new();
    for (i, c) in coords.iter().enumerate() {
        if i > 0 && on(&coords[i - 1]) && on(c) {
            if run.len() >= 2 {
                out.push(LineString(std::mem::take(&mut run)));
            }
            run.clear();
        }
        run.push(*c);
    }
    if run.len() >= 2 {
        out.push(LineString(run));
    }
    out
}

fn classify(arc: &BorderArc, subject: &[MapFeature]) -> BorderKind {
    match arc.b {
        None => BorderKind::Outline,
        Some(b) => match (&subject[arc.a].parent, &subject[b].parent) {
            (Some(pa), Some(pb)) if pa != pb => BorderKind::Parent,
            _ => BorderKind::Internal,
        },
    }
}

/// Moves every vertex within `tolerance` of `segments` onto the nearest point
/// of the nearest segment. Used to pull neighbouring countries from one
/// dataset onto the outline of regions from another, closing gaps and
/// doubled borders.
pub(crate) fn snap_onto(topo: &mut Topology, segments: &[Line<f64>], tolerance: f64) {
    if segments.is_empty() || tolerance <= 0.0 {
        return;
    }
    let tree = RTree::bulk_load(segments.to_vec());
    let tol2 = tolerance * tolerance;
    topo.map_vertices(|c| {
        let p = Point::from(c);
        let Some(seg) = tree.nearest_neighbor(&p) else {
            return c;
        };
        let q = match seg.closest_point(&p) {
            Closest::Intersection(q) | Closest::SinglePoint(q) => q,
            Closest::Indeterminate => return c,
        };
        let (dx, dy) = (q.x() - c.x, q.y() - c.y);
        if dx * dx + dy * dy <= tol2 {
            q.into()
        } else {
            c
        }
    });
}

fn clip_lines(lines: MultiLineString<f64>, frame: Option<Rect<f64>>) -> MultiLineString<f64> {
    let Some(r) = frame else { return lines };
    let (inside, crossing): (Vec<LineString<f64>>, Vec<LineString<f64>>) =
        lines.0.into_iter().partition(|l| {
            l.bounding_rect().is_some_and(|b| {
                b.min().x >= r.min().x
                    && b.min().y >= r.min().y
                    && b.max().x <= r.max().x
                    && b.max().y <= r.max().y
            })
        });
    let mut out = inside;
    if !crossing.is_empty() {
        out.extend(r.to_polygon().clip(&MultiLineString(crossing), false));
    }
    MultiLineString(out)
}

/// Drops rings containing non-finite coordinates (points a projection could
/// not transform).
fn drop_non_finite(mp: MultiPolygon<f64>) -> MultiPolygon<f64> {
    let finite = |r: &LineString<f64>| r.0.iter().all(|c| c.x.is_finite() && c.y.is_finite());
    MultiPolygon(
        mp.0.into_iter()
            .filter(|p| finite(p.exterior()))
            .map(|p| {
                let (ext, holes) = p.into_inner();
                Polygon::new(ext, holes.into_iter().filter(finite).collect())
            })
            .collect(),
    )
}

fn sphere_outline(p: &MapProjection) -> Polygon<f64> {
    let lon0 = p.lon0();
    let edge = |lon: f64, lats: &mut dyn Iterator<Item = f64>| -> Vec<Coord<f64>> {
        lats.map(|lat| {
            let (x, y) = p.project(lon, lat);
            coord! { x: x, y: y }
        })
        .collect()
    };
    let mut pts = edge(lon0 - 180.0, &mut (0..=180).map(|i| -90.0 + f64::from(i)));
    pts.extend(edge(
        lon0 + 180.0,
        &mut (0..=180).map(|i| 90.0 - f64::from(i)),
    ));
    pts.push(pts[0]);
    Polygon::new(LineString(pts), vec![])
}

fn fit(frame: Rect<f64>, placement: Placement) -> Result<Viewport> {
    let (dx, dy) = (frame.width(), frame.height());
    if dx <= 0.0 || dy <= 0.0 {
        return Err(Error::DegenerateExtent);
    }
    match placement {
        Placement::Canvas { width, padding } => {
            let pad = f64::from(padding);
            let inner_w = f64::from(width) - 2.0 * pad;
            if inner_w <= 0.0 {
                return Err(Error::DegenerateExtent);
            }
            let scale = inner_w / dx;
            Ok(Viewport {
                min_x: frame.min().x,
                max_y: frame.max().y,
                scale,
                left: pad,
                top: pad,
                canvas_height: (dy * scale + 2.0 * pad).ceil(),
            })
        }
        Placement::Box(b) => {
            let scale = (b.width() / dx).min(b.height() / dy);
            Ok(Viewport {
                min_x: frame.min().x,
                max_y: frame.max().y,
                scale,
                left: b.min().x + (b.width() - dx * scale) / 2.0,
                top: b.min().y + (b.height() - dy * scale) / 2.0,
                canvas_height: 0.0,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sq(x: f64, y: f64, s: f64) -> MultiPolygon<f64> {
        MultiPolygon(vec![Rect::new(
            coord! { x: x, y: y },
            coord! { x: x + s, y: y + s },
        )
        .to_polygon()])
    }

    #[test]
    fn snapping_pulls_nearby_vertices_onto_the_outline() {
        // Neighbour's left edge is 0.3 units right of the subject's right edge.
        let subject = Topology::build(&[sq(0.0, 0.0, 10.0)]);
        let mut neighbour = Topology::build(&[sq(10.3, 0.0, 10.0)]);
        snap_onto(&mut neighbour, &subject.outline_segments(), 0.5);
        let (g, _) = neighbour.finish(0.0, |_| true);
        let min_x = g[0]
            .exterior_coords_iter()
            .map(|c| c.x)
            .fold(f64::INFINITY, f64::min);
        assert_eq!(min_x, 10.0);
        // Out of tolerance: untouched.
        let mut far = Topology::build(&[sq(11.0, 0.0, 10.0)]);
        snap_onto(&mut far, &subject.outline_segments(), 0.5);
        let (g, _) = far.finish(0.0, |_| true);
        assert_eq!(
            g[0].exterior_coords_iter()
                .map(|c| c.x)
                .fold(f64::INFINITY, f64::min),
            11.0
        );
    }

    #[test]
    fn seam_segments_are_dropped_from_borders() {
        let c = |x: f64, y: f64| coord! { x: x, y: y };
        let seam: HashSet<(u64, u64)> = [key(10.0, 0.0), key(10.0, 1.0), key(10.0, 2.0)].into();
        // Coast → along the seam (two segments) → coast again.
        let arc = vec![
            c(0.0, 0.0),
            c(10.0, 0.0),
            c(10.0, 1.0),
            c(10.0, 2.0),
            c(0.0, 2.0),
        ];
        let runs = without_seam(arc, &seam);
        assert_eq!(
            runs,
            vec![
                LineString(vec![c(0.0, 0.0), c(10.0, 0.0)]),
                LineString(vec![c(10.0, 2.0), c(0.0, 2.0)])
            ]
        );
    }

    #[test]
    fn outline_splits_into_coast_and_land_border() {
        // Subject square [0,10]²; a neighbour covers x ∈ [10, 20].
        let neighbour = MapFeature {
            geometry: sq(10.0, 0.0, 10.0),
            ..MapFeature::default()
        };
        let sides = SideTest::new(&[neighbour], 0.5);
        // Counter-clockwise walk around the subject: interior on the left.
        let ring = LineString::from(vec![
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (0.0, 10.0),
            (0.0, 0.0),
        ]);
        let parts = sides.split(ring, true);
        let kinds: Vec<BorderKind> = parts.iter().map(|(k, _)| *k).collect();
        assert_eq!(
            kinds,
            [BorderKind::Coast, BorderKind::External, BorderKind::Coast]
        );
        assert_eq!(
            parts[1].1,
            LineString::from(vec![(10.0, 0.0), (10.0, 10.0)])
        );
    }

    #[test]
    fn border_kinds_follow_parents() {
        let f = |id: &str, parent: Option<&str>| MapFeature {
            id: id.into(),
            name: id.into(),
            class: "x".into(),
            parent: parent.map(Into::into),
            ..MapFeature::default()
        };
        let subject = vec![
            f("a", Some("P")),
            f("b", Some("P")),
            f("c", Some("Q")),
            f("d", None),
        ];
        let arc = |a, b| BorderArc {
            coords: vec![],
            a,
            b,
            interior_left: None,
        };
        assert_eq!(classify(&arc(0, Some(1)), &subject), BorderKind::Internal);
        assert_eq!(classify(&arc(0, Some(2)), &subject), BorderKind::Parent);
        assert_eq!(classify(&arc(0, Some(3)), &subject), BorderKind::Internal);
        assert_eq!(classify(&arc(0, None), &subject), BorderKind::Outline);
    }
}
