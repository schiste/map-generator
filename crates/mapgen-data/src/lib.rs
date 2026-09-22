//! Data source adapters for `map-generator`.
//!
//! Every adapter returns [`mapgen_core::MapFeature`]s in WGS84, sorted by id.
//! GeoPackage filters run in SQL, so only the requested region is loaded.

pub mod error;
pub mod geojson;
mod geometry;
pub mod gpkg;
mod layer;

pub use error::{Error, Result};
pub use layer::{list_regions, read_grouped, read_layer, Format, LayerQuery, Source};
