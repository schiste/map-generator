//! map-generator's public HTTP API, v1 (`docs/api.md`, `openapi.json`).
//!
//! The same render options as the WebAssembly build (`mapgen-spec`), so the
//! CLI, WASM and the API give byte-identical maps. Read-only and anonymous:
//! no cookies, no user data stored, request logs without IP addresses.

pub mod api;
pub mod cache;
pub mod error;
pub mod query;
pub mod registry;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Semaphore;

pub use api::app;

/// Server settings, from the environment.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Holds `datasets.toml`, the data files and `crosswalks/`.
    pub data_dir: PathBuf,
    /// Disk cache of rendered maps; `None` disables it.
    pub cache_dir: Option<PathBuf>,
    pub cache_max_bytes: u64,
    /// Renders at a time; more wait (up to 10 s) or get `503`.
    pub max_concurrent: usize,
    pub render_timeout: Duration,
    /// Static site served at `/` (the WebAssembly playground).
    pub www_dir: Option<PathBuf>,
    pub max_width: u32,
}

impl Settings {
    /// `MAPGEN_DATA_DIR` (default `data`), `MAPGEN_CACHE_DIR`,
    /// `MAPGEN_CACHE_MAX_BYTES` (default 2 GB), `MAPGEN_MAX_CONCURRENT`
    /// (default 2), `MAPGEN_RENDER_TIMEOUT` (seconds, default 20),
    /// `MAPGEN_WWW_DIR`, `MAPGEN_MAX_WIDTH` (default 4000).
    pub fn from_env() -> Settings {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let num = |k: &str, d: u64| var(k).and_then(|v| v.parse().ok()).unwrap_or(d);
        Settings {
            data_dir: var("MAPGEN_DATA_DIR").map_or_else(|| PathBuf::from("data"), PathBuf::from),
            cache_dir: var("MAPGEN_CACHE_DIR").map(PathBuf::from),
            cache_max_bytes: num("MAPGEN_CACHE_MAX_BYTES", 2_000_000_000),
            max_concurrent: num("MAPGEN_MAX_CONCURRENT", 2).max(1) as usize,
            render_timeout: Duration::from_secs(num("MAPGEN_RENDER_TIMEOUT", 20)),
            www_dir: var("MAPGEN_WWW_DIR").map(PathBuf::from),
            max_width: num("MAPGEN_MAX_WIDTH", 4000) as u32,
        }
    }
}

pub struct AppState {
    pub registry: registry::Registry,
    pub cache: cache::Cache,
    pub permits: Arc<Semaphore>,
    pub settings: Settings,
}

impl AppState {
    pub fn new(settings: Settings) -> Result<AppState, String> {
        let registry = registry::Registry::load(&settings.data_dir)?;
        AppState::with_registry(settings, registry)
    }

    pub fn with_registry(
        settings: Settings,
        registry: registry::Registry,
    ) -> Result<AppState, String> {
        Ok(AppState {
            cache: cache::Cache::new(settings.cache_dir.clone(), settings.cache_max_bytes)
                .map_err(|e| format!("cache: {e}"))?,
            permits: Arc::new(Semaphore::new(settings.max_concurrent)),
            registry,
            settings,
        })
    }
}
