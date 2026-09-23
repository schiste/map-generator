//! `mapgen-server`: serves the API on `$PORT` (default 8000). Configuration
//! is read from the environment (see `Settings::from_env`).

use std::sync::Arc;

use mapgen_server::{app, AppState, Settings};

#[tokio::main]
async fn main() {
    let settings = Settings::from_env();
    let started = std::time::Instant::now();
    let state = match AppState::new(settings) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("mapgen-server: {e}");
            std::process::exit(1);
        }
    };
    let regions: usize = state
        .registry
        .datasets
        .values()
        .map(|d| d.regions.len())
        .sum();
    eprintln!(
        "mapgen-server {}: {} dataset(s), {regions} region(s), {} crosswalk(s), loaded in {:.1} s",
        mapgen_server::api::VERSION,
        state.registry.datasets.len(),
        state.registry.crosswalks.len(),
        started.elapsed().as_secs_f64()
    );
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8000);
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
        .await
        .unwrap_or_else(|e| panic!("binding port {port}: {e}"));
    eprintln!("listening on port {port}");
    axum::serve(listener, app(Arc::new(state)))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .expect("server error");
}
