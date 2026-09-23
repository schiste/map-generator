//! Query strings: the shared parameter vocabulary (`mapgen_spec::params`),
//! with the region and format taken from the path.

pub use mapgen_spec::params::{camel, canonical, encode, MapParams};

use crate::error::ApiError;

pub fn parse(pairs: &[(String, String)]) -> Result<MapParams, ApiError> {
    mapgen_spec::params::parse(pairs, &mapgen_spec::params::PATH_PARAMS)
        .map_err(|e| ApiError::bad_param(&e.param, &e.why))
}
