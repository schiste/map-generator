use geo::{BoundingRect, MapCoords};
use geo_types::Rect;

use crate::error::{Error, Result};
use crate::feature::MapFeature;
use crate::projection::{select_projection, Projection};
use crate::simplify::{simplify, vw_epsilon};
use crate::svg::{write_svg, SvgOptions, Viewport};

/// End-to-end rendering options.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderOptions {
    pub svg: SvgOptions,
    /// Simplification tolerance in output pixels. `0.0` disables simplification.
    pub simplify_px: f64,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            svg: SvgOptions::default(),
            simplify_px: 0.5,
        }
    }
}

/// Renders WGS84 features to an SVG string.
///
/// Features are sorted by `id` first, so input order never affects output.
pub fn render(features: &[MapFeature], opts: &RenderOptions) -> Result<String> {
    if features.is_empty() {
        return Err(Error::Empty);
    }
    let mut features = features.to_vec();
    features.sort_by(|a, b| a.id.cmp(&b.id));

    let projection = select_projection(&features);
    for f in &mut features {
        f.geometry = f.geometry.map_coords(|c| {
            let (x, y) = projection.project(c.x, c.y);
            geo_types::coord! { x: x, y: y }
        });
    }

    let extent = projected_extent(&features).ok_or(Error::Empty)?;
    let viewport = fit_viewport(extent, &opts.svg)?;

    if opts.simplify_px > 0.0 {
        let epsilon = vw_epsilon(opts.simplify_px, viewport.scale);
        for f in &mut features {
            f.geometry = simplify(&f.geometry, epsilon);
        }
    }

    Ok(write_svg(&features, viewport, &opts.svg))
}

fn projected_extent(features: &[MapFeature]) -> Option<Rect<f64>> {
    features
        .iter()
        .filter_map(|f| f.geometry.bounding_rect())
        .reduce(|a, b| {
            Rect::new(
                geo_types::coord! { x: a.min().x.min(b.min().x), y: a.min().y.min(b.min().y) },
                geo_types::coord! { x: a.max().x.max(b.max().x), y: a.max().y.max(b.max().y) },
            )
        })
}

fn fit_viewport(extent: Rect<f64>, opts: &SvgOptions) -> Result<Viewport> {
    let (dx, dy) = (extent.width(), extent.height());
    if dx <= 0.0 || dy <= 0.0 {
        return Err(Error::DegenerateExtent);
    }
    let padding = f64::from(opts.padding);
    let inner_w = f64::from(opts.width) - 2.0 * padding;
    let scale = inner_w / dx;
    let height = (dy * scale + 2.0 * padding).ceil() as u32;
    Ok(Viewport {
        min_x: extent.min().x,
        max_y: extent.max().y,
        scale,
        padding,
        width: opts.width,
        height,
    })
}
