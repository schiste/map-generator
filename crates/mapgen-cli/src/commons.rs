//! What `mapgen batch` writes for uploading a set of maps to Wikimedia
//! Commons: file names from a template, a manifest with each file's SHA-1
//! and description fields, and optional `.wikitext` description pages. The
//! upload itself is left to existing tools (see `docs/commons.md`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use serde::Serialize;

/// One uploadable file. The first columns follow Pattypan's upload
/// spreadsheet (`path`, `name` without extension, then the
/// `{{Information}}` fields, `license`, `categories`).
#[derive(Debug, Clone, Default, Serialize)]
pub struct ManifestRow {
    pub path: String,
    pub name: String,
    pub description: String,
    pub date: String,
    pub source: String,
    pub author: String,
    pub permission: String,
    pub other_versions: String,
    pub license: String,
    /// Category names, without `[[Category:…]]`.
    pub categories: Vec<String>,
    /// Commons file name, with extension.
    pub file_name: String,
    /// SHA-1 of the file, as Commons stores it (hex).
    pub sha1: String,
    pub region: String,
    pub data_credit: String,
    pub share_alike: bool,
    pub boundary_year: String,
    pub source_release: String,
}

/// Replaces the `{key}` placeholders of `vars`, leaving other braces (such
/// as wikitext `{{templates}}`) as they are.
pub fn fill(template: &str, vars: &BTreeMap<&str, String>) -> String {
    vars.iter().fold(template.to_owned(), |t, (k, v)| {
        t.replace(&format!("{{{k}}}"), v)
    })
}

/// A file name from `template`: placeholders filled, made a valid Commons
/// title. A brace left over is a placeholder `vars` doesn't know (braces
/// aren't allowed in Commons titles anyway).
pub fn file_name(template: &str, vars: &BTreeMap<&str, String>) -> Result<String> {
    let name = fill(template, vars);
    if name.contains(['{', '}']) {
        bail!(
            "unknown placeholder in --name-template {template:?} (known: {})",
            vars.keys()
                .map(|k| format!("{{{k}}}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    Ok(commons_name(&name))
}

/// A valid Commons file name: characters MediaWiki forbids in titles
/// (`# < > [ ] | { } / :` and controls) become `-`, `_` becomes a space
/// (MediaWiki treats them alike) and runs of spaces collapse.
pub fn commons_name(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|c| match c {
            '#' | '<' | '>' | '[' | ']' | '|' | '{' | '}' | '/' | ':' | '\\' => '-',
            '_' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    replaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn sha1_hex(bytes: &[u8]) -> String {
    sha1_smol::Sha1::from(bytes).digest().to_string()
}

/// The description page: `{{Information}}`, licence and categories.
pub fn wikitext(row: &ManifestRow) -> String {
    let mut t = String::from("=={{int:filedesc}}==\n{{Information\n");
    for (k, v) in [
        ("description", &row.description),
        ("date", &row.date),
        ("source", &row.source),
        ("author", &row.author),
        ("permission", &row.permission),
        ("other versions", &row.other_versions),
    ] {
        t.push_str(&format!("|{k}={v}\n"));
    }
    t.push_str("}}\n\n=={{int:license-header}}==\n");
    t.push_str(&row.license);
    t.push('\n');
    if !row.categories.is_empty() {
        t.push('\n');
        for c in &row.categories {
            t.push_str(&format!("[[Category:{c}]]\n"));
        }
    }
    t
}

/// Writes the manifest as JSON (`.json`) or CSV (anything else; categories
/// joined with `;`), sorted by name.
pub fn write_manifest(path: &Path, rows: &mut [ManifestRow]) -> Result<()> {
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    let is_json = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("json"));
    let text = if is_json {
        serde_json::to_string_pretty(rows)? + "\n"
    } else {
        let header = [
            "path",
            "name",
            "description",
            "date",
            "source",
            "author",
            "permission",
            "other_versions",
            "license",
            "categories",
            "file_name",
            "sha1",
            "region",
            "data_credit",
            "share_alike",
            "boundary_year",
            "source_release",
        ];
        let mut out = header.join(",") + "\n";
        for r in rows.iter() {
            let cells = [
                &r.path,
                &r.name,
                &r.description,
                &r.date,
                &r.source,
                &r.author,
                &r.permission,
                &r.other_versions,
                &r.license,
                &r.categories.join(";"),
                &r.file_name,
                &r.sha1,
                &r.region,
                &r.data_credit,
                &r.share_alike.to_string(),
                &r.boundary_year,
                &r.source_release,
            ];
            let cells: Vec<String> = cells.iter().map(|c| csv(c)).collect();
            out.push_str(&cells.join(","));
            out.push('\n');
        }
        out
    };
    std::fs::write(path, text)?;
    Ok(())
}

/// Names given to more than one file, with the files.
pub fn duplicate_names(rows: &[ManifestRow]) -> Vec<(String, Vec<PathBuf>)> {
    let mut by_name: BTreeMap<&str, Vec<PathBuf>> = BTreeMap::new();
    for r in rows {
        by_name
            .entry(&r.file_name)
            .or_default()
            .push(PathBuf::from(&r.path));
    }
    by_name
        .into_iter()
        .filter(|(_, p)| p.len() > 1)
        .map(|(n, p)| (n.to_owned(), p))
        .collect()
}

fn csv(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_templates_and_rejects_unknown_keys() {
        let vars = BTreeMap::from([
            ("name", "France".to_string()),
            ("level", "ADM1".to_string()),
        ]);
        assert_eq!(
            file_name("Map of {name} ({level}).svg", &vars).unwrap(),
            "Map of France (ADM1).svg"
        );
        let e = file_name("Map of {nmae}.svg", &vars)
            .unwrap_err()
            .to_string();
        assert!(e.contains("{level}, {name}"), "{e}");
        // Wikitext templates are left alone.
        assert_eq!(
            fill("{{en|1=Map of {name}}} {{own}}", &vars),
            "{{en|1=Map of France}} {{own}}"
        );
    }

    #[test]
    fn commons_names_are_valid_titles() {
        assert_eq!(
            commons_name("Map of São Tomé: ADM1/2 [2022]"),
            "Map of São Tomé- ADM1-2 -2022-"
        );
        assert_eq!(commons_name("a_b  c\td"), "a b c d");
    }

    #[test]
    fn sha1_matches_commons_format() {
        // `printf abc | sha1sum`
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
    }

    #[test]
    fn wikitext_has_information_licence_and_categories() {
        let row = ManifestRow {
            description: "{{en|1=Map of France}}".into(),
            source: "{{own}}".into(),
            license: "{{self|cc-by-sa-4.0}}".into(),
            categories: vec!["Blank maps of France".into()],
            ..ManifestRow::default()
        };
        let t = wikitext(&row);
        assert!(t.starts_with(
            "=={{int:filedesc}}==\n{{Information\n|description={{en|1=Map of France}}\n"
        ));
        assert!(t.contains("=={{int:license-header}}==\n{{self|cc-by-sa-4.0}}\n"));
        assert!(t.ends_with("\n[[Category:Blank maps of France]]\n"));
    }
}
