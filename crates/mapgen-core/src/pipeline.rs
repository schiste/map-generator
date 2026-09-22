use geo::{Area, BoundingRect, CoordsIter, MapCoords};
use geo_types::{coord, LineString, MultiPolygon, Polygon, Rect};

use crate::antimeridian::{covering_arc, split_at_seam, wrap_longitude};
use crate::error::{Error, Result};
use crate::feature::MapFeature;
use crate::frame::{anchor, clip_to_rect, FrameMode};
use crate::projection::{
    EqualEarth, LambertAzimuthalEqualArea, MapProjection, Projection, ProjectionChoice,
};
use crate::simplify::{simplify_shared, vw_epsilon};
use crate::svg::{write_svg, SvgDocument, Viewport};
use crate::theme::Theme;

/// The features to draw, in WGS84.
#[derive(Debug, Clone, Default)]
pub struct MapLayers {
    /// The regions the map is about (drawn in `land` colour, labelled).
    pub subject: Vec<MapFeature>,
    /// Neighbouring countries for context (drawn in `context-land` colour).
    pub context: Vec<MapFeature>,
    /// Lakes, drawn in `water` colour above the land.
    pub lakes: Vec<MapFeature>,
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
    pub theme: Theme,
    /// Emit `var(--mg-*, …)` colours for restyling from page CSS.
    pub css_vars: bool,
    pub labels: bool,
    /// Simplification tolerance in output pixels; `0.0` disables it.
    pub simplify_px: f64,
    /// Islands and lakes smaller than this many px² are dropped (a subject
    /// region always keeps its largest part).
    pub min_area_px: f64,
    pub projection: ProjectionChoice,
    pub frame: FrameMode,
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
            theme: Theme::default(),
            css_vars: false,
            labels: false,
            simplify_px: 0.5,
            min_area_px: 0.5,
            projection: ProjectionChoice::Auto,
            frame: FrameMode::Auto,
            margin: 0.04,
            center_lon: None,
        }
    }
}

/// A rendered map plus facts about how it was made.
#[derive(Debug, Clone)]
pub struct Rendered {
    pub svg: String,
    pub width: u32,
    pub height: u32,
    pub projection: MapProjection,
    /// Ids of subject features that fell entirely outside the frame.
    pub outside_frame: Vec<String>,
}

/// Renders a map. Output is a pure function of the inputs: layers are sorted
/// by id first, so input order never affects the SVG.
pub fn render(layers: &MapLayers, opts: &RenderOptions) -> Result<Rendered> {
    if layers.subject.is_empty() {
        return Err(Error::Empty);
    }
    let sorted = |v: &[MapFeature]| {
        let mut v = v.to_vec();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    };
    let mut subject = sorted(&layers.subject);
    let mut context = sorted(&layers.context);
    let mut lakes = sorted(&layers.lakes);

    let mut anchor = anchor(&subject, opts.frame);
    if anchor.as_ref().is_some_and(|a| a.0.is_empty()) {
        return Err(Error::Empty);
    }
    let projection = choose_projection(anchor.as_ref(), opts);

    if let Some(a) = &anchor {
        let extent = GeoExtent::of(a);
        context.retain(|f| extent.near(&f.geometry));
        lakes.retain(|f| extent.near(&f.geometry));
    }

    // Project everything (cutting along the seam first when there is one).
    let lon0 = projection.lon0();
    let prepare = |g: &MultiPolygon<f64>| {
        let g = if projection.has_seam() {
            split_at_seam(g, lon0)
        } else {
            g.clone()
        };
        g.map_coords(|c| {
            let (x, y) = projection.project(c.x, c.y);
            coord! { x: x, y: y }
        })
    };
    for f in subject
        .iter_mut()
        .chain(context.iter_mut())
        .chain(lakes.iter_mut())
    {
        f.geometry = prepare(&f.geometry);
    }
    anchor = anchor.map(|a| prepare(&a));

    // Frame: a rectangle around the anchor, or the globe outline.
    let (frame, water) = match &anchor {
        Some(a) => {
            let b = a.bounding_rect().ok_or(Error::Empty)?;
            let m = match opts.frame {
                FrameMode::BBox(_) => 0.0,
                _ => opts.margin.max(0.0) * b.width().max(b.height()),
            };
            let r = Rect::new(
                coord! { x: b.min().x - m, y: b.min().y - m },
                coord! { x: b.max().x + m, y: b.max().y + m },
            );
            (r, MultiPolygon(vec![r.to_polygon()]))
        }
        None => {
            let sphere = sphere_outline(&projection);
            (
                sphere.bounding_rect().ok_or(Error::Empty)?,
                MultiPolygon(vec![sphere]),
            )
        }
    };
    let viewport = fit_viewport(frame, opts.width, opts.padding)?;

    // Simplify each layer as a whole so shared borders stay shared.
    let eps = if opts.simplify_px > 0.0 {
        vw_epsilon(opts.simplify_px, viewport.scale)
    } else {
        0.0
    };
    for layer in [&mut subject, &mut context, &mut lakes] {
        let geoms: Vec<_> = layer.iter().map(|f| f.geometry.clone()).collect();
        for (f, g) in layer.iter_mut().zip(simplify_shared(&geoms, eps)) {
            f.geometry = g;
        }
    }

    // Clip to the frame and drop specks.
    let min_area = opts.min_area_px.max(0.0) / (viewport.scale * viewport.scale);
    let clip = anchor.is_some();
    let finish = |layer: &mut Vec<MapFeature>, keep_largest: bool| {
        for f in layer.iter_mut() {
            if clip {
                f.geometry = clip_to_rect(&f.geometry, frame);
            }
            f.geometry = cull(&f.geometry, min_area, keep_largest);
        }
    };
    finish(&mut subject, true);
    finish(&mut context, false);
    finish(&mut lakes, false);
    let outside_frame = subject
        .iter()
        .filter(|f| f.geometry.0.is_empty())
        .map(|f| f.id.clone())
        .collect();
    for layer in [&mut subject, &mut context, &mut lakes] {
        layer.retain(|f| !f.geometry.0.is_empty());
    }

    let svg = write_svg(&SvgDocument {
        viewport,
        water: &water,
        context: &context,
        subject: &subject,
        lakes: &lakes,
        labels: opts.labels,
        theme: &opts.theme,
        title: opts.title.as_deref(),
        precision: opts.precision,
        css_vars: opts.css_vars,
    });
    Ok(Rendered {
        svg,
        width: viewport.width,
        height: viewport.height,
        projection,
        outside_frame,
    })
}

fn choose_projection(anchor: Option<&MultiPolygon<f64>>, opts: &RenderOptions) -> MapProjection {
    let (lon0, lat0, span) = match anchor {
        Some(a) => {
            let e = GeoExtent::of(a);
            (
                wrap_longitude(e.west + e.span / 2.0),
                (e.south + e.north) / 2.0,
                e.span,
            )
        }
        None => (0.0, 0.0, 360.0),
    };
    let lon0 = opts.center_lon.unwrap_or(lon0);
    let equal_earth = match opts.projection {
        ProjectionChoice::Auto => anchor.is_none() || span > 200.0,
        ProjectionChoice::Laea => false,
        ProjectionChoice::EqualEarth => true,
    };
    if equal_earth {
        MapProjection::EqualEarth(EqualEarth { lon0 })
    } else {
        MapProjection::Laea(LambertAzimuthalEqualArea { lon0, lat0 })
    }
}

/// Lon/lat extent that is aware of the antimeridian.
struct GeoExtent {
    west: f64,
    span: f64,
    south: f64,
    north: f64,
}

impl GeoExtent {
    fn of(mp: &MultiPolygon<f64>) -> GeoExtent {
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

    /// Cheap pre-filter: could `g` be visible in a frame around this extent?
    fn near(&self, g: &MultiPolygon<f64>) -> bool {
        let Some(b) = g.bounding_rect() else {
            return false;
        };
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

fn sphere_outline(p: &MapProjection) -> Polygon<f64> {
    let lon0 = p.lon0();
    let edge = |lon: f64, lats: &mut dyn Iterator<Item = f64>| -> Vec<_> {
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

fn cull(mp: &MultiPolygon<f64>, min_area: f64, keep_largest: bool) -> MultiPolygon<f64> {
    if min_area <= 0.0 || mp.0.is_empty() {
        return mp.clone();
    }
    let areas: Vec<f64> = mp.0.iter().map(|p| p.unsigned_area()).collect();
    let largest = (0..areas.len())
        .max_by(|&a, &b| areas[a].total_cmp(&areas[b]).then(b.cmp(&a)))
        .unwrap_or(0);
    MultiPolygon(
        mp.0.iter()
            .enumerate()
            .filter(|&(i, _)| areas[i] >= min_area || (keep_largest && i == largest))
            .map(|(_, p)| p.clone())
            .collect(),
    )
}

fn fit_viewport(frame: Rect<f64>, width: u32, padding: u32) -> Result<Viewport> {
    let (dx, dy) = (frame.width(), frame.height());
    let pad = f64::from(padding);
    let inner_w = f64::from(width) - 2.0 * pad;
    if dx <= 0.0 || dy <= 0.0 || inner_w <= 0.0 {
        return Err(Error::DegenerateExtent);
    }
    let scale = inner_w / dx;
    Ok(Viewport {
        min_x: frame.min().x,
        max_y: frame.max().y,
        scale,
        padding: pad,
        width,
        height: (dy * scale + 2.0 * pad).ceil() as u32,
    })
}
