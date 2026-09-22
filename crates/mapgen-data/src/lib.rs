//! Data source adapters for `map-generator`.
//!
//! Every adapter returns [`mapgen_core::MapFeature`]s in WGS84, sorted by id.
//! Filtering happens in SQL so only the requested region is ever loaded.

pub mod error;
pub mod geojson;
mod geometry;
pub mod gpkg;

pub use error::{Error, Result};
pub use gpkg::{LayerQuery, Source};
