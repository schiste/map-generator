//! Deterministic geometry pipeline for `map-generator`.
//!
//! The pipeline is a pure function of its inputs: the same features and
//! options always produce byte-identical SVG output.
//!
//! ```text
//! features (WGS84) ─► frame ─► projection ─► seam split ─► project
//!     ─► shared-border simplification ─► clip ─► SVG (themed layers)
//! ```

pub mod antimeridian;
#[cfg(feature = "proj")]
pub mod epsg;
pub mod error;
pub mod feature;
pub mod frame;
pub mod html;
pub mod labels;
pub mod layout;
mod math;
pub mod panel;
pub mod pipeline;
pub mod projection;
pub mod simplify;
pub mod svg;
pub mod theme;
pub mod units;
pub mod validate;

pub use error::{Error, Result};
pub use feature::{MapFeature, MapLine, MapPlace, PlaceKind};
pub use frame::{FrameMode, GeoBBox};
pub use html::html_page;
pub use layout::LegendSlot;
pub use pipeline::{
    render, BorderMode, Capitals, InsetInfo, InsetMode, MapLayers, RenderOptions, Rendered, Target,
};
pub use projection::{MapProjection, Projection, ProjectionChoice};
pub use svg::CONTRACT_VERSION;
pub use theme::{Color, Theme};
