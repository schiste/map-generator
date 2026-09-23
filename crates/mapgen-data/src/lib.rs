//! Data source adapters for `map-generator`.
//!
//! Every adapter returns [`mapgen_core::MapFeature`]s in WGS84, sorted by id.
//! GeoPackage filters run in SQL, so only the requested region is loaded.
//! Without the default `gpkg` feature (e.g. on wasm32), only GeoJSON is supported.

pub mod crosswalk;
pub mod error;
pub mod geojson;
mod geometry;
#[cfg(feature = "gpkg")]
pub mod gpkg;
#[cfg(feature = "gpkg")]
pub mod gpkg_write;
pub mod iso;
mod layer;
pub mod table;

pub use error::{Error, Result};
pub use layer::{
    group_rows, list_regions, read_grouped, read_layer, read_layer_in, read_layer_str,
    worldview_path, Format, LayerQuery, Source, NATURAL_EARTH_WORLDVIEWS,
};
