//! Routes and handlers of `/api/v1`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use mapgen_core::{GeoBBox, CONTRACT_VERSION};
use mapgen_spec::{
    bbox_table, match_layer, recipe::Recipe, render_map, reshape_with, theme_table,
    CrosswalkSource, Format, Frame, InlineCrosswalk, LoadedLayer, MapOutput, MatchHint, MatchSpec,
    RenderSpec, ReshapeSpec, Sources,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use tower_http::compression::CompressionLayer;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;

use crate::cache::Cache;
use crate::error::ApiError;
use crate::query::{self, MapParams};
use crate::registry::{DatasetEntry, Provenance, Region};
use crate::AppState;

type St = State<Arc<AppState>>;
type ApiResult<T> = Result<T, ApiError>;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Set at build time (`MAPGEN_COMMIT`), for `/version`.
const COMMIT: Option<&str> = option_env!("MAPGEN_COMMIT");
const OPENAPI: &str = include_str!("../assets/openapi.json");
const CLIENT: &str = include_str!("../assets/client.js");
const BODY_LIMIT: usize = 5 * 1024 * 1024;

/// Every route, as documented in `openapi.json` (checked by a test).
pub const ROUTES: [&str; 18] = [
    "/api/v1/",
    "/api/v1/openapi.json",
    "/api/v1/client.js",
    "/api/v1/health",
    "/api/v1/version",
    "/api/v1/themes",
    "/api/v1/bbox-presets",
    "/api/v1/render-options",
    "/api/v1/datasets",
    "/api/v1/datasets/{dataset}/regions",
    "/api/v1/datasets/{dataset}/regions/{region}/features",
    "/api/v1/maps/{dataset}/{file}",
    "/api/v1/render",
    "/api/v1/match",
    "/api/v1/crosswalks",
    "/api/v1/crosswalks/{file}",
    "/api/v1/reshape",
    "/api/v1/contract/fixture.svg",
];

pub fn app(state: Arc<AppState>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([header::CONTENT_TYPE, header::IF_NONE_MATCH, header::ACCEPT])
        .expose_headers([
            header::ETAG,
            header::CONTENT_LOCATION,
            header::LINK,
            header::RETRY_AFTER,
            header::HeaderName::from_static("x-mapgen-version"),
            header::HeaderName::from_static("x-mapgen-contract"),
            header::HeaderName::from_static("x-dataset-release"),
        ])
        .max_age(Duration::from_secs(86400));
    let api = Router::new()
        .route("/api/v1", get(index))
        .route("/api/v1/", get(index))
        .route("/api/v1/openapi.json", get(openapi))
        .route("/api/v1/client.js", get(client))
        .route("/api/v1/health", get(health))
        .route("/healthz", get(health))
        .route("/api/v1/version", get(version))
        .route("/api/v1/themes", get(themes))
        .route("/api/v1/bbox-presets", get(bbox_presets))
        .route("/api/v1/render-options", get(render_options))
        .route("/api/v1/datasets", get(datasets))
        .route("/api/v1/datasets/{dataset}/regions", get(regions))
        .route(
            "/api/v1/datasets/{dataset}/regions/{region}/features",
            get(features),
        )
        .route("/api/v1/maps/{dataset}/{file}", get(map_get))
        .route("/api/v1/render", post(render_post))
        .route("/api/v1/match", post(match_post))
        .route("/api/v1/crosswalks", get(crosswalks))
        .route("/api/v1/crosswalks/{file}", get(crosswalk_file))
        .route("/api/v1/reshape", post(reshape_post))
        .route("/api/v1/contract/fixture.svg", get(contract_fixture))
        .route("/api/{*rest}", get(api_not_found).post(api_not_found));
    let router = match &state.settings.www_dir {
        Some(www) => api.fallback_service(ServeDir::new(www)),
        None => api.route("/", get(home)),
    };
    router
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .layer(CompressionLayer::new())
        .layer(cors)
        .layer(middleware::from_fn(cache_policy))
        .layer(middleware::from_fn(log))
        .with_state(state)
}

/// Maps change only with a deploy or a data release, so they're kept for
/// 30 days (clients needing the newest can revalidate with the ETag, or pin
/// `release=` for an immutable URL).
const MAPS_CACHE: &str = "public, max-age=2592000";
const DAY: &str = "public, max-age=86400";
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// `Cache-Control` for successful responses that don't set their own:
/// listings and static API files for a day; health and version never;
/// playground files under content-hashed folders (`pkg-<hash>/`,
/// `data-<hash>/`, see scripts/deploy-toolforge.sh) for a year, and the
/// rest of the playground (index.html, app.js) revalidated every time so a
/// deploy shows at once.
async fn cache_policy(req: Request, next: Next) -> Response {
    let path = req.uri().path().to_owned();
    let get = req.method() == Method::GET || req.method() == Method::HEAD;
    let mut res = next.run(req).await;
    let ok = res.status().is_success() || res.status() == StatusCode::NOT_MODIFIED;
    if !get || !ok || res.headers().contains_key(header::CACHE_CONTROL) {
        return res;
    }
    let policy = if let Some(api) = path.strip_prefix("/api/") {
        match api.trim_start_matches("v1").trim_start_matches('/') {
            "health" | "version" | "" => "no-cache",
            _ => DAY,
        }
    } else if path == "/healthz" {
        "no-cache"
    } else if path.split('/').any(is_hashed_dir) {
        IMMUTABLE
    } else {
        "no-cache"
    };
    res.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(policy));
    res
}

/// `pkg-1a2b3c4d`, `data-0f9e8d7c`: a folder named after its content.
fn is_hashed_dir(segment: &str) -> bool {
    ["pkg-", "data-"].iter().any(|p| {
        segment
            .strip_prefix(p)
            .is_some_and(|h| h.len() >= 8 && h.bytes().all(|b| b.is_ascii_hexdigit()))
    })
}

/// One line per request: method, path, status, time. No IP addresses or
/// user agents.
async fn log(req: Request, next: Next) -> Response {
    let (method, path) = (req.method().clone(), req.uri().path().to_owned());
    let started = Instant::now();
    let res = next.run(req).await;
    let cache = res
        .headers()
        .get("x-cache")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-");
    eprintln!(
        "{method} {path} {} {} ms cache={cache}",
        res.status().as_u16(),
        started.elapsed().as_millis()
    );
    res
}

async fn api_not_found() -> ApiError {
    ApiError::not_found("no such endpoint: see /api/v1/ and /api/v1/openapi.json")
}

async fn home() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        "<!doctype html><meta charset=utf-8><title>map-generator</title>\
         <h1>map-generator</h1><p>Deterministic SVG maps from open data. \
         API: <a href=/api/v1/>/api/v1/</a> · \
         <a href=https://github.com/schiste/map-generator/blob/main/docs/api.md>documentation</a> · \
         <a href=https://github.com/schiste/map-generator>source</a></p>",
    )
}

async fn index() -> Json<Value> {
    Json(json!({
        "name": "map-generator API",
        "version": VERSION,
        "contract": CONTRACT_VERSION,
        "documentation": "https://github.com/schiste/map-generator/blob/main/docs/api.md",
        "openapi": "/api/v1/openapi.json",
        "client": "/api/v1/client.js",
        "links": {
            "health": "/api/v1/health",
            "version": "/api/v1/version",
            "themes": "/api/v1/themes",
            "bboxPresets": "/api/v1/bbox-presets",
            "renderOptions": "/api/v1/render-options",
            "datasets": "/api/v1/datasets",
            "regions": "/api/v1/datasets/{dataset}/regions",
            "features": "/api/v1/datasets/{dataset}/regions/{region}/features",
            "map": "/api/v1/maps/{dataset}/{region}.{svg|json|html}",
            "render": "POST /api/v1/render",
            "match": "POST /api/v1/match",
            "crosswalks": "/api/v1/crosswalks",
            "reshape": "POST /api/v1/reshape",
            "contractFixture": "/api/v1/contract/fixture.svg"
        }
    }))
}

async fn openapi() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "application/json")], OPENAPI)
}

async fn client() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        CLIENT,
    )
}

async fn health(State(s): St) -> Json<Value> {
    Json(json!({"status": "ok", "datasets": s.registry.datasets.len()}))
}

async fn version(State(s): St) -> Json<Value> {
    let releases: Map<String, Value> = s
        .registry
        .datasets
        .iter()
        .map(|(id, d)| {
            // One file per region: each has its own release (see /regions).
            let r = if d.config.files.is_some() {
                "per region".to_owned()
            } else {
                d.regions
                    .values()
                    .next()
                    .map(|r| r.provenance.release.clone())
                    .unwrap_or_default()
            };
            (id.clone(), json!(r))
        })
        .collect();
    Json(json!({
        "mapgen": VERSION,
        "commit": COMMIT,
        "contract": CONTRACT_VERSION,
        "datasets": releases,
    }))
}

async fn themes() -> Json<Value> {
    Json(json!(theme_table()))
}

async fn bbox_presets() -> Json<Value> {
    Json(json!(bbox_table()))
}

/// Every map setting, described for settings forms (`options.rs`), with
/// this host's width limit and points of view.
async fn render_options(State(s): St) -> Json<Value> {
    let mut description = mapgen_spec::options::describe(s.settings.max_width);
    let views = s
        .registry
        .worldviews()
        .into_iter()
        .map(|(code, label)| mapgen_spec::options::Choice {
            value: json!(code),
            label,
        })
        .collect();
    description.set_choices("worldview", views);
    Json(json!(description))
}

async fn contract_fixture() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "image/svg+xml; charset=utf-8")],
        include_str!("../../mapgen-data/tests/fixtures/twin-regions.svg"),
    )
}

async fn datasets(State(s): St) -> Json<Value> {
    let list: Vec<Value> = s
        .registry
        .datasets
        .values()
        .map(|d| {
            let c = &d.config;
            let p = d
                .regions
                .values()
                .next()
                .map(|r| r.provenance.clone())
                .unwrap_or_default();
            let per_region = c.files.is_some();
            json!({
                "id": c.id,
                "title": c.title,
                "level": c.level,
                "regions": d.regions.len(),
                "world": c.world,
                "worldviews": c.worldviews,
                "languages": c.languages,
                // With one file per region, licences can differ: see /regions.
                "credit": (!per_region).then_some(&p.credit),
                "licence": (!per_region).then_some(&p.licence),
                "licenceUrl": if per_region { None } else { p.licence_url.clone() },
                "shareAlike": (!per_region).then_some(p.share_alike),
                "release": (!per_region).then_some(&p.release),
                "boundaryYear": if per_region { None } else { p.boundary_year.clone() },
                "licencePerRegion": per_region,
            })
        })
        .collect();
    Json(json!(list))
}

fn dataset<'a>(s: &'a AppState, id: &str) -> ApiResult<&'a DatasetEntry> {
    s.registry.datasets.get(id).ok_or_else(|| {
        ApiError::not_found(format!(
            "no dataset {id:?}; hosted: {}",
            s.registry
                .datasets
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })
}

fn region<'a>(d: &'a DatasetEntry, code: &str) -> ApiResult<&'a Region> {
    d.regions.get(code).ok_or_else(|| {
        ApiError::not_found(format!(
            "no region {code:?} in {}: see /api/v1/datasets/{}/regions",
            d.config.id, d.config.id
        ))
    })
}

/// One or more regions of a dataset, as one map.
struct Selection<'a> {
    regions: Vec<&'a Region>,
    /// Their codes joined by `,` (as in the canonical path).
    code: String,
    provenance: Provenance,
}

/// Resolves `FRA`, `FRA,DEU,ITA` or names (`France,Germany`) to regions.
fn select<'a>(d: &'a DatasetEntry, spec: &str) -> ApiResult<Selection<'a>> {
    // Every region an input can mean: a code, else names and aliases.
    let lookup = |input: &str| -> Vec<&'a Region> {
        if let Some(r) = d.regions.get(input).or_else(|| {
            d.regions
                .values()
                .find(|r| r.code.eq_ignore_ascii_case(input))
        }) {
            return vec![r];
        }
        let lower = input.to_lowercase();
        d.regions
            .values()
            .filter(|r| r.name.to_lowercase() == lower || r.aliases.contains(&lower))
            .collect()
    };
    let candidates = |input: &str| -> Vec<&'a Region> {
        let found = lookup(input);
        if !found.is_empty() {
            return found;
        }
        // Wikipedia titles: `New York (state)` is New York.
        mapgen_spec::wikitext::without_disambiguation(input)
            .map(lookup)
            .unwrap_or_default()
    };
    // The country a subdivision of a mixed dataset is in (`FR-59` → FR).
    let country =
        |r: &Region| (r.level > 0).then(|| r.code.split('-').next().unwrap_or("").to_owned());
    let inputs: Vec<&str> = match d.regions.get(spec) {
        Some(_) => vec![spec],
        None => spec
            .split(',')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .collect(),
    };
    let resolved: Vec<Vec<&'a Region>> = inputs.iter().map(|i| candidates(i)).collect();
    let mut shared: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for c in resolved.iter().filter(|c| c.len() == 1) {
        if let Some(k) = country(c[0]) {
            *shared.entry(k).or_default() += 1;
        }
    }
    // A name shared by several regions: a country before a subdivision
    // (Luxembourg), then the country most other inputs are in (Nord among
    // French départements is France's), then the first by code.
    let mut regions = Vec::new();
    for (input, candidates) in inputs.iter().zip(resolved) {
        let pick = candidates.into_iter().min_by_key(|r| {
            let n = country(r)
                .and_then(|k| shared.get(&k).copied())
                .unwrap_or(0);
            (r.level, std::cmp::Reverse(n), r.code.clone())
        });
        match pick {
            Some(r) => regions.push(r),
            None => {
                return Err(ApiError::not_found(format!(
                    "no region {input:?} in {}: see /api/v1/datasets/{}/regions",
                    d.config.id, d.config.id
                )))
            }
        }
    }
    regions.sort_by(|a, b| a.code.cmp(&b.code));
    regions.dedup_by(|a, b| a.code == b.code);
    if regions.is_empty() {
        return Err(ApiError::not_found("no region given"));
    }
    if regions.len() > 1 && regions.iter().any(|r| r.code == "world") {
        return Err(ApiError::bad_request(
            "`world` can't be combined with other regions",
        ));
    }
    let distinct = |f: &dyn Fn(&Region) -> String| {
        let mut v: Vec<String> = regions
            .iter()
            .map(|r| f(r))
            .filter(|x| !x.is_empty())
            .collect();
        v.sort();
        v.dedup();
        v
    };
    let single = |v: Vec<String>| {
        if v.len() == 1 {
            v.into_iter().next()
        } else {
            None
        }
    };
    let provenance = Provenance {
        credit: distinct(&|r| r.provenance.credit.clone()).join("; "),
        licence: distinct(&|r| r.provenance.licence.clone()).join("; "),
        licence_url: single(distinct(&|r| {
            r.provenance.licence_url.clone().unwrap_or_default()
        })),
        share_alike: regions.iter().any(|r| r.provenance.share_alike),
        release: distinct(&|r| r.provenance.release.clone()).join(" + "),
        boundary_year: single(distinct(&|r| {
            r.provenance.boundary_year.clone().unwrap_or_default()
        })),
    };
    Ok(Selection {
        code: regions
            .iter()
            .map(|r| r.code.as_str())
            .collect::<Vec<_>>()
            .join(","),
        regions,
        provenance,
    })
}

#[derive(Deserialize)]
struct LangQuery {
    languages: Option<String>,
}

fn languages(q: &LangQuery) -> Vec<String> {
    q.languages
        .as_deref()
        .map(|l| {
            l.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

async fn regions(State(s): St, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let d = dataset(&s, &id)?;
    let list: Vec<Value> = d
        .regions
        .values()
        .map(|r| {
            json!({
                "code": r.code,
                "name": r.name,
                "kind": match r.column.as_deref() {
                    None if r.code == "world" => "world",
                    None => "file",
                    Some("CONTINENT") => "continent",
                    Some(_) => "region",
                },
                "provenance": r.provenance,
            })
        })
        .collect();
    Ok(Json(json!(list)))
}

async fn features(
    State(s): St,
    Path((id, code)): Path<(String, String)>,
    Query(q): Query<LangQuery>,
) -> ApiResult<Response> {
    let state = s.clone();
    let langs = languages(&q);
    blocking(&s, move || {
        let d = dataset(&state, &id)?;
        let r = region(d, &code)?;
        let feats = state
            .registry
            .features(d, r, &langs, None)
            .map_err(ApiError::internal)?;
        let list: Vec<Value> = feats
            .iter()
            .map(|f| {
                json!({
                    "code": f.id,
                    "name": f.name,
                    "names": f.names,
                    "parent": f.parent,
                    "parentName": f.parent_name,
                    "country": f.country,
                    "units": f.units,
                    "wikidata": f.wikidata,
                })
            })
            .collect();
        Ok(Json(json!(list)).into_response())
    })
    .await
}

/// Runs `f` on the blocking pool, bounded by the render permits and timeout.
async fn blocking<T: Send + 'static>(
    s: &Arc<AppState>,
    f: impl FnOnce() -> ApiResult<T> + Send + 'static,
) -> ApiResult<T> {
    let permit = tokio::time::timeout(Duration::from_secs(10), s.permits.clone().acquire_owned())
        .await
        .map_err(|_| ApiError::busy("the server is busy; try again in a few seconds"))?
        .map_err(|_| ApiError::internal("shutting down"))?;
    let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        f()
    });
    match tokio::time::timeout(s.settings.render_timeout, task).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => Err(ApiError::internal(format!("render failed: {e}"))),
        Err(_) => Err(ApiError::busy(
            "the map took too long to render; try a smaller width",
        )),
    }
}

/// A rendered map with what the response needs.
struct Rendered {
    output: MapOutput,
    credit: Option<String>,
    boundary_year: Option<String>,
    source_release: Option<String>,
}

/// Renders `region` of `d` with `spec`.
fn render_selection(
    s: &AppState,
    d: &DatasetEntry,
    sel: &Selection,
    mut spec: RenderSpec,
    worldview: Option<&str>,
) -> ApiResult<Rendered> {
    if spec.region.is_some() || spec.regions.is_some() {
        return Err(ApiError::bad_param(
            "region",
            "the regions are set by the path, the `region` member or the recipe",
        ));
    }
    if spec.width.is_some_and(|w| w > s.settings.max_width) {
        return Err(ApiError::bad_param(
            "width",
            &format!("at most {} px", s.settings.max_width),
        ));
    }
    // Each region's features, tagged with its code (regions may come from
    // different files or columns).
    let mut rows = Vec::new();
    for r in &sel.regions {
        let features = s
            .registry
            .features(d, r, &spec.languages, worldview)
            .map_err(|e| {
                if e.contains("not hosted") {
                    ApiError::not_found(e)
                } else {
                    ApiError::internal(e)
                }
            })?;
        rows.extend(features.into_iter().map(|f| (Some(r.code.clone()), f)));
    }
    let credit = match worldview {
        Some(v) if d.config.worldviews => {
            format!("Natural Earth ({} view)", v.to_ascii_uppercase())
        }
        _ => sel.provenance.credit.clone(),
    };
    let subject = LoadedLayer::from_rows(rows, true, Some(credit).filter(|c| !c.is_empty()));
    let codes: Vec<String> = sel.regions.iter().map(|r| r.code.clone()).collect();
    if let [code] = codes.as_slice() {
        spec.region = Some(code.clone());
    } else {
        spec.regions = Some(codes);
    }
    // A continent gets its frame preset, as `mapgen render --continent`.
    if let [r] = sel.regions.as_slice() {
        if r.column.as_deref() == Some("CONTINENT")
            && spec.bbox.is_none()
            && spec.frame == Frame::Auto
            && GeoBBox::parse(&r.code).is_ok()
        {
            spec.bbox = Some(r.code.clone());
        }
    }
    if spec.boundary_year.is_none() {
        spec.boundary_year = sel.provenance.boundary_year.clone();
    }
    if spec.source_release.is_none() && !sel.provenance.release.is_empty() {
        spec.source_release = Some(sel.provenance.release.clone());
    }
    let countries = s
        .registry
        .countries_for(worldview)
        .map_err(ApiError::not_found)?;
    let src = Sources {
        subject: &subject,
        context: countries.as_deref(),
        lakes: s.registry.lakes.as_ref(),
        disputed_areas: s.registry.disputed_areas.as_ref(),
        disputed: s.registry.disputed.as_ref(),
        places: s.registry.places.as_deref(),
        units: None,
    };
    let credit = spec.attribution.clone().or_else(|| src.credits());
    let output = render_map(src, &spec).map_err(|e| ApiError::bad_request(e.0))?;
    Ok(Rendered {
        output,
        credit,
        boundary_year: spec.boundary_year,
        source_release: spec.source_release,
    })
}

fn metadata(d: &DatasetEntry, sel: &Selection, m: &Rendered, sha1: &str, canonical: &str) -> Value {
    let mut out = serde_json::to_value(&m.output).unwrap_or_default();
    let obj = out.as_object_mut().expect("MapOutput is an object");
    obj.remove("svg");
    obj.remove("html");
    let p = &sel.provenance;
    obj.insert("dataset".into(), json!(d.config.id));
    obj.insert("region".into(), json!(sel.code));
    obj.insert("sha1".into(), json!(sha1));
    obj.insert("credit".into(), json!(m.credit));
    obj.insert("licence".into(), json!(p.licence));
    obj.insert("licenceUrl".into(), json!(p.licence_url));
    obj.insert("shareAlike".into(), json!(p.share_alike));
    obj.insert("boundaryYear".into(), json!(m.boundary_year));
    obj.insert("sourceRelease".into(), json!(m.source_release));
    obj.insert("release".into(), json!(p.release));
    obj.insert("url".into(), json!(canonical));
    out
}

fn sha1(bytes: &[u8]) -> String {
    sha1_smol::Sha1::from(bytes).digest().to_string()
}

/// A map response: ETag (the SHA-1 Commons would store), 304, cache and
/// provenance headers.
fn map_response(
    body: Vec<u8>,
    content_type: &'static str,
    p: &Provenance,
    pinned: bool,
    canonical: &str,
    headers: &HeaderMap,
    cache_hit: bool,
) -> Response {
    let etag = format!("\"{}\"", sha1(&body));
    let not_modified = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(',')
                .any(|t| t.trim().trim_start_matches("W/") == etag)
        });
    let mut res = if not_modified {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        ([(header::CONTENT_TYPE, content_type)], body).into_response()
    };
    let h = res.headers_mut();
    let set = |h: &mut HeaderMap, k: header::HeaderName, v: &str| {
        if let Ok(v) = HeaderValue::from_str(v) {
            h.insert(k, v);
        }
    };
    set(h, header::ETAG, &etag);
    set(
        h,
        header::CACHE_CONTROL,
        if pinned { IMMUTABLE } else { MAPS_CACHE },
    );
    set(h, header::CONTENT_LOCATION, canonical);
    set(
        h,
        header::HeaderName::from_static("x-mapgen-version"),
        VERSION,
    );
    set(
        h,
        header::HeaderName::from_static("x-mapgen-contract"),
        &CONTRACT_VERSION.to_string(),
    );
    set(
        h,
        header::HeaderName::from_static("x-dataset-release"),
        &p.release,
    );
    set(
        h,
        header::HeaderName::from_static("x-cache"),
        if cache_hit { "hit" } else { "miss" },
    );
    if let Some(url) = &p.licence_url {
        set(h, header::LINK, &format!("<{url}>; rel=\"license\""));
    }
    res
}

async fn map_get(
    State(s): St,
    Path((id, file)): Path<(String, String)>,
    Query(pairs): Query<Vec<(String, String)>>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let (code, ext) = file
        .rsplit_once('.')
        .ok_or_else(|| ApiError::not_found("add the format to the path: .svg, .json or .html"))?;
    let (content_type, format) = match ext {
        "svg" => ("image/svg+xml; charset=utf-8", "svg"),
        "json" => ("application/json", "json"),
        "html" => ("text/html; charset=utf-8", "html"),
        e => {
            return Err(ApiError::not_found(format!(
                "unknown format .{e}: use .svg, .json or .html"
            )))
        }
    };
    let params = query::parse(&pairs)?;
    check_worldview(params.worldview.as_deref())?;
    let d = dataset(&s, &id)?;
    let sel = select(d, code)?;
    let pinned = match &params.release {
        Some(rel) if *rel != sel.provenance.release => {
            return Err(ApiError::not_found(format!(
                "release {rel:?} of {id} is not hosted (current: {:?}); drop `release` for the current one",
                sel.provenance.release
            )))
        }
        Some(_) => true,
        None => false,
    };
    // Canonical: region codes sorted, whatever order or spelling was asked.
    let canonical = {
        let q = query::canonical(&params);
        let base = format!(
            "/api/v1/maps/{}/{}.{format}",
            query::encode(&id),
            query::encode(&sel.code)
        );
        if q.is_empty() {
            base
        } else {
            format!("{base}?{q}")
        }
    };
    // The build commit too: a dependency update can change output without
    // changing the crate version.
    let key = Cache::key(&[
        VERSION,
        COMMIT.unwrap_or("dev"),
        &id,
        &sel.provenance.release,
        &canonical,
    ]);
    if let Some(body) = s.cache.get(&key) {
        return Ok(map_response(
            body,
            content_type,
            &sel.provenance,
            pinned,
            &canonical,
            &headers,
            true,
        ));
    }
    let mut spec = spec_from(&params, true)?;
    if format == "html" {
        spec.format = Format::Html;
    }
    let state = s.clone();
    let (id2, code2, canonical2) = (id.clone(), sel.code.clone(), canonical.clone());
    let body = blocking(&s, move || {
        let d = dataset(&state, &id2)?;
        let sel = select(d, &code2)?;
        let m = render_selection(&state, d, &sel, spec, params.worldview.as_deref())?;
        let svg_sha1 = sha1(m.output.svg.as_bytes());
        Ok(match format {
            "svg" => m.output.svg.clone().into_bytes(),
            "html" => m.output.html.clone().unwrap_or_default().into_bytes(),
            _ => serde_json::to_vec_pretty(&metadata(d, &sel, &m, &svg_sha1, &canonical2))
                .unwrap_or_default(),
        })
    })
    .await?;
    s.cache.put(&key, &body);
    Ok(map_response(
        body,
        content_type,
        &sel.provenance,
        pinned,
        &canonical,
        &headers,
        false,
    ))
}

/// A Natural Earth point of view that exists (hosting is checked later).
fn check_worldview(view: Option<&str>) -> ApiResult<()> {
    match view {
        Some(v)
            if !mapgen_data::NATURAL_EARTH_WORLDVIEWS
                .contains(&v.to_ascii_uppercase().as_str()) =>
        {
            Err(ApiError::bad_param(
                "worldview",
                &format!(
                    "unknown point of view {v:?}; Natural Earth has {}",
                    mapgen_data::NATURAL_EARTH_WORLDVIEWS.join(", ")
                ),
            ))
        }
        _ => Ok(()),
    }
}

fn spec_from(params: &MapParams, from_query: bool) -> ApiResult<RenderSpec> {
    serde_json::from_value(Value::Object(params.spec.clone()))
        .map_err(|e| ApiError::spec(&e.to_string(), from_query))
}

/// `POST /render` body.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RenderRequest {
    dataset: String,
    region: String,
    worldview: Option<String>,
    #[serde(default)]
    spec: Map<String, Value>,
}

async fn render_post(State(s): St, headers: HeaderMap, body: Bytes) -> ApiResult<Response> {
    let is_csv = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.starts_with("text/csv") || t.starts_with("text/plain"));
    // A JSON request, or a map recipe (docs/recipes.md).
    let (dataset_id, regions, view, spec) = if is_csv {
        let text = std::str::from_utf8(&body)
            .map_err(|_| ApiError::bad_request("the recipe is not UTF-8"))?;
        let recipe =
            Recipe::parse(text).map_err(|e| ApiError::bad_request(format!("recipe: {}", e.0)))?;
        let spec = recipe
            .spec()
            .map_err(|e| ApiError::bad_request(format!("recipe: {}", e.0)))?;
        let dataset = recipe.dataset.clone().ok_or_else(|| {
            ApiError::bad_param(
                "dataset",
                "the recipe needs a `dataset` row (e.g. countries)",
            )
        })?;
        if recipe.regions.is_empty() {
            return Err(ApiError::bad_param("region", "the recipe lists no region"));
        }
        (
            dataset,
            recipe.regions.join(","),
            recipe.worldview.clone(),
            spec,
        )
    } else {
        let req: RenderRequest =
            serde_json::from_slice(&body).map_err(|e| ApiError::bad_request(e.to_string()))?;
        let spec: RenderSpec = serde_json::from_value(Value::Object(req.spec.clone()))
            .map_err(|e| ApiError::spec(&format!("spec: {e}"), false))?;
        (req.dataset, req.region, req.worldview, spec)
    };
    check_worldview(view.as_deref())?;
    let wants_json = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|a| a.contains("application/json"));
    let html = spec.format == Format::Html;
    let state = s.clone();
    let (body, content_type, release) = blocking(&s, move || {
        let d = dataset(&state, &dataset_id)?;
        let sel = select(d, &regions)?;
        let m = render_selection(&state, d, &sel, spec, view.as_deref())?;
        let svg_sha1 = sha1(m.output.svg.as_bytes());
        let release = sel.provenance.release.clone();
        if wants_json {
            let mut meta = metadata(d, &sel, &m, &svg_sha1, "");
            meta["svg"] = json!(m.output.svg);
            if let Some(h) = &m.output.html {
                meta["html"] = json!(h);
            }
            Ok((
                serde_json::to_vec(&meta).unwrap_or_default(),
                "application/json",
                release,
            ))
        } else if html {
            Ok((
                m.output.html.unwrap_or_default().into_bytes(),
                "text/html; charset=utf-8",
                release,
            ))
        } else {
            Ok((
                m.output.svg.into_bytes(),
                "image/svg+xml; charset=utf-8",
                release,
            ))
        }
    })
    .await?;
    let etag = format!("\"{}\"", sha1(&body));
    let mut res = ([(header::CONTENT_TYPE, content_type)], body).into_response();
    if let Ok(v) = HeaderValue::from_str(&etag) {
        res.headers_mut().insert(header::ETAG, v);
    }
    res.headers_mut()
        .insert("x-mapgen-version", HeaderValue::from_static(VERSION));
    if let Ok(v) = HeaderValue::from_str(&release) {
        res.headers_mut().insert("x-dataset-release", v);
    }
    Ok(res)
}

async fn match_post(State(s): St, body: Bytes) -> ApiResult<Json<Value>> {
    let mut obj: Map<String, Value> =
        serde_json::from_slice(&body).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let id = match obj.remove("dataset") {
        Some(Value::String(id)) => id,
        _ => {
            return Err(ApiError::bad_param(
                "dataset",
                "required: a dataset id from /api/v1/datasets",
            ))
        }
    };
    let spec: MatchSpec = serde_json::from_value(Value::Object(obj))
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let code = spec.region.clone().ok_or_else(|| {
        ApiError::bad_param(
            "region",
            "required: a region from /api/v1/datasets/{dataset}/regions",
        )
    })?;
    let state = s.clone();
    let out = blocking(&s, move || {
        let d = dataset(&state, &id)?;
        let r = region(d, &code)?;
        let feats = state
            .registry
            .features(d, r, &[], None)
            .map_err(ApiError::internal)?;
        let layer =
            LoadedLayer::from_rows(feats.into_iter().map(|f| (None, f)).collect(), false, None);
        let spec = MatchSpec {
            region: None,
            ..spec
        };
        let mut out = match_layer(&layer, &spec).map_err(|e| ApiError::bad_request(e.0))?;
        out.hints = hints(&state, &out.data_not_on_map, &spec.code_prefix);
        Ok(out)
    })
    .await?;
    Ok(Json(json!(out)))
}

/// Hosted crosswalks that know codes missing from the map.
fn hints(s: &AppState, missing: &[String], prefix: &str) -> Vec<MatchHint> {
    let bare: Vec<&str> = missing
        .iter()
        .map(|c| c.strip_prefix(prefix).unwrap_or(c))
        .collect();
    let mut out = Vec::new();
    for cw in s.registry.crosswalks.values() {
        let from: std::collections::BTreeSet<&str> =
            cw.rows.iter().map(|r| r.from.as_str()).collect();
        let to: std::collections::BTreeSet<&str> = cw.rows.iter().map(|r| r.to.as_str()).collect();
        let pick = |side: &std::collections::BTreeSet<&str>,
                    other: &std::collections::BTreeSet<&str>|
         -> Vec<String> {
            bare.iter()
                .filter(|c| side.contains(**c) && !other.contains(**c))
                .map(|c| format!("{prefix}{c}"))
                .collect()
        };
        let title = cw
            .meta
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or(&cw.id)
            .to_owned();
        let old = pick(&from, &to);
        if !old.is_empty() {
            out.push(MatchHint {
                crosswalk: cw.id.clone(),
                direction: "old-data",
                message: format!(
                    "{} code(s) are old codes of {title}: reshape the data with crosswalk {:?}",
                    old.len(),
                    cw.id
                ),
                codes: old,
            });
        }
        let new = pick(&to, &from);
        if !new.is_empty() {
            out.push(MatchHint {
                crosswalk: cw.id.clone(),
                direction: "new-data",
                message: format!(
                    "{} code(s) are new codes of {title}: the map's boundaries are older than the data",
                    new.len()
                ),
                codes: new,
            });
        }
    }
    out
}

async fn crosswalks(State(s): St) -> Json<Value> {
    Json(json!(s
        .registry
        .crosswalks
        .values()
        .map(|c| {
            let mut m = c.meta.clone();
            m["table"] = json!(format!("/api/v1/crosswalks/{}.csv", c.id));
            m
        })
        .collect::<Vec<_>>()))
}

async fn crosswalk_file(State(s): St, Path(file): Path<String>) -> ApiResult<Response> {
    let (id, ext) = file.rsplit_once('.').unwrap_or((&file, ""));
    let cw = s.registry.crosswalks.get(id).ok_or_else(|| {
        ApiError::not_found(format!("no crosswalk {id:?}: see /api/v1/crosswalks"))
    })?;
    match ext {
        "csv" => Ok((
            [(header::CONTENT_TYPE, "text/csv; charset=utf-8")],
            cw.table.clone(),
        )
            .into_response()),
        "json" | "" => Ok(Json(cw.meta.clone()).into_response()),
        e => Err(ApiError::not_found(format!(
            "unknown format .{e}: use .csv or .json"
        ))),
    }
}

async fn reshape_post(State(s): St, headers: HeaderMap, body: Bytes) -> ApiResult<Response> {
    let spec: ReshapeSpec =
        serde_json::from_slice(&body).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let crosswalk = match &spec.crosswalk {
        CrosswalkSource::Inline(c) => c.clone(),
        CrosswalkSource::Hosted(id) => {
            let cw = s.registry.crosswalks.get(id).ok_or_else(|| {
                ApiError::bad_param(
                    "crosswalk",
                    &format!("no hosted crosswalk {id:?}: see /api/v1/crosswalks"),
                )
            })?;
            InlineCrosswalk {
                table: cw.table.clone(),
                from_column: "from".into(),
                to_column: "to".into(),
                weight_column: Some("weight".into()),
            }
        }
    };
    let wants_csv = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|a| a.starts_with("text/csv"));
    let out = blocking(&s, move || {
        reshape_with(&spec, &crosswalk).map_err(|e| ApiError::bad_request(e.0))
    })
    .await?;
    if out.csv.is_none() {
        return Err(ApiError {
            extra: Some(Box::new(("conflicts", json!(out.conflicts)))),
            ..ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "Values need a decision",
                format!(
                    "{} value(s) can't be carried over without a rule (listed in `conflicts`): add weights to \
                     the crosswalk, or pass allowConflicts to leave them out",
                    out.conflicts.len()
                ),
            )
        });
    }
    if wants_csv {
        return Ok((
            [(header::CONTENT_TYPE, "text/csv; charset=utf-8")],
            out.csv.unwrap_or_default(),
        )
            .into_response());
    }
    Ok(Json(json!(out)).into_response())
}
