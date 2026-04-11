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
