//! The seeds of the `read` fuzz target parse and reach every reader.
//!
//! Starting from an empty corpus, libFuzzer does not write a file that parses
//! into even one instance within CI's 60 seconds, so without seeds the `read`
//! target would pass without running a reader at all. CI therefore starts it
//! from `fuzz/seeds/read`. This test keeps those seeds parsing as the parser
//! changes, and checks that between them they reach every reader with both
//! an answer and a refusal, the geometry readers in the contexts the target
//! builds them in. Each reader added to the target adds seeds and a line
//! here.
//!
//! `fuzz/` is not packaged, so in a downloaded crate this test skips.

use std::fs;
use std::path::{Path, PathBuf};

use stepq::geometry::Reader;
use stepq::model::Graph;
use stepq::p21::parse;
use stepq::units::{ContextUnits, representation_context, unit};

fn seeds() -> Option<Vec<PathBuf>> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/seeds/read");
    let entries = fs::read_dir(&dir).ok()?;
    let mut seeds: Vec<PathBuf> = entries
        .map(|entry| entry.expect("seed directory is readable").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "stp"))
        .collect();
    seeds.sort();
    assert!(!seeds.is_empty(), "no seeds in {}", dir.display());
    Some(seeds)
}

/// How often a reader answered and refused, over every instance of every
/// seed.
#[derive(Debug, Default)]
struct Reached {
    ok: usize,
    err: usize,
}

impl Reached {
    fn count<T, E>(&mut self, result: &Result<T, E>) {
        if result.is_ok() {
            self.ok += 1;
        } else {
            self.err += 1;
        }
    }
}

#[test]
fn read_seeds_reach_every_reader() {
    let Some(seeds) = seeds() else {
        eprintln!("skipping: fuzz/seeds/read is not here (a packaged crate)");
        return;
    };
    let mut readers: [(&str, Reached); 14] = [
        ("unit", Reached::default()),
        ("representation_context", Reached::default()),
        ("ContextUnits::of_context", Reached::default()),
        ("ContextUnits::of_representation", Reached::default()),
        ("ContextUnits::of_item", Reached::default()),
        ("Reader::for_representation", Reached::default()),
        ("Reader::for_item", Reached::default()),
        ("Reader::point", Reached::default()),
        ("Reader::direction", Reached::default()),
        ("Reader::vector", Reached::default()),
        ("Reader::axis1_placement", Reached::default()),
        ("Reader::axis2_placement_3d", Reached::default()),
        ("Reader::surface", Reached::default()),
        ("Reader::curve", Reached::default()),
    ];
    for path in &seeds {
        let src = fs::read(path).expect("seed is readable");
        let exchange = parse(&src).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(
            !exchange.instances().is_empty(),
            "{} has no instances",
            path.display()
        );
        // As the fuzz target builds it: dangling references are allowed.
        let graph = Graph::build(exchange);
        for instance in graph.exchange().instances() {
            let id = instance.id;
            readers[0].1.count(&unit(&graph, id));
            readers[1].1.count(&representation_context(&graph, id));
            readers[2].1.count(&ContextUnits::of_context(&graph, id));
            readers[3]
                .1
                .count(&ContextUnits::of_representation(&graph, id));
            readers[4].1.count(&ContextUnits::of_item(&graph, id));
            readers[5]
                .1
                .count(&Reader::for_representation(&graph, id).map(|_| ()));
            readers[6]
                .1
                .count(&Reader::for_item(&graph, id).map(|_| ()));
        }
        // As the target does: the first readers the file yields, over every
        // instance.
        let geometry: Vec<Reader<'_, '_>> = graph
            .exchange()
            .instances()
            .iter()
            .flat_map(|instance| {
                [
                    Reader::for_representation(&graph, instance.id),
                    Reader::for_item(&graph, instance.id),
                ]
            })
            .filter_map(Result::ok)
            .take(4)
            .collect();
        for reader in &geometry {
            for instance in graph.exchange().instances() {
                let id = instance.id;
                readers[7].1.count(&reader.point(id));
                readers[8].1.count(&reader.direction(id));
                readers[9].1.count(&reader.vector(id));
                readers[10].1.count(&reader.axis1_placement(id));
                readers[11].1.count(&reader.axis2_placement_3d(id));
                readers[12].1.count(&reader.surface(id));
                readers[13].1.count(&reader.curve(id));
            }
        }
    }
    for (reader, reached) in &readers {
        assert!(
            reached.ok > 0 && reached.err > 0,
            "{reader}: the seeds should reach both an answer and a refusal: {reached:?}"
        );
    }
}
