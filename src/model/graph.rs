//! Forward and backward reference indices over a parsed file.

use std::fmt;

use crate::error::{Error, Result};
use crate::p21::{Exchange, Instance};

/// The entity graph of a parsed file, with forward and back references.
///
/// Nodes are positions in [`Exchange::instances`]: node `n` is
/// `exchange.instances()[n]`. Each index lists a neighbour once, in
/// ascending node order, however many times the reference is repeated.
///
/// The back index is load-bearing, not an optimisation: a product's
/// shape, colours and placement all point *at* the product, so most
/// questions about a product are answered by
/// [`referenced_by`](Self::referenced_by). See `docs/ARCHITECTURE.md`.
pub struct Graph<'a> {
    exchange: Exchange<'a>,
    forward: Adjacency,
    backward: Adjacency,
    unresolved: Vec<(usize, u64)>,
}

/// Compressed adjacency lists: node `n`'s neighbours are
/// `targets[offsets[n]..offsets[n + 1]]`.
struct Adjacency {
    offsets: Vec<usize>,
    targets: Vec<usize>,
}

impl<'a> Graph<'a> {
    /// Builds both reference indices, failing on a dangling reference.
    ///
    /// Use this for anything that transforms or extracts: a dangling
    /// reference means the file is already missing data, and every graph
    /// operation on it would silently produce a wrong answer.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnresolvedReference`] for the first reference, in
    /// file order, to an instance that is not defined.
    pub fn new(exchange: Exchange<'a>) -> Result<Self> {
        let graph = Self::build(exchange);
        match graph.unresolved.first() {
            Some(&(node, to)) => Err(Error::UnresolvedReference {
                from: graph.instance(node).id,
                to,
            }),
            None => Ok(graph),
        }
    }

    /// Builds both reference indices, recording dangling references
    /// instead of failing.
    ///
    /// Use this where a partial answer is still useful, such as reporting
    /// every problem in a file. Dangling references are left out of both
    /// indices and listed by [`unresolved`](Self::unresolved).
    pub fn build(exchange: Exchange<'a>) -> Self {
        let instances = exchange.instances();
        let mut offsets = Vec::with_capacity(instances.len() + 1);
        offsets.push(0);
        let mut targets = Vec::new();
        let mut unresolved = Vec::new();
        let mut neighbours = Vec::new();
        for (node, instance) in instances.iter().enumerate() {
            neighbours.clear();
            for id in exchange.references(instance) {
                match exchange.position(id) {
                    Some(target) => neighbours.push(target),
                    None => unresolved.push((node, id)),
                }
            }
            neighbours.sort_unstable();
            neighbours.dedup();
            targets.extend_from_slice(&neighbours);
            offsets.push(targets.len());
        }
        let forward = Adjacency { offsets, targets };
        let backward = forward.reversed();
        Self {
            exchange,
            forward,
            backward,
            unresolved,
        }
    }

    /// The parsed file this graph indexes.
    pub fn exchange(&self) -> &Exchange<'a> {
        &self.exchange
    }

    /// The number of nodes, which is the number of instances.
    pub fn len(&self) -> usize {
        self.exchange.instances().len()
    }

    /// True if the file has no instances.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The node for instance `#id`.
    pub fn node(&self, id: u64) -> Option<usize> {
        self.exchange.position(id)
    }

    /// The instance at `node`.
    ///
    /// # Panics
    ///
    /// Panics if `node >= self.len()`.
    pub fn instance(&self, node: usize) -> &Instance {
        &self.exchange.instances()[node]
    }

    /// The nodes `node` refers to.
    ///
    /// # Panics
    ///
    /// Panics if `node >= self.len()`.
    pub fn references(&self, node: usize) -> &[usize] {
        self.forward.neighbours(node)
    }

    /// The nodes that refer to `node`.
    ///
    /// # Panics
    ///
    /// Panics if `node >= self.len()`.
    pub fn referenced_by(&self, node: usize) -> &[usize] {
        self.backward.neighbours(node)
    }

    /// References to undefined instances, as `(node, missing #id)` in file
    /// order. Always empty for a graph from [`new`](Self::new).
    pub fn unresolved(&self) -> &[(usize, u64)] {
        &self.unresolved
    }
}

impl fmt::Debug for Graph<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Graph")
            .field("nodes", &self.len())
            .field("edges", &self.forward.targets.len())
            .field("unresolved", &self.unresolved.len())
            .finish_non_exhaustive()
    }
}

impl Adjacency {
    fn neighbours(&self, node: usize) -> &[usize] {
        &self.targets[self.offsets[node]..self.offsets[node + 1]]
    }

    /// The same edges pointing the other way. Sources are visited in
    /// ascending order, so every reversed list comes out sorted.
    fn reversed(&self) -> Self {
        let nodes = self.offsets.len() - 1;
        let mut offsets = vec![0; nodes + 1];
        for &target in &self.targets {
            offsets[target + 1] += 1;
        }
        let mut total = 0;
        for offset in &mut offsets {
            total += *offset;
            *offset = total;
        }
        let mut next = offsets.clone();
        let mut targets = vec![0; self.targets.len()];
        for source in 0..nodes {
            for &target in self.neighbours(source) {
                targets[next[target]] = source;
                next[target] += 1;
            }
        }
        Self { offsets, targets }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::p21::parse;

    fn file_with(data: &str) -> String {
        format!("ISO-10303-21;HEADER;FILE_SCHEMA(('X'));ENDSEC;DATA;{data}ENDSEC;END-ISO-10303-21;")
    }

    #[test]
    fn forward_and_back_indices() {
        let src = file_with("#1=A(#2,#3,#2);#2=B(#3);#3=C('see #1');#4=D((#3,#1));");
        let graph = Graph::new(parse(src.as_bytes()).unwrap()).unwrap();
        assert_eq!(graph.len(), 4);
        assert_eq!(graph.references(0), [1, 2]);
        assert_eq!(graph.references(1), [2]);
        assert!(
            graph.references(2).is_empty(),
            "'#1' in a string is not a reference"
        );
        assert_eq!(graph.references(3), [0, 2]);
        assert_eq!(graph.referenced_by(0), [3]);
        assert_eq!(graph.referenced_by(2), [0, 1, 3]);
        assert!(graph.referenced_by(3).is_empty());
        assert!(graph.unresolved().is_empty());
    }

    #[test]
    fn nodes_follow_file_order_not_ids() {
        let src = file_with("#10=A(#5);#5=B();");
        let graph = Graph::new(parse(src.as_bytes()).unwrap()).unwrap();
        assert_eq!(graph.node(10), Some(0));
        assert_eq!(graph.node(5), Some(1));
        assert_eq!(graph.instance(1).id, 5);
        assert_eq!(graph.references(0), [1]);
        assert_eq!(graph.node(7), None);
    }

    #[test]
    fn dangling_references() {
        let src = file_with("#1=A(#2,#8);#2=B((#9));");

        let err = Graph::new(parse(src.as_bytes()).unwrap()).unwrap_err();
        assert!(matches!(err, Error::UnresolvedReference { from: 1, to: 8 }));

        let graph = Graph::build(parse(src.as_bytes()).unwrap());
        assert_eq!(graph.unresolved(), [(0, 8), (1, 9)]);
        assert_eq!(graph.references(0), [1]);
        assert!(graph.references(1).is_empty());
    }

    #[test]
    fn empty_data_section() {
        let src = file_with("");
        let graph = Graph::new(parse(src.as_bytes()).unwrap()).unwrap();
        assert!(graph.is_empty());
    }
}
