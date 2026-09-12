//! End-to-end checks against the fetched STEP fixtures.
//!
//! Fixtures are downloaded by `tools/fetch-fixtures.sh` and are not in git,
//! so on a fresh clone these tests find nothing and pass.

use std::fs;
use std::path::{Path, PathBuf};

use stepq::model::Graph;
use stepq::p21::{Exchange, Lexer, Numbering, TokenKind, Writer, decode_string, parse};

#[test]
fn every_fixture_lexes() {
    for_each_fixture(|src| {
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
    });
}

#[test]
fn every_fixture_parses_into_a_graph() {
    for_each_fixture(|src| {
        let exchange = parse(src).map_err(|e| e.to_string())?;
        if !exchange.header().any(|r| r.is("FILE_SCHEMA")) {
            return Err("no FILE_SCHEMA in the header".to_owned());
        }
        if exchange.instances().is_empty() {
            return Err("no instances".to_owned());
        }
        Graph::new(exchange).map_err(|e| e.to_string())?;
        Ok(())
    });
}

#[test]
fn every_fixture_round_trips_through_the_writer() {
    for_each_fixture(|src| {
        let source = parse(src).map_err(|e| e.to_string())?;
        let count = source.instances().len();

        // Preserved names: every instance is written back byte for byte.
        let preserved = write(&source, Numbering::Preserve)?;
        let reparsed = parse(&preserved).map_err(|e| format!("preserved output: {e}"))?;
        if reparsed.instances().len() != count {
            return Err("preserved output has a different instance count".to_owned());
        }
        for (a, b) in source.instances().iter().zip(reparsed.instances()) {
            if source.text(a) != reparsed.text(b) {
                return Err(format!("#{} is not written verbatim", a.id));
            }
        }

        // Dense names: only instance names change, references follow them,
        // and writing the result again changes nothing.
        let dense = write(&source, Numbering::Dense)?;
        let renumbered = parse(&dense).map_err(|e| format!("dense output: {e}"))?;
        if renumbered.instances().len() != count {
            return Err("dense output has a different instance count".to_owned());
        }
        let new_name = |position: usize| u64::try_from(position + 1).unwrap();
        for (position, (a, b)) in source
            .instances()
            .iter()
            .zip(renumbered.instances())
            .enumerate()
        {
            if b.id != new_name(position) {
                return Err(format!("#{} became #{}", a.id, b.id));
            }
            let expected: Vec<u64> = source
                .references(a)
                .map(|id| source.position(id).map_or(0, new_name))
                .collect();
            if renumbered.references(b).collect::<Vec<_>>() != expected {
                return Err(format!(
                    "references of #{} were not renumbered consistently",
                    a.id
                ));
            }
            if non_name_tokens(source.text(a)) != non_name_tokens(renumbered.text(b)) {
                return Err(format!("#{} changed beyond its instance names", a.id));
            }
        }
        if write(&renumbered, Numbering::Preserve)? != dense {
            return Err("rewriting the dense output changed it".to_owned());
        }
        Ok(())
    });
}

#[test]
fn as1_assembly_links_resolve_both_ways() {
    let Some(src) = read_fixture("steptools/as1-ug-214.stp") else {
        return;
    };
    let graph = Graph::new(parse(&src).unwrap()).unwrap();
    let exchange = graph.exchange();
    let is = |node: usize, name: &str| exchange.records(graph.instance(node)).any(|r| r.is(name));

    let nauos: Vec<usize> = exchange
        .instances_of("NEXT_ASSEMBLY_USAGE_OCCURRENCE")
        .collect();
    assert_eq!(nauos.len(), 13);
    for &nauo in &nauos {
        let record = exchange.records(graph.instance(nauo)).next().unwrap();
        // Attributes 3 and 4: relating and related product definitions.
        for id in [3, 4].map(|index| record.param(index).and_then(|p| p.reference()).unwrap()) {
            let end = graph.node(id).unwrap();
            assert!(is(end, "PRODUCT_DEFINITION"), "#{id}");
            assert!(graph.referenced_by(end).contains(&nauo));
        }
    }
}

/// `docs/ARCHITECTURE.md`, "The back-reference problem": a forward closure
/// from a part's `product_definition` reaches six entities and no geometry;
/// the shape is only reachable backwards.
#[test]
fn forward_closure_from_a_part_reaches_no_geometry() {
    let Some(src) = read_fixture("steptools/as1-ug-214.stp") else {
        return;
    };
    let graph = Graph::new(parse(&src).unwrap()).unwrap();
    let exchange = graph.exchange();
    let is = |node: usize, name: &str| exchange.records(graph.instance(node)).any(|r| r.is(name));

    let part = (0..graph.len())
        .find(|&node| {
            is(node, "PRODUCT_DEFINITION")
                && graph.referenced_by(node).iter().all(|&user| {
                    !is(user, "NEXT_ASSEMBLY_USAGE_OCCURRENCE") || {
                        // Only ever the related (child) end, never the relating one.
                        let record = exchange.records(graph.instance(user)).next().unwrap();
                        let relating = record.param(3).and_then(|p| p.reference());
                        relating != Some(graph.instance(node).id)
                    }
                })
        })
        .expect("AS1 has leaf parts");

    let mut closure = vec![part];
    let mut seen = vec![false; graph.len()];
    seen[part] = true;
    let mut i = 0;
    while let Some(&node) = closure.get(i) {
        for &next in graph.references(node) {
            if !seen[next] {
                seen[next] = true;
                closure.push(next);
            }
        }
        i += 1;
    }
    let mut names: Vec<String> = closure
        .iter()
        .flat_map(|&node| exchange.records(graph.instance(node)))
        .map(|record| String::from_utf8_lossy(record.name()).to_ascii_uppercase())
        .collect();
    names.sort();

    assert_eq!(closure.len(), 6, "{names:?}");
    assert!(
        !names
            .iter()
            .any(|name| name.contains("SHAPE") || name.contains("REPRESENTATION")),
        "{names:?}"
    );
    assert!(
        graph
            .referenced_by(part)
            .iter()
            .any(|&user| is(user, "PRODUCT_DEFINITION_SHAPE")),
        "the shape points back at the part"
    );
}

fn write(exchange: &Exchange<'_>, numbering: Numbering) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    Writer::new(exchange)
        .numbering(numbering)
        .write_all(&mut out)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// The text of every token in `text` except instance names.
fn non_name_tokens(text: &[u8]) -> Vec<&[u8]> {
    Lexer::new(text)
        .map(|token| token.expect("instance text lexes"))
        .filter(|token| !matches!(token.kind, TokenKind::InstanceName(_)))
        .map(|token| token.span.slice(text))
        .collect()
}

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn read_fixture(relative: &str) -> Option<Vec<u8>> {
    let path = fixtures_root().join(relative);
    let src = fs::read(&path).ok();
    if src.is_none() {
        eprintln!(
            "skipping: {} not fetched (run tools/fetch-fixtures.sh)",
            path.display()
        );
    }
    src
}

/// Runs `check` on every fixture and fails listing every file that failed.
fn for_each_fixture(check: impl Fn(&[u8]) -> Result<(), String>) {
    let root = fixtures_root();
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
