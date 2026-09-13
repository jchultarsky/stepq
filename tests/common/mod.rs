//! Shared helpers for the tests that read fetched fixtures.
//!
//! Files over [`LARGE_FIXTURE_BYTES`] are skipped unless
//! `STEPQ_LARGE_FIXTURES=1` is set: the Open Compute assemblies run to
//! 200 MB, and checking all of them in a debug build takes minutes and
//! gigabytes. The smaller Open Rack files still exercise the same Creo
//! exporter by default.

// Each test crate compiles this module and uses a different part of it.
#![allow(dead_code)]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Size above which a fixture is only tested on request.
pub const LARGE_FIXTURE_BYTES: u64 = 16 * 1024 * 1024;

pub fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// True if `STEPQ_LARGE_FIXTURES` asks for the large fixtures too.
pub fn include_large() -> bool {
    env::var_os("STEPQ_LARGE_FIXTURES").is_some_and(|value| value != "0" && !value.is_empty())
}

/// True if `path` is small enough to test, or large fixtures were asked for.
pub fn is_selected(path: &Path) -> bool {
    include_large() || fs::metadata(path).is_ok_and(|meta| meta.len() <= LARGE_FIXTURE_BYTES)
}

/// Every selected STEP file under `tests/fixtures`, sorted. Says how many
/// large files were skipped.
pub fn step_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect(&fixtures_root(), &mut files);
    files.sort();
    let total = files.len();
    files.retain(|path| is_selected(path));
    if files.len() < total {
        eprintln!(
            "skipping {} fixtures over {} MB (set STEPQ_LARGE_FIXTURES=1 to include them)",
            total - files.len(),
            LARGE_FIXTURE_BYTES / (1024 * 1024)
        );
    }
    files
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("stp") || ext.eq_ignore_ascii_case("step"))
        {
            out.push(path);
        }
    }
}
