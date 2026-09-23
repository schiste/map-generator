//! A bounded on-disk cache of rendered maps. Output is deterministic, so an
//! entry is valid for as long as the mapgen version and the dataset release
//! it was made with, and the build commit, which are all part of the key.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

pub struct Cache {
    dir: Option<PathBuf>,
    max_bytes: u64,
    used: AtomicU64,
    evicting: Mutex<()>,
}

impl Cache {
    /// `dir: None` disables caching.
    pub fn new(dir: Option<PathBuf>, max_bytes: u64) -> std::io::Result<Cache> {
        let mut used = 0;
        if let Some(d) = &dir {
            std::fs::create_dir_all(d)?;
            used = entries(d).iter().map(|(_, len, _)| len).sum();
        }
        Ok(Cache {
            dir,
            max_bytes,
            used: AtomicU64::new(used),
            evicting: Mutex::new(()),
        })
    }

    pub fn key(parts: &[&str]) -> String {
        sha1_smol::Sha1::from(parts.join("\u{1f}"))
            .digest()
            .to_string()
    }

    fn path(&self, key: &str) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(&key[..2]).join(key))
    }

    pub fn get(&self, key: &str) -> Option<Vec<u8>> {
        std::fs::read(self.path(key)?).ok()
    }

    pub fn put(&self, key: &str, body: &[u8]) {
        let Some(path) = self.path(key) else { return };
        // Write then rename, so a reader never sees half a file.
        let tmp = path.with_extension("tmp");
        let ok = path
            .parent()
            .is_some_and(|p| std::fs::create_dir_all(p).is_ok())
            && std::fs::write(&tmp, body).is_ok()
            && std::fs::rename(&tmp, &path).is_ok();
        if ok
            && self.used.fetch_add(body.len() as u64, Ordering::Relaxed) + body.len() as u64
                > self.max_bytes
        {
            self.evict();
        }
    }

    /// Deletes the least recently modified entries down to 80 % of the limit.
    fn evict(&self) {
        let (Some(dir), Ok(_guard)) = (&self.dir, self.evicting.try_lock()) else {
            return;
        };
        let mut files = entries(dir);
        files.sort_by_key(|(_, _, modified)| *modified);
        let mut used: u64 = files.iter().map(|(_, len, _)| len).sum();
        let target = self.max_bytes / 10 * 8;
        for (path, len, _) in files {
            if used <= target {
                break;
            }
            if std::fs::remove_file(&path).is_ok() {
                used -= len;
            }
        }
        self.used.store(used, Ordering::Relaxed);
    }
}

fn entries(dir: &Path) -> Vec<(PathBuf, u64, std::time::SystemTime)> {
    let mut out = Vec::new();
    for sub in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        for f in std::fs::read_dir(sub.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            if let Ok(m) = f.metadata() {
                if m.is_file() {
                    out.push((
                        f.path(),
                        m.len(),
                        m.modified().unwrap_or(std::time::UNIX_EPOCH),
                    ));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_and_evicts() {
        let dir = std::env::temp_dir().join(format!("mapgen-cache-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cache = Cache::new(Some(dir.clone()), 100).unwrap();
        let (a, b) = (Cache::key(&["a"]), Cache::key(&["b"]));
        cache.put(&a, &[1; 60]);
        assert_eq!(cache.get(&a).unwrap().len(), 60);
        std::thread::sleep(std::time::Duration::from_millis(20));
        cache.put(&b, &[2; 60]); // over 100: the older entry goes
        assert!(cache.get(&a).is_none());
        assert!(cache.get(&b).is_some());
        let off = Cache::new(None, 0).unwrap();
        off.put(&a, b"x");
        assert!(off.get(&a).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
