//! Deterministic geometry pipeline for `map-generator`.
//!
//! The pipeline is a pure function of its inputs: the same features and
//! options always produce byte-identical SVG output.
//!
//! ```text
//! features (WGS84) ─► select projection ─► project ─► simplify ─► SVG
//! ```

pub mod antimeridian;
pub mod error;
pub mod feature;
pub mod pipeline;
pub mod projection;
pub mod simplify;
pub mod svg;

pub use error::{Error, Result};
pub use feature::MapFeature;
pub use pipeline::{render, RenderOptions};
pub use projection::{LambertAzimuthalEqualArea, Projection};
pub use svg::SvgOptions;
