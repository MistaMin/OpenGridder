//! Child-process scanner with deadman blacklist tracking.
//! Fulfills H9: Scan crash deadman blacklists the plugin that killed the scanner.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct DeadmanScanner {
    deadman_file: PathBuf,
    blacklist_file: PathBuf,
}

impl DeadmanScanner {
    pub fn new(deadman_file: impl Into<PathBuf>, blacklist_file: impl Into<PathBuf>) -> Self {
        Self {
            deadman_file: deadman_file.into(),
            blacklist_file: blacklist_file.into(),
        }
    }

    pub fn deadman_path(&self) -> &Path {
        &self.deadman_file
    }

    pub fn blacklist_path(&self) -> &Path {
        &self.blacklist_file
    }

    pub fn read_blacklist(&self) -> HashSet<String> {
        let mut set = HashSet::new();
        if let Ok(file) = File::open(&self.blacklist_file) {
            let reader = BufReader::new(file);
            for line in reader.lines().flatten() {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    set.insert(trimmed.to_string());
                }
            }
        }
        set
    }

    pub fn add_to_blacklist(&self, plugin_id: &str) -> std::io::Result<()> {
        let mut list = self.read_blacklist();
        if list.insert(plugin_id.to_string()) {
            if let Some(parent) = self.blacklist_file.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let mut file = File::create(&self.blacklist_file)?;
            for item in &list {
                writeln!(file, "{item}")?;
            }
        }
        Ok(())
    }

    /// Run before scanning a specific plugin: record to deadman file.
    pub fn record_start(&self, plugin_id: &str) -> std::io::Result<()> {
        if let Some(parent) = self.deadman_file.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let mut file = File::create(&self.deadman_file)?;
        writeln!(file, "{plugin_id}")?;
        file.sync_all()
    }

    /// Run after successfully scanning a plugin: clear the deadman file.
    pub fn record_success(&self) -> std::io::Result<()> {
        if self.deadman_file.exists() {
            fs::remove_file(&self.deadman_file)?;
        }
        Ok(())
    }

    /// Process scan results / startup check:
    /// If deadman file still exists (meaning a crash occurred while scanning),
    /// read the culprit plugin ID, add it to blacklist, and delete deadman file.
    pub fn process_crash_deadman(&self) -> Vec<String> {
        let mut blacklisted = Vec::new();
        if self.deadman_file.exists() {
            if let Ok(file) = File::open(&self.deadman_file) {
                let reader = BufReader::new(file);
                for line in reader.lines().flatten() {
                    let trimmed = line.trim().to_string();
                    if !trimmed.is_empty() {
                        let _ = self.add_to_blacklist(&trimmed);
                        blacklisted.push(trimmed);
                    }
                }
            }
            let _ = fs::remove_file(&self.deadman_file);
        }
        blacklisted
    }

    /// Execute safe scan item using closure. If closure panics or fails,
    /// the deadman file will remain until processed.
    pub fn scan_item<F, T>(&self, plugin_id: &str, scan_fn: F) -> Result<T, String>
    where
        F: FnOnce() -> Result<T, String>,
    {
        if self.read_blacklist().contains(plugin_id) {
            return Err(format!("plugin '{plugin_id}' is blacklisted"));
        }
        self.record_start(plugin_id).map_err(|e| e.to_string())?;
        let res = scan_fn();
        if res.is_ok() {
            let _ = self.record_success();
        }
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn h9_scan_crash_deadman_blacklists_culprit_plugin() {
        let dir = std::env::temp_dir().join(format!("ag_deadman_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let deadman = dir.join("deadman.txt");
        let blacklist = dir.join("blacklist.txt");

        let scanner = DeadmanScanner::new(&deadman, &blacklist);

        // 1. Successful scan
        let res = scanner.scan_item("good-plugin-1", || Ok("success"));
        assert_eq!(res.unwrap(), "success");
        assert!(!deadman.exists(), "deadman file must be cleared after success");
        assert!(!scanner.read_blacklist().contains("good-plugin-1"));

        // 2. Simulated crash during scanning (e.g. process died while scanning bad-plugin-crash)
        scanner.record_start("bad-plugin-crash").unwrap();
        assert!(deadman.exists(), "deadman file must exist during scan");

        // 3. Next startup / scanner recovery processes deadman crash file
        let recovered = scanner.process_crash_deadman();
        assert_eq!(recovered, vec!["bad-plugin-crash".to_string()]);
        assert!(!deadman.exists(), "deadman file must be removed after crash processing");
        assert!(scanner.read_blacklist().contains("bad-plugin-crash"), "culprit must be added to blacklist");

        // 4. Subsequent scan must reject the blacklisted plugin
        let res_blacklisted = scanner.scan_item("bad-plugin-crash", || Ok("should not run"));
        assert!(res_blacklisted.is_err());
        assert!(res_blacklisted.unwrap_err().contains("blacklisted"));

        let _ = fs::remove_dir_all(&dir);
    }
}
