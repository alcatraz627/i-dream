//! File-based storage layer.
//!
//! All subconscious state is stored as files (JSON, JSONL, TOML, Markdown).
//! This module provides atomic write operations, JSONL append, and directory
//! initialization.

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{de::DeserializeOwned, Serialize};
use std::io::{BufRead, Write};
use std::path::PathBuf;

/// Root data directory for all subconscious state.
pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn new(root: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&root)
            .with_context(|| format!("Failed to create store at {}", root.display()))?;
        Ok(Self { root })
    }

    /// Initialize the full directory structure on first run.
    pub fn init_dirs(&self) -> Result<()> {
        let dirs = [
            "dreams",
            "dreams/traces",
            "metacog",
            "metacog/samples",
            "metacog/audits",
            "valence",
            "introspection",
            "introspection/chains",
            "introspection/reports",
            "intentions",
            "logs",
        ];

        for dir in &dirs {
            std::fs::create_dir_all(self.root.join(dir))?;
        }

        Ok(())
    }

    /// Atomically write a JSON file (write to .tmp, then rename).
    pub fn write_json<T: Serialize>(&self, rel_path: &str, data: &T) -> Result<PathBuf> {
        let path = self.root.join(rel_path);
        let tmp_path = path.with_extension("tmp");

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let content = serde_json::to_string_pretty(data)?;
        std::fs::write(&tmp_path, &content)?;
        std::fs::rename(&tmp_path, &path)?;

        Ok(path)
    }

    /// Read a JSON file.
    pub fn read_json<T: DeserializeOwned>(&self, rel_path: &str) -> Result<T> {
        let path = self.root.join(rel_path);
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let data: T = serde_json::from_str(&content)?;
        Ok(data)
    }

    /// Append a line to a JSONL file.
    pub fn append_jsonl<T: Serialize>(&self, rel_path: &str, entry: &T) -> Result<()> {
        let path = self.root.join(rel_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;

        let line = serde_json::to_string(entry)?;
        writeln!(file, "{line}")?;

        Ok(())
    }

    /// Read all entries from a JSONL file.
    pub fn read_jsonl<T: DeserializeOwned>(&self, rel_path: &str) -> Result<Vec<T>> {
        let path = self.root.join(rel_path);
        if !path.exists() {
            return Ok(Vec::new());
        }

        let file = std::fs::File::open(&path)?;
        let reader = std::io::BufReader::new(file);
        let mut entries = Vec::new();

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let entry: T = serde_json::from_str(&line)
                .with_context(|| format!("Failed to parse JSONL line: {line}"))?;
            entries.push(entry);
        }

        Ok(entries)
    }

    /// Count entries in a JSONL file without loading them all.
    pub fn count_jsonl(&self, rel_path: &str) -> Result<usize> {
        let path = self.root.join(rel_path);
        if !path.exists() {
            return Ok(0);
        }
        let file = std::fs::File::open(&path)?;
        let reader = std::io::BufReader::new(file);
        Ok(reader.lines().filter_map(|l| l.ok()).filter(|l| !l.trim().is_empty()).count())
    }

    /// Write a markdown file.
    pub fn write_md(&self, rel_path: &str, content: &str) -> Result<PathBuf> {
        let path = self.root.join(rel_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, content)?;
        Ok(path)
    }

    /// Generate a timestamped filename.
    pub fn timestamped_name(prefix: &str, ext: &str) -> String {
        let now = Utc::now();
        format!("{}-{}.{}", now.format("%Y%m%d-%H%M"), prefix, ext)
    }

    /// Check if a file exists.
    pub fn exists(&self, rel_path: &str) -> bool {
        self.root.join(rel_path).exists()
    }

    /// Get absolute path for a relative path.
    pub fn path(&self, rel_path: &str) -> PathBuf {
        self.root.join(rel_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use tempfile::TempDir;

    // Helper struct for store tests — simple enough to verify by eye
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct TestEntry {
        id: u32,
        name: String,
    }

    fn test_store() -> (TempDir, Store) {
        let dir = TempDir::new().unwrap();
        let store = Store::new(dir.path().to_path_buf()).unwrap();
        (dir, store)
    }

    // ── Store::new ────────────────────────────────────────────
    // Verifies that creating a Store also creates the root directory.
    // If this fails, every subsequent file operation will fail.

    #[test]
    fn store_new_creates_root_dir() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("subconscious");
        assert!(!root.exists());

        let _store = Store::new(root.clone()).unwrap();
        assert!(root.exists());
    }

    // ── Store::init_dirs ──────────────────────────────────────
    // The daemon expects a specific directory tree. Missing dirs
    // cause runtime panics when modules try to write state files.

    #[test]
    fn store_init_dirs_creates_all_subdirs() {
        let (_dir, store) = test_store();
        store.init_dirs().unwrap();

        let expected = [
            "dreams", "dreams/traces", "metacog", "metacog/samples",
            "metacog/audits", "valence", "introspection",
            "introspection/chains", "introspection/reports",
            "intentions", "logs",
        ];
        for subdir in &expected {
            assert!(
                store.path(subdir).exists(),
                "Missing directory: {subdir}"
            );
        }
    }

    // ── JSON write + read ─────────────────────────────────────
    // Atomic JSON persistence: write_json uses a .tmp → rename
    // pattern to prevent partial writes on crash. These tests
    // verify both the happy path and atomicity guarantee.

    #[test]
    fn store_json_roundtrip() {
        let (_dir, store) = test_store();
        let entry = TestEntry { id: 1, name: "alpha".into() };

        store.write_json("test.json", &entry).unwrap();
        let loaded: TestEntry = store.read_json("test.json").unwrap();

        assert_eq!(loaded, entry);
    }

    #[test]
    fn store_json_write_no_leftover_tmp() {
        let (_dir, store) = test_store();
        let entry = TestEntry { id: 1, name: "alpha".into() };

        store.write_json("data.json", &entry).unwrap();

        // The .tmp file should be renamed away — not left behind
        assert!(!store.path("data.tmp").exists());
        assert!(store.path("data.json").exists());
    }

    #[test]
    fn store_json_creates_parent_dirs() {
        let (_dir, store) = test_store();
        let entry = TestEntry { id: 1, name: "nested".into() };

        store.write_json("deep/nested/data.json", &entry).unwrap();
        let loaded: TestEntry = store.read_json("deep/nested/data.json").unwrap();
        assert_eq!(loaded, entry);
    }

    #[test]
    fn store_json_read_missing_file_errors() {
        let (_dir, store) = test_store();
        let result: Result<TestEntry> = store.read_json("nonexistent.json");
        assert!(result.is_err());
    }

    // ── JSONL append + read ───────────────────────────────────
    // JSONL is the format for append-only logs (dream journal,
    // calibration entries, intention registry). Tests verify:
    // - ordering is preserved (critical for temporal data)
    // - empty/missing files return empty Vec (not errors)
    // - blank lines are skipped (robustness against partial writes)

    #[test]
    fn store_jsonl_append_and_read() {
        let (_dir, store) = test_store();

        let entries = vec![
            TestEntry { id: 1, name: "first".into() },
            TestEntry { id: 2, name: "second".into() },
            TestEntry { id: 3, name: "third".into() },
        ];
        for entry in &entries {
            store.append_jsonl("log.jsonl", entry).unwrap();
        }

        let loaded: Vec<TestEntry> = store.read_jsonl("log.jsonl").unwrap();
        assert_eq!(loaded, entries);
    }

    #[test]
    fn store_jsonl_read_missing_file_returns_empty() {
        let (_dir, store) = test_store();
        let loaded: Vec<TestEntry> = store.read_jsonl("nonexistent.jsonl").unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn store_jsonl_skips_blank_lines() {
        let (_dir, store) = test_store();

        // Write one entry, then inject a blank line manually
        let entry = TestEntry { id: 1, name: "only".into() };
        store.append_jsonl("log.jsonl", &entry).unwrap();

        // Append a blank line directly
        use std::io::Write;
        let path = store.path("log.jsonl");
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "").unwrap();
        writeln!(file, "  ").unwrap();

        let loaded: Vec<TestEntry> = store.read_jsonl("log.jsonl").unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0], entry);
    }

    // ── JSONL count ───────────────────────────────────────────
    // Used by should_run() checks to decide whether enough data
    // has accumulated for analysis. Must be consistent with read_jsonl.

    #[test]
    fn store_jsonl_count_matches_read_len() {
        let (_dir, store) = test_store();

        for i in 0..5 {
            let entry = TestEntry { id: i, name: format!("entry-{i}") };
            store.append_jsonl("log.jsonl", &entry).unwrap();
        }

        let count = store.count_jsonl("log.jsonl").unwrap();
        let entries: Vec<TestEntry> = store.read_jsonl("log.jsonl").unwrap();
        assert_eq!(count, entries.len());
        assert_eq!(count, 5);
    }

    #[test]
    fn store_jsonl_count_missing_file_returns_zero() {
        let (_dir, store) = test_store();
        assert_eq!(store.count_jsonl("nope.jsonl").unwrap(), 0);
    }

    // ── exists / path ─────────────────────────────────────────

    #[test]
    fn store_exists_true_for_written_file() {
        let (_dir, store) = test_store();
        assert!(!store.exists("data.json"));

        store.write_json("data.json", &TestEntry { id: 1, name: "x".into() }).unwrap();
        assert!(store.exists("data.json"));
    }

    #[test]
    fn store_path_joins_correctly() {
        let (dir, store) = test_store();
        let expected = dir.path().join("sub/file.json");
        assert_eq!(store.path("sub/file.json"), expected);
    }

    // ── Markdown write ────────────────────────────────────────

    #[test]
    fn store_write_md() {
        let (_dir, store) = test_store();
        store.write_md("notes/report.md", "# Hello\n\nWorld").unwrap();

        let content = std::fs::read_to_string(store.path("notes/report.md")).unwrap();
        assert_eq!(content, "# Hello\n\nWorld");
    }

    // ── timestamped_name ──────────────────────────────────────
    // Format: YYYYMMDD-HHMM-{prefix}.{ext}

    #[test]
    fn store_timestamped_name_format() {
        let name = Store::timestamped_name("dream", "jsonl");
        // Should match pattern like "20260411-1234-dream.jsonl"
        assert!(name.ends_with("-dream.jsonl"), "Got: {name}");
        assert_eq!(name.len(), "20260411-1234-dream.jsonl".len());
    }
}
