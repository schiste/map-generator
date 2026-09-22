/// Errors produced by the geometry pipeline.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no features to render")]
    Empty,
    #[error("projected extent is degenerate (zero width or height)")]
    DegenerateExtent,
}

pub type Result<T> = std::result::Result<T, Error>;
