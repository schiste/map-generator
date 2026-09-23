//! The SVG contract (`docs/contract.md`): what tools that colour our maps
//! may rely on, checked on the golden fixture and every gallery map.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use mapgen_core::CONTRACT_VERSION;
use roxmltree::{Document, Node};

/// Layer groups of the main map, bottom to top.
const LAYERS: [&str; 9] = [
    "background",
    "water",
    "context",
    "context-borders",
    "land",
    "lakes",
    "disputed-areas",
    "borders",
    "labels",
];

fn maps() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = vec![root.join("tests/fixtures/twin-regions.svg")];
    let gallery = root.join("../../docs/examples");
    let mut svgs: Vec<PathBuf> = std::fs::read_dir(&gallery)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "svg"))
        .collect();
    svgs.sort();
    assert!(
        svgs.len() >= 5,
        "gallery not found in {}",
        gallery.display()
    );
    files.extend(svgs);
    files
}

fn has_class(n: Node, class: &str) -> bool {
    n.attribute("class")
        .is_some_and(|c| c.split_whitespace().any(|t| t == class))
}

fn in_layer(n: Node, layer: &str) -> bool {
    let class = format!("mg-{layer}");
    n.ancestors()
        .any(|a| a.attribute("id") == Some(layer) || has_class(a, &class))
}

fn check(path: &Path) {
    let name = path.file_name().unwrap().to_string_lossy();
    let text = std::fs::read_to_string(path).unwrap();
    let doc = Document::parse(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
    let svg = doc.root_element();

    // Root.
    assert_eq!(
        svg.attribute("data-mapgen-contract"),
        Some(CONTRACT_VERSION.to_string().as_str()),
        "{name}: contract version"
    );
    for a in ["width", "height", "viewBox"] {
        assert!(svg.attribute(a).is_some(), "{name}: <svg {a}>");
    }

    // Ids are unique.
    let mut ids = HashSet::new();
    for n in doc.descendants().filter(|n| n.is_element()) {
        if let Some(id) = n.attribute("id") {
            assert!(ids.insert(id), "{name}: duplicate id {id:?}");
        }
    }

    // Main-map layers appear in paint order.
    let order: Vec<usize> = svg
        .children()
        .filter_map(|n| n.attribute("id"))
        .filter_map(|id| LAYERS.iter().position(|l| *l == id))
        .collect();
    assert!(
        order.windows(2).all(|w| w[0] < w[1]),
        "{name}: layer order {order:?}"
    );
    assert!(order.contains(&4), "{name}: no #land layer");

    // Regions: one path each, with the contract's attributes.
    let regions: Vec<Node> = doc
        .descendants()
        .filter(|n| n.has_tag_name("path") && has_class(*n, "mg-land"))
        .collect();
    assert!(!regions.is_empty(), "{name}: no regions");
    for r in &regions {
        let id = r
            .attribute("id")
            .unwrap_or_else(|| panic!("{name}: region without id"));
        assert!(in_layer(*r, "land"), "{name}: {id} outside the land layer");
        for a in ["data-code", "data-name", "d"] {
            assert!(r.attribute(a).is_some(), "{name}: {id} has no {a}");
        }
        assert!(
            r.children().any(|c| c.has_tag_name("title")),
            "{name}: {id} has no <title>"
        );
        if r.attribute("data-parent-name").is_some() {
            assert!(
                r.attribute("data-parent").is_some(),
                "{name}: {id}: parent name without parent"
            );
        }
    }

    // Colours live in the stylesheet: no inline fills on regions or
    // neighbours, so a consumer's CSS or inline style always wins.
    for n in doc
        .descendants()
        .filter(|n| has_class(*n, "mg-land") || has_class(*n, "mg-context"))
    {
        assert!(n.attribute("fill").is_none(), "{name}: inline fill");
        assert!(n.attribute("style").is_none(), "{name}: inline style");
    }

    // Labels only in label layers.
    for n in doc.descendants().filter(|n| has_class(*n, "mg-label")) {
        assert!(in_layer(n, "labels"), "{name}: label outside #labels");
    }

    // With CSS variables, every colour of the stylesheet is overridable.
    let style = doc
        .descendants()
        .find(|n| n.has_tag_name("style"))
        .and_then(|n| n.text())
        .unwrap_or_default();
    if style.contains("var(--mg-") {
        for decl in style.split([';', '{', '}']).map(str::trim) {
            if let Some(v) = ["fill:", "stroke:"]
                .iter()
                .find_map(|p| decl.strip_prefix(p))
            {
                if v.starts_with('#') {
                    panic!("{name}: colour without a variable: {decl}");
                }
            }
        }
    }
}

#[test]
fn every_map_follows_the_contract() {
    for path in maps() {
        check(&path);
    }
}
