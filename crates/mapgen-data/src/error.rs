#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[cfg(feature = "gpkg")]
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[cfg(feature = "gpkg")]
    #[error("geometry decoding: {0}")]
    Geometry(#[from] geozero::error::GeozeroError),
    #[error("GeoPackage support is not compiled in (enable the `gpkg` feature)")]
    GeoPackageUnsupported,
    #[error("geojson: {0}")]
    GeoJson(Box<::geojson::Error>),
    #[error("invalid GeoJSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid SQL identifier {0:?}")]
    InvalidIdentifier(String),
    #[error("table {0:?} is not registered in gpkg_geometry_columns")]
    UnknownLayer(String),
    #[error("a GeoPackage layer needs a table name (use --table)")]
    MissingTable,
    #[error("unsupported file type {0:?} (expected .gpkg, .geojson or .json)")]
    UnsupportedFormat(String),
    #[error("{0} has no filter column; cannot list regions")]
    NoFilterColumn(String),
}

impl From<::geojson::Error> for Error {
    fn from(e: ::geojson::Error) -> Self {
        Error::GeoJson(Box::new(e))
    }
}

pub type Result<T> = std::result::Result<T, Error>;
