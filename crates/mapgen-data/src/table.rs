//! Delimited text tables (CSV, TSV or pipe-separated), with a header row.
//! Used for code and name lookups (`mapgen convert --codes-from`,
//! `--parent-names`) and for crosswalks between data units and regions.

use std::collections::BTreeMap;
use std::path::Path;

use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub header: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

pub fn read_table(path: &Path) -> Result<Table> {
    parse_table(&std::fs::read_to_string(path)?)
        .map_err(|e| Error::Table(format!("{}: {e}", path.display())))
}

/// Parses a table. The delimiter is whichever of `,`, `\t`, `|` or `;` is
/// most frequent in the header line; fields may be double-quoted (with `""`
/// for a literal quote), as in CSV.
pub fn parse_table(text: &str) -> std::result::Result<Table, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let first = text.lines().next().ok_or("empty table")?;
    let delim = [',', '\t', '|', ';']
        .into_iter()
        .max_by_key(|d| first.matches(*d).count())
        .unwrap_or(',');
    let mut records = split_records(text, delim)?;
    if records.is_empty() {
        return Err("empty table".into());
    }
    let header: Vec<String> = records
        .remove(0)
        .into_iter()
        .map(|h| h.trim().to_owned())
        .collect();
    let rows = records
        .into_iter()
        .filter(|r| !(r.len() == 1 && r[0].trim().is_empty()))
        .collect();
    Ok(Table { header, rows })
}

fn split_records(text: &str, delim: char) -> std::result::Result<Vec<Vec<String>>, String> {
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            (true, '"') => quoted = false,
            (true, c) => field.push(c),
            (false, '"') if field.is_empty() => quoted = true,
            (false, c) if c == delim => record.push(std::mem::take(&mut field)),
            (false, '\r') => {}
            (false, '\n') => {
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
            }
            (false, c) => field.push(c),
        }
    }
    if quoted {
        return Err("unterminated quoted field".into());
    }
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    Ok(records)
}

impl Table {
    /// Index of a column, matched case-insensitively.
    pub fn column(&self, name: &str) -> Result<usize> {
        self.header
            .iter()
            .position(|h| h.eq_ignore_ascii_case(name))
            .ok_or_else(|| {
                Error::Table(format!(
                    "no column {name:?} (columns: {})",
                    self.header.join(", ")
                ))
            })
    }

    /// Cell of a row, trimmed; blank cells are `None`.
    pub fn cell(&self, row: &[String], col: usize) -> Option<String> {
        row.get(col)
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    }

    /// `key → value` for two columns. Keys that appear with different values
    /// are ambiguous: they are left out and returned separately.
    pub fn lookup(
        &self,
        key: &str,
        value: &str,
    ) -> Result<(BTreeMap<String, String>, Vec<String>)> {
        let (k, v) = (self.column(key)?, self.column(value)?);
        let mut map: BTreeMap<String, String> = BTreeMap::new();
        let mut ambiguous: BTreeMap<String, ()> = BTreeMap::new();
        for row in &self.rows {
            let (Some(key), Some(value)) = (self.cell(row, k), self.cell(row, v)) else {
                continue;
            };
            match map.get(&key) {
                Some(existing) if *existing != value => {
                    ambiguous.insert(key, ());
                }
                _ => {
                    map.insert(key, value);
                }
            }
        }
        for key in ambiguous.keys() {
            map.remove(key);
        }
        Ok((map, ambiguous.into_keys().collect()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_pipe_separated_census_files() {
        let t = parse_table("STATE|STUSAB|STATE_NAME\n01|AL|Alabama\n31|NE|Nebraska\n").unwrap();
        assert_eq!(t.header, ["STATE", "STUSAB", "STATE_NAME"]);
        let (m, amb) = t.lookup("state", "state_name").unwrap();
        assert_eq!(m["31"], "Nebraska");
        assert!(amb.is_empty());
    }

    #[test]
    fn reads_quoted_csv() {
        let t = parse_table(
            "\u{feff}code,name\r\n25007,\"Dukes and Nantucket, MA\"\r\n9,\"Say \"\"hi\"\"\"\r\n",
        )
        .unwrap();
        assert_eq!(t.rows[0], ["25007", "Dukes and Nantucket, MA"]);
        assert_eq!(t.rows[1][1], "Say \"hi\"");
    }

    #[test]
    fn ambiguous_keys_are_reported_not_guessed() {
        let t = parse_table("name,fips\nLancaster,31109\nLancaster,42071\nAda,16001\nAda,16001\n")
            .unwrap();
        let (m, amb) = t.lookup("name", "fips").unwrap();
        assert_eq!(amb, ["Lancaster"]);
        assert_eq!(m.len(), 1);
        assert_eq!(m["Ada"], "16001");
        assert!(t.column("missing").is_err());
    }
}
