//! Lexes every fetched STEP fixture end to end, decoding every string.
//!
//! Fixtures are downloaded by `tools/fetch-fixtures.sh` and are not in git,
//! so on a fresh clone this test finds nothing and passes.

use std::fs;
use std::path::{Path, PathBuf};

use stepq::p21::{Lexer, TokenKind, decode_string};

#[test]
fn every_fixture_lexes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut files = Vec::new();
    collect_step_files(&root, &mut files);
    if files.is_empty() {
        eprintln!(
            "skipping: no STEP files under {} (run tools/fetch-fixtures.sh)",
            root.display()
        );
        return;
    }
    files.sort();

    let failures: Vec<String> = files
        .iter()
        .filter_map(|path| {
            let src = fs::read(path).expect("fixture is readable");
            check(&src)
                .err()
                .map(|err| format!("{}: {err}", path.display()))
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{} of {} fixtures failed:\n{}",
        failures.len(),
        files.len(),
        failures.join("\n")
    );
}

fn check(src: &[u8]) -> Result<(), String> {
    let mut first = None;
    for token in Lexer::new(src) {
        let token = token.map_err(|e| e.to_string())?;
        first.get_or_insert(token);
        if token.kind == TokenKind::String {
            decode_string(src, token.span).map_err(|e| e.to_string())?;
        }
    }
    match first {
        Some(token) if token.span.slice(src) == b"ISO-10303-21" => Ok(()),
        _ => Err("does not start with ISO-10303-21".to_owned()),
    }
}

fn collect_step_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_step_files(&path, out);
        } else if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("stp") || ext.eq_ignore_ascii_case("step"))
        {
            out.push(path);
        }
    }
}
