//! The files opened lately, most recent first, kept in a small JSON file in
//! the user's configuration directory so the list survives a restart.

use std::path::{Path, PathBuf};

/// How many files the list remembers.
pub const LIMIT: usize = 10;

#[derive(Debug, Default)]
pub struct RecentFiles {
    /// Where the list is stored; `None` keeps it in memory only.
    store: Option<PathBuf>,
    entries: Vec<PathBuf>,
}

/// Two spellings of one file are one entry: Windows paths ignore case.
fn same_file(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

impl RecentFiles {
    /// A list read from `store`, or an empty one if it is missing or unreadable.
    pub fn load(store: Option<PathBuf>) -> Self {
        let entries = store
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str::<Vec<PathBuf>>(&text).ok())
            .unwrap_or_default()
            .into_iter()
            .take(LIMIT)
            .collect();
        Self { store, entries }
    }

    pub fn entries(&self) -> &[PathBuf] {
        &self.entries
    }

    /// Put a file at the top of the list.
    pub fn touch(&mut self, path: &Path) {
        let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        self.entries.retain(|entry| !same_file(entry, &path));
        self.entries.insert(0, path);
        self.entries.truncate(LIMIT);
        self.save();
    }

    /// Forget a file, e.g. one that no longer opens.
    pub fn remove(&mut self, path: &Path) {
        self.entries.retain(|entry| !same_file(entry, path));
        self.save();
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.save();
    }

    fn save(&self) {
        let Some(store) = &self.store else { return };
        if let Some(dir) = store.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(&self.entries) {
            let _ = std::fs::write(store, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("riffly-recent-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("recent.json")
    }

    #[test]
    fn newest_first_without_duplicates() {
        let mut recent = RecentFiles::load(None);
        recent.touch(Path::new("/a.md"));
        recent.touch(Path::new("/b.md"));
        recent.touch(Path::new("/a.md"));
        let names: Vec<_> = recent
            .entries()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["a.md", "b.md"]);
    }

    #[test]
    fn keeps_only_the_limit() {
        let mut recent = RecentFiles::load(None);
        for ix in 0..LIMIT + 5 {
            recent.touch(Path::new(&format!("/{ix}.md")));
        }
        assert_eq!(recent.entries().len(), LIMIT);
        assert!(recent.entries()[0].ends_with(format!("{}.md", LIMIT + 4)));
    }

    #[test]
    fn survives_a_reload() {
        let store = scratch("reload");
        let mut recent = RecentFiles::load(Some(store.clone()));
        recent.touch(Path::new("/kept.md"));
        let again = RecentFiles::load(Some(store.clone()));
        assert_eq!(again.entries().len(), 1);
        assert!(again.entries()[0].ends_with("kept.md"));
        let _ = std::fs::remove_dir_all(store.parent().unwrap());
    }
}
