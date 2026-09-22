#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("geometry decoding: {0}")]
    Geometry(#[from] geozero::error::GeozeroError),
    #[error("geojson: {0}")]
    GeoJson(#[from] ::geojson::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid SQL identifier {0:?}")]
    InvalidIdentifier(String),
    #[error("table {0:?} is not registered in gpkg_geometry_columns")]
    UnknownLayer(String),
}

pub type Result<T> = std::result::Result<T, Error>;
