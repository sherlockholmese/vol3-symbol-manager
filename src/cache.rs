use crate::sources::SymbolEntry;
use anyhow::Result;
use std::path::PathBuf;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct LastSearchResults {
    pub query: String,
    pub entries: Vec<SymbolEntry>,
}

pub fn write_last_search_results(query: &str, results: &[&SymbolEntry]) -> Result<()> {
    let cache = LastSearchResults {
        query: query.to_string(),
        entries: results.iter().map(|entry| (*entry).clone()).collect(),
    };
    let path = search_cache_path();
    let data = serde_json::to_vec_pretty(&cache)?;
    std::fs::write(&path, data)?;
    Ok(())
}

pub fn read_last_search_results() -> Result<LastSearchResults> {
    let path = search_cache_path();
    let data = std::fs::read(&path).map_err(|err| {
        anyhow::anyhow!(
            "Could not read last search results from {}: {}",
            path.display(),
            err
        )
    })?;
    let mut cache: LastSearchResults = serde_json::from_slice(&data)?;
    cache.entries.iter_mut().for_each(repair_legacy_url);
    Ok(cache)
}

/// Caches written before 1.1.0 prefixed Abyss release URLs with the raw
/// repository base, producing `.../raw/master/https://...`.
fn repair_legacy_url(entry: &mut SymbolEntry) {
    if let Some(pos) = entry.download_url.find("/https://") {
        entry.download_url = entry.download_url[pos + 1..].to_string();
    }
}

pub fn search_cache_path() -> PathBuf {
    if let Ok(path) = std::env::var("VOL3SM_SEARCH_CACHE") {
        return PathBuf::from(path);
    }

    std::env::temp_dir().join("vol3sm-last-search.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn entry(source_id: &str, banner: &str, path: &str) -> SymbolEntry {
        SymbolEntry {
            source_id: source_id.to_string(),
            source_name: source_id.to_string(),
            banner: banner.to_string(),
            path: path.to_string(),
            download_url: format!("https://example.invalid/{path}"),
        }
    }

    fn test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn search_results_are_cached_and_resolvable() {
        let _guard = test_lock().lock().unwrap();
        let cache_path =
            std::env::temp_dir().join(format!("vol3sm-test-cache-{}", std::process::id()));
        unsafe {
            std::env::set_var("VOL3SM_SEARCH_CACHE", &cache_path);
        }

        let entries = [
            entry(
                "abyss",
                "Linux version 6.1",
                "Debian/amd64/6.1.0/1/debian.json.xz",
            ),
            entry(
                "leludo",
                "Linux version 6.1",
                "profiles/debian/debian.json.xz",
            ),
        ];
        let entry_refs = entries.iter().collect::<Vec<_>>();

        write_last_search_results("debian 6.1", &entry_refs).unwrap();

        let cache = read_last_search_results().unwrap();
        assert_eq!(cache.query, "debian 6.1");
        assert_eq!(cache.entries.len(), 2);

        let _ = std::fs::remove_file(search_cache_path());
        unsafe {
            std::env::remove_var("VOL3SM_SEARCH_CACHE");
        }
    }
}
