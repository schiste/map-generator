//! The API end to end, on the twin-regions fixture (no downloads needed).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use mapgen_server::{app, AppState, Settings};
use serde_json::Value;
use tower::ServiceExt;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../mapgen-data/tests/fixtures/twin-regions.geojson"
);

/// A data directory with the fixture as a one-file dataset (region
/// `world`), the same file as a per-region dataset (`twin-files/XA.geojson`),
/// and a hosted crosswalk.
fn data_dir(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("mapgen-server-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("twin-files")).unwrap();
    std::fs::create_dir_all(dir.join("crosswalks")).unwrap();
    std::fs::copy(FIXTURE, dir.join("twin.geojson")).unwrap();
    std::fs::copy(FIXTURE, dir.join("twin-files/XA.geojson")).unwrap();
    std::fs::write(
        dir.join("twin-files/XA.license.json"),
        r#"{"license": "CC BY-SA 4.0", "source": "Test survey", "via": "tests", "year": "2020", "release": "r1"}"#,
    )
    .unwrap();
    // A second country for multi-region maps, with another licence.
    std::fs::write(
        dir.join("twin-files/XB.geojson"),
        r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{"id":"XB-01","name":"Bland"},"geometry":{"type":"Polygon","coordinates":[[[11,45],[12,45],[12,46],[11,46],[11,45]]]}}]}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("twin-files/XB.license.json"),
        r#"{"license": "CC0", "source": "Other survey", "year": "2020", "release": "r2"}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("crosswalks/twin-old-new.csv"),
        "from,to,weight\nXA-00,XA-01,1\nXA-02,XA-09,1\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("crosswalks/twin-old-new.json"),
        r#"{"id": "twin-old-new", "title": "Twin, old to new"}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("datasets.toml"),
        r#"
[[dataset]]
id = "twin"
title = "Twin regions"
preset = "custom"
file = "twin.geojson"
world = true
credit = "Test data (CC0)"
licence = "CC0"
licence_url = "https://creativecommons.org/publicdomain/zero/1.0/"
release = "2024-01"

[[dataset]]
id = "twin-files"
title = "Twin regions, one file per region"
preset = "custom"
files = "twin-files/{region}.geojson"
"#,
    )
    .unwrap();
    dir
}

fn server(name: &str) -> (Router, PathBuf) {
    server_with(name, false)
}

/// With `www`: a playground with hashed folders, as deploy-toolforge.sh makes.
fn server_with(name: &str, www: bool) -> (Router, PathBuf) {
    let dir = data_dir(name);
    let www_dir = www.then(|| {
        let w = dir.join("www");
        for (f, body) in [
            ("index.html", "<!doctype html>"),
            ("app.js", "import './pkg-0123abcd/mapgen_wasm.js';"),
            ("pkg-0123abcd/mapgen_wasm.js", "export {}"),
            ("data-89abcdef/countries.geojson", "{}"),
            ("pkg/unhashed.js", "export {}"),
        ] {
            let path = w.join(f);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
        w
    });
    let settings = Settings {
        data_dir: dir.clone(),
        cache_dir: Some(dir.join("cache")),
        cache_max_bytes: 10_000_000,
        max_concurrent: 2,
        render_timeout: Duration::from_secs(20),
        www_dir,
        max_width: 4000,
    };
    (app(Arc::new(AppState::new(settings).unwrap())), dir)
}

struct Reply {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: Vec<u8>,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&self.body)))
    }
    fn text(&self) -> String {
        String::from_utf8(self.body.clone()).unwrap()
    }
    fn header(&self, h: &str) -> &str {
        self.headers
            .get(h)
            .map(|v| v.to_str().unwrap())
            .unwrap_or("")
    }
}

async fn send(app: &Router, req: Request<Body>) -> Reply {
    let res = app.clone().oneshot(req).await.unwrap();
    let (parts, body) = res.into_parts();
    Reply {
        status: parts.status,
        headers: parts.headers,
        body: body.collect().await.unwrap().to_bytes().to_vec(),
    }
}

async fn get(app: &Router, uri: &str) -> Reply {
    send(app, Request::get(uri).body(Body::empty()).unwrap()).await
}

async fn post(app: &Router, uri: &str, body: &str) -> Reply {
    send(
        app,
        Request::post(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_owned()))
            .unwrap(),
    )
    .await
}

fn sha1(b: &[u8]) -> String {
    sha1_smol::Sha1::from(b).digest().to_string()
}

#[tokio::test]
async fn discovery_endpoints() {
    let (app, dir) = server("discovery");
    assert_eq!(get(&app, "/api/v1/health").await.json()["status"], "ok");
    let v = get(&app, "/api/v1/version").await.json();
    assert_eq!(v["contract"], 1);
    assert_eq!(v["datasets"]["twin"], "2024-01");
    assert_eq!(v["datasets"]["twin-files"], "per region");
    let ds = get(&app, "/api/v1/datasets").await.json();
    assert_eq!(ds.as_array().unwrap().len(), 2);
    assert_eq!(ds[0]["licence"], "CC0");
    assert_eq!(ds[1]["licencePerRegion"], true);
    assert_eq!(ds[1]["regions"], 2);
    let regions = get(&app, "/api/v1/datasets/twin-files/regions")
        .await
        .json();
    assert_eq!(regions[0]["code"], "XA");
    assert_eq!(regions[0]["provenance"]["shareAlike"], true);
    assert_eq!(
        regions[0]["provenance"]["credit"],
        "Test survey (CC BY-SA 4.0) via tests"
    );
    let feats = get(&app, "/api/v1/datasets/twin/regions/world/features")
        .await
        .json();
    let codes: Vec<&str> = feats
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["XA-01", "XA-02"]);
    assert!(get(&app, "/api/v1/themes").await.json()["wikimedia"].is_object());
    assert!(get(&app, "/api/v1/bbox-presets").await.json()["europe"].is_array());
    let client = get(&app, "/api/v1/client.js").await;
    assert!(
        client.header("content-type").starts_with("text/javascript")
            && client.text().contains("export class MapgenClient")
    );
    assert!(get(&app, "/api/v1/contract/fixture.svg")
        .await
        .text()
        .contains("data-mapgen-contract=\"1\""));
    assert!(get(&app, "/").await.text().contains("/api/v1/"));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn openapi_documents_every_route() {
    let (app, dir) = server("openapi");
    let doc = get(&app, "/api/v1/openapi.json").await.json();
    let mut documented: Vec<String> = doc["paths"].as_object().unwrap().keys().cloned().collect();
    documented.sort();
    let mut routes: Vec<String> = mapgen_server::api::ROUTES
        .iter()
        .map(|s| s.to_string())
        .collect();
    routes.sort();
    assert_eq!(documented, routes);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn maps_are_deterministic_cached_and_conditional() {
    let (app, dir) = server("maps");
    let uri = "/api/v1/maps/twin/world.svg?width=400&labels=true&title=Twin%20regions";
    let a = get(&app, uri).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text());
    assert_eq!(a.header("content-type"), "image/svg+xml; charset=utf-8");
    assert_eq!(a.header("x-cache"), "miss");
    let svg = a.text();
    assert!(
        svg.contains("data-mapgen-contract=\"1\"")
            && svg.contains("data-source-release=\"2024-01\"")
    );
    assert!(svg.contains("<desc id=\"attribution\">Test data (CC0)</desc>"));
    assert_eq!(a.header("etag"), format!("\"{}\"", sha1(&a.body)));
    assert_eq!(
        a.header("link"),
        "<https://creativecommons.org/publicdomain/zero/1.0/>; rel=\"license\""
    );
    assert_eq!(
        a.header("content-location"),
        "/api/v1/maps/twin/world.svg?labels=true&title=Twin%20regions&width=400"
    );
    // Same parameters in another order: same canonical request, from the cache.
    let b = get(
        &app,
        "/api/v1/maps/twin/world.svg?title=Twin+regions&labels=1&width=400",
    )
    .await;
    assert_eq!((b.header("x-cache"), b.body == a.body), ("hit", true));
    let c = send(
        &app,
        Request::get(uri)
            .header(header::IF_NONE_MATCH, a.header("etag"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(c.status, StatusCode::NOT_MODIFIED);
    // The JSON body gives the same map by POST.
    let p = post(&app, "/api/v1/render", r#"{"dataset":"twin","region":"world","spec":{"width":400,"labels":true,"title":"Twin regions"}}"#).await;
    assert_eq!(p.text(), svg);
    // Metadata.
    let m = get(
        &app,
        "/api/v1/maps/twin/world.json?width=400&labels=true&title=Twin%20regions",
    )
    .await
    .json();
    assert_eq!(m["sha1"], sha1(svg.as_bytes()));
    assert_eq!(
        (
            m["width"].as_u64(),
            m["legendSlots"].as_array().map(Vec::len)
        ),
        (Some(400), Some(9))
    );
    assert_eq!(
        (m["licence"].as_str(), m["release"].as_str()),
        (Some("CC0"), Some("2024-01"))
    );
    // Pinned releases are immutable; others aren't hosted.
    let pinned = get(&app, "/api/v1/maps/twin/world.svg?release=2024-01").await;
    assert!(pinned.header("cache-control").contains("immutable"));
    assert_eq!(
        get(&app, "/api/v1/maps/twin/world.svg?release=2019")
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    // HTML and per-region files.
    assert!(get(&app, "/api/v1/maps/twin/world.html")
        .await
        .text()
        .contains("<svg"));
    let f = get(&app, "/api/v1/maps/twin-files/XA.svg").await;
    assert!(
        f.text()
            .contains("Test survey (CC BY-SA 4.0) via tests; boundaries as of 2020"),
        "{}",
        f.text()
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn errors_are_problem_json() {
    let (app, dir) = server("errors");
    for (uri, status, needle) in [
        (
            "/api/v1/maps/twin/world.svg?colour-water=red",
            400,
            "unknown field `colour-water`",
        ),
        (
            "/api/v1/maps/twin/world.svg?width=wide",
            400,
            "not a whole number",
        ),
        (
            "/api/v1/maps/twin/world.svg?width=5000",
            400,
            "at most 4000",
        ),
        (
            "/api/v1/maps/twin/world.svg?region=XA",
            400,
            "part of the path",
        ),
        ("/api/v1/maps/twin/world.png", 404, "unknown format"),
        ("/api/v1/maps/twin/XX.svg", 404, "no region"),
        ("/api/v1/maps/nope/world.svg", 404, "no dataset"),
        (
            "/api/v1/maps/twin/world.svg?worldview=XYZ",
            400,
            "unknown point of view",
        ),
        ("/api/v1/nothing", 404, "no such endpoint"),
    ] {
        let r = get(&app, uri).await;
        assert_eq!(r.status.as_u16(), status, "{uri}");
        assert_eq!(
            r.header("content-type"),
            "application/problem+json",
            "{uri}"
        );
        let p = r.json();
        assert_eq!(p["status"], status);
        assert!(
            p["detail"].as_str().unwrap().contains(needle),
            "{uri}: {}",
            p["detail"]
        );
    }
    let r = post(
        &app,
        "/api/v1/render",
        r#"{"dataset":"twin","region":"world","spec":{"colours":{}}}"#,
    )
    .await;
    assert!(r.json()["detail"]
        .as_str()
        .unwrap()
        .contains("unknown field `colours`"));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn match_and_reshape() {
    let (app, dir) = server("joins");
    let m = post(
        &app,
        "/api/v1/match",
        r#"{"dataset":"twin","region":"world","codes":["XA-01","XA-00","XA-09"]}"#,
    )
    .await
    .json();
    assert_eq!(m["matched"], 1);
    assert_eq!(m["dataNotOnMap"], serde_json::json!(["XA-00", "XA-09"]));
    let hints = m["hints"].as_array().unwrap();
    assert_eq!(
        (
            hints[0]["direction"].as_str(),
            hints[0]["codes"][0].as_str()
        ),
        (Some("old-data"), Some("XA-00"))
    );
    assert_eq!(
        (
            hints[1]["direction"].as_str(),
            hints[1]["codes"][0].as_str()
        ),
        (Some("new-data"), Some("XA-09"))
    );
    assert_eq!(
        post(&app, "/api/v1/match", r#"{"region":"world","codes":[]}"#)
            .await
            .status,
        StatusCode::BAD_REQUEST
    );

    let cws = get(&app, "/api/v1/crosswalks").await.json();
    assert_eq!(cws[0]["table"], "/api/v1/crosswalks/twin-old-new.csv");
    assert!(get(&app, "/api/v1/crosswalks/twin-old-new.csv")
        .await
        .text()
        .starts_with("from,to,weight\n"));

    let r = post(
        &app,
        "/api/v1/reshape",
        r#"{"table":"code,n\nXA-00,3\nXA-02,4\n","codeColumn":"code","crosswalk":"twin-old-new"}"#,
    )
    .await
    .json();
    assert_eq!(r["csv"], "code,n\nXA-01,3\nXA-09,4\n");
    let blocked = post(
        &app,
        "/api/v1/reshape",
        r#"{"table":"code,n\nA,1\n","codeColumn":"code","crosswalk":{"table":"from,to\nA,B\nA,C\n"}}"#,
    )
    .await;
    assert_eq!(blocked.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(blocked.json()["conflicts"][0]["from"], "A");
    let unknown = post(
        &app,
        "/api/v1/reshape",
        r#"{"table":"a\n","codeColumn":"a","crosswalk":"nope"}"#,
    )
    .await;
    assert_eq!(unknown.status, StatusCode::BAD_REQUEST);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn cross_origin_requests_are_allowed() {
    let (app, dir) = server("cors");
    let pre = send(
        &app,
        Request::options("/api/v1/match")
            .header(header::ORIGIN, "https://maphue.toolforge.org")
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
            .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "content-type")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(pre.header("access-control-allow-origin"), "*");
    assert!(pre.header("access-control-allow-methods").contains("POST"));
    let r = send(
        &app,
        Request::get("/api/v1/maps/twin/world.svg")
            .header(header::ORIGIN, "https://maphue.toolforge.org")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(r.header("access-control-allow-origin"), "*");
    assert!(r.header("access-control-expose-headers").contains("etag"));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn cache_headers() {
    let (app, dir) = server_with("cache", true);
    let cc = |r: &Reply| r.header("cache-control").to_owned();
    // Playground: hashed folders forever, the rest revalidated.
    assert_eq!(
        cc(&get(&app, "/pkg-0123abcd/mapgen_wasm.js").await),
        "public, max-age=31536000, immutable"
    );
    assert_eq!(
        cc(&get(&app, "/data-89abcdef/countries.geojson").await),
        "public, max-age=31536000, immutable"
    );
    assert_eq!(cc(&get(&app, "/pkg/unhashed.js").await), "no-cache");
    assert_eq!(cc(&get(&app, "/app.js").await), "no-cache");
    let index = get(&app, "/").await;
    assert_eq!(
        (index.status, cc(&index)),
        (StatusCode::OK, "no-cache".to_owned())
    );
    // API: maps for 30 days (a year when pinned), listings for a day,
    // health and version always fresh, errors not cached.
    assert_eq!(
        cc(&get(&app, "/api/v1/maps/twin/world.svg").await),
        "public, max-age=2592000"
    );
    assert_eq!(
        cc(&get(&app, "/api/v1/datasets").await),
        "public, max-age=86400"
    );
    assert_eq!(
        cc(&get(&app, "/api/v1/datasets/twin/regions/world/features").await),
        "public, max-age=86400"
    );
    assert_eq!(
        cc(&get(&app, "/api/v1/client.js").await),
        "public, max-age=86400"
    );
    assert_eq!(cc(&get(&app, "/api/v1/health").await), "no-cache");
    assert_eq!(cc(&get(&app, "/api/v1/version").await), "no-cache");
    assert_eq!(cc(&get(&app, "/api/v1/maps/twin/XX.svg").await), "");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn custom_maps_of_several_regions_and_recipes() {
    let (app, dir) = server("multi");
    // Codes in any order and case, or names: one canonical map.
    let a = get(&app, "/api/v1/maps/twin-files/XB,XA.svg?width=400").await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text());
    assert_eq!(
        a.header("content-location"),
        "/api/v1/maps/twin-files/XA,XB.svg?width=400"
    );
    let svg = a.text();
    assert!(svg.contains("id=\"XA-01\"") && svg.contains("id=\"XB-01\""));
    assert!(
        svg.contains("Other survey (CC0); Test survey (CC BY-SA 4.0) via tests"),
        "{svg}"
    );
    let b = get(&app, "/api/v1/maps/twin-files/xa,XB.svg?width=400").await;
    assert_eq!((b.header("x-cache"), b.body == a.body), ("hit", true));
    let m = get(&app, "/api/v1/maps/twin-files/XA,XB.json").await.json();
    assert_eq!(
        (m["region"].as_str(), m["shareAlike"].as_bool()),
        (Some("XA,XB"), Some(true))
    );
    assert_eq!(m["release"], "r1 + r2");
    assert_eq!(
        get(&app, "/api/v1/maps/twin-files/XA,ZZ.svg").await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&app, "/api/v1/maps/twin-files/XA.svg?regions=XB")
            .await
            .status,
        StatusCode::BAD_REQUEST
    );

    // The same map from a recipe.
    let recipe = "# two regions\nkey,value\ndataset,twin-files\nregion,XB\nregion,XA\nwidth,400\n";
    let r = send(
        &app,
        Request::post("/api/v1/render")
            .header(header::CONTENT_TYPE, "text/csv")
            .body(Body::from(recipe))
            .unwrap(),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.body, a.body);
    let bad = send(
        &app,
        Request::post("/api/v1/render")
            .header(header::CONTENT_TYPE, "text/csv")
            .body(Body::from(
                "key,value\ndataset,twin-files\nregion,XA\ncolour-water,red\n",
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    assert!(bad.json()["detail"]
        .as_str()
        .unwrap()
        .contains("colour-water"));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn regions_by_iso2_and_names_in_other_languages() {
    // Two countries in Natural Earth's Admin-0 layout, as the context layer
    // and as a dataset.
    let dir =
        std::env::temp_dir().join(format!("mapgen-server-test-aliases-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let country = |a3: &str, a2: &str, name: &str, fr: &str, x: f64| {
        format!(
            r#"{{"type":"Feature","properties":{{"ADM0_A3":"{a3}","ISO_A2_EH":"{a2}","NAME":"{name}","NAME_FR":"{fr}"}},"geometry":{{"type":"Polygon","coordinates":[[[{x},45],[{x1},45],[{x1},46],[{x},46],[{x},45]]]}}}}"#,
            x1 = x + 1.0
        )
    };
    std::fs::write(
        dir.join("countries.geojson"),
        format!(
            r#"{{"type":"FeatureCollection","features":[{},{}]}}"#,
            country("DEU", "DE", "Germany", "Allemagne", 10.0),
            country("NLD", "NL", "Netherlands", "Pays-Bas", 12.0)
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("datasets.toml"),
        "[context]\ncountries = \"countries.geojson\"\n\n[[dataset]]\nid = \"countries\"\ntitle = \"Countries\"\npreset = \"ne-admin0\"\nfile = \"countries.geojson\"\n",
    )
    .unwrap();
    let settings = Settings {
        data_dir: dir.clone(),
        cache_dir: None,
        cache_max_bytes: 0,
        max_concurrent: 2,
        render_timeout: Duration::from_secs(20),
        www_dir: None,
        max_width: 4000,
    };
    let app = app(Arc::new(AppState::new(settings).unwrap()));
    for path in [
        "DEU,NLD",
        "de,nl",
        "Allemagne,Pays-Bas",
        "germany,NETHERLANDS",
    ] {
        let r = get(&app, &format!("/api/v1/maps/countries/{path}.svg")).await;
        assert_eq!(r.status, StatusCode::OK, "{path}: {}", r.text());
        assert_eq!(
            r.header("content-location"),
            "/api/v1/maps/countries/DEU,NLD.svg",
            "{path}"
        );
    }
    assert_eq!(
        get(&app, "/api/v1/maps/countries/Atlantis.svg")
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    let _ = std::fs::remove_dir_all(dir);
}
