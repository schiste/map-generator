/// Errors produced by the geometry pipeline.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no features to render")]
    Empty,
    #[error("projected extent is degenerate (zero width or height)")]
    DegenerateExtent,
    #[error("invalid colour {0:?}")]
    InvalidColor(String),
    #[error("unknown colour slot {0:?} (expected one of: {1})")]
    UnknownColorSlot(String, String),
    #[error("projection: {0}")]
    Projection(String),
    #[error("EPSG projections need the `proj` feature (cargo build --features proj)")]
    ProjUnavailable,
    #[error("invalid bounding box {0:?}: expected `west,south,east,north` or a preset ({1})")]
    InvalidBBox(String, String),
}

pub type Result<T> = std::result::Result<T, Error>;
