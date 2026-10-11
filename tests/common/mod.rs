//! Shared helpers for the tests that read fetched fixtures.
//!
//! Files over [`LARGE_FIXTURE_BYTES`] are skipped unless
//! `STEPQ_LARGE_FIXTURES=1` is set: the Open Compute assemblies run to
//! 200 MB, and checking all of them in a debug build takes minutes and
//! gigabytes. The smaller Open Rack files still exercise the same Creo
//! exporter by default.
//!
//! A test that finds no fixture skips, so a fresh clone passes. With
//! `STEPQ_REQUIRE_FIXTURES=1`, as CI's geometry job sets after fetching the
//! core set, a missing core fixture fails instead ([`fixture`],
//! [`require_core_set`]); Open Compute files still skip when absent.

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

/// The fixtures `tools/fetch-fixtures.sh core` fetches: the STEP Tools AS1
/// assembly from five exporters, the NIST PMI files and the NIST EDM files.
pub const CORE_FILES: usize = 45;

/// True if `STEPQ_REQUIRE_FIXTURES` asks for missing core fixtures to fail.
pub fn require_fixtures() -> bool {
    env::var_os("STEPQ_REQUIRE_FIXTURES").is_some_and(|value| value != "0" && !value.is_empty())
}

/// True if `path`, under `tests/fixtures`, belongs to the core set.
pub fn is_core(path: &Path) -> bool {
    let relative = path.strip_prefix(fixtures_root()).unwrap_or(path);
    ["steptools", "nist", "nist-edm"]
        .iter()
        .any(|set| relative.starts_with(set))
}

/// Reads the fixture at `relative` (to `tests/fixtures`). `None`, with a
/// note, if it is missing or over the size limit, unless it is a core
/// fixture and [`require_fixtures`] holds: then the test fails.
pub fn fixture(relative: &str) -> Option<Vec<u8>> {
    let path = fixtures_root().join(relative);
    let required = require_fixtures() && is_core(&path);
    if !path.exists() {
        assert!(
            !required,
            "{} is missing and STEPQ_REQUIRE_FIXTURES is set (run tools/fetch-fixtures.sh core)",
            path.display()
        );
        eprintln!(
            "skipping: {} not fetched (run tools/fetch-fixtures.sh)",
            path.display()
        );
        return None;
    }
    if !is_selected(&path) {
        eprintln!(
            "skipping: {} is over {} MB (set STEPQ_LARGE_FIXTURES=1 to include it)",
            path.display(),
            LARGE_FIXTURE_BYTES / (1024 * 1024)
        );
        return None;
    }
    Some(fs::read(&path).expect("fixture is readable"))
}

/// Fails if [`require_fixtures`] holds and `files` (from [`step_files`])
/// lack any of the core set.
pub fn require_core_set(files: &[PathBuf]) {
    let core = files.iter().filter(|path| is_core(path)).count();
    assert!(
        !require_fixtures() || core == CORE_FILES,
        "{core} of the {CORE_FILES} core fixtures found and STEPQ_REQUIRE_FIXTURES is set \
         (run tools/fetch-fixtures.sh core)"
    );
}
