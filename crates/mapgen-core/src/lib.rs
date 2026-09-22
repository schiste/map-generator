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
pub mod error;
pub mod feature;
pub mod frame;
pub mod html;
pub mod pipeline;
pub mod projection;
pub mod simplify;
pub mod svg;
pub mod theme;

pub use error::{Error, Result};
pub use feature::MapFeature;
pub use frame::{FrameMode, GeoBBox};
pub use html::html_page;
pub use pipeline::{render, MapLayers, RenderOptions, Rendered};
pub use projection::{MapProjection, Projection, ProjectionChoice};
pub use theme::{Color, Theme};
