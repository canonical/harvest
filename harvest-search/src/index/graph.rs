use std::collections::{HashMap, HashSet, VecDeque};

use crate::domain::UnitId;
use crate::index::corpus::{Edge, Relation};

#[derive(Clone, Debug, Default)]
struct Csr {
    offsets: Vec<u32>,
    targets: Vec<(UnitId, Relation)>,
}

impl Csr {
    fn build(nodes: usize, mut pairs: Vec<(UnitId, UnitId, Relation)>) -> Self {
        pairs.sort_unstable();
        pairs.dedup();
        let mut offsets = vec![0u32; nodes + 1];
        for (from, _, _) in &pairs {
            offsets[*from as usize + 1] += 1;
        }
        for i in 0..nodes {
            offsets[i + 1] += offsets[i];
        }
        let targets = pairs.into_iter().map(|(_, to, r)| (to, r)).collect();
        Self { offsets, targets }
    }

    fn of(&self, node: UnitId) -> &[(UnitId, Relation)] {
        let i = node as usize;
        if i + 1 >= self.offsets.len() {
            return &[];
        }
        &self.targets[self.offsets[i] as usize..self.offsets[i + 1] as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    Out,
    In,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Depth {
    Direct,
    Transitive,
}

#[derive(Clone, Debug, Default)]
pub struct EdgeWeights {
    weights: HashMap<(Relation, Direction), f32>,
}

impl EdgeWeights {
    pub fn new(entries: &[(Relation, Direction, f32)]) -> Self {
        Self {
            weights: entries.iter().map(|(r, d, w)| ((*r, *d), *w)).collect(),
        }
    }

    pub fn weight(&self, relation: Relation, direction: Direction) -> f32 {
        self.weights
            .get(&(relation, direction))
            .copied()
            .unwrap_or(0.0)
    }
}

#[derive(Clone, Debug, Default)]
pub struct CodeGraph {
    nodes: usize,
    out: Csr,
    inn: Csr,
}

impl CodeGraph {
    pub fn build(nodes: usize, edges: &[Edge]) -> Self {
        let forward = edges.iter().map(|e| (e.src, e.dst, e.relation)).collect();
        let backward = edges.iter().map(|e| (e.dst, e.src, e.relation)).collect();
        Self {
            nodes,
            out: Csr::build(nodes, forward),
            inn: Csr::build(nodes, backward),
        }
    }

    pub fn node_count(&self) -> usize {
        self.nodes
    }

    pub fn edge_count(&self) -> usize {
        self.out.targets.len()
    }

    pub fn neighbors(
        &self,
        node: UnitId,
        direction: Direction,
        relations: &[Relation],
    ) -> impl Iterator<Item = UnitId> + '_ {
        let csr = match direction {
            Direction::Out => &self.out,
            Direction::In => &self.inn,
        };
        let wanted: Vec<Relation> = relations.to_vec();
        csr.of(node)
            .iter()
            .filter(move |(_, r)| wanted.is_empty() || wanted.contains(r))
            .map(|(n, _)| *n)
    }

    pub fn edges_of(&self, node: UnitId, direction: Direction) -> &[(UnitId, Relation)] {
        match direction {
            Direction::Out => self.out.of(node),
            Direction::In => self.inn.of(node),
        }
    }

    pub fn walk(
        &self,
        start: UnitId,
        direction: Direction,
        relations: &[Relation],
        depth: Depth,
    ) -> Vec<UnitId> {
        let mut seen = HashSet::from([start]);
        let mut order = Vec::new();
        let mut queue = VecDeque::from([start]);
        while let Some(node) = queue.pop_front() {
            for next in self.neighbors(node, direction, relations) {
                if seen.insert(next) {
                    order.push(next);
                    if depth == Depth::Transitive {
                        queue.push_back(next);
                    }
                }
            }
        }
        order
    }

    pub fn subclasses(&self, class: UnitId, depth: Depth) -> Vec<UnitId> {
        self.walk(
            class,
            Direction::In,
            &[Relation::Inherits, Relation::Implements],
            depth,
        )
    }

    pub fn ancestors(&self, class: UnitId) -> Vec<UnitId> {
        self.walk(
            class,
            Direction::Out,
            &[Relation::Inherits, Relation::Implements],
            Depth::Transitive,
        )
    }

    pub fn personalized_pagerank(
        &self,
        seeds: &[(UnitId, f32)],
        weights: &EdgeWeights,
        restart: f32,
        iterations: usize,
    ) -> Vec<(UnitId, f32)> {
        let total: f32 = seeds.iter().map(|(_, s)| s.max(0.0)).sum();
        if total <= 0.0 || self.nodes == 0 {
            return Vec::new();
        }
        let table: Vec<(f32, f32)> = Relation::ALL
            .iter()
            .map(|r| {
                (
                    weights.weight(*r, Direction::Out),
                    weights.weight(*r, Direction::In),
                )
            })
            .collect();
        let weight_of = |relation: Relation, direction: Direction| {
            let (out, inn) = table[relation as usize];
            match direction {
                Direction::Out => out,
                Direction::In => inn,
            }
        };
        let mut teleport = vec![0f32; self.nodes];
        let mut rank = vec![0f32; self.nodes];
        let mut active: Vec<UnitId> = Vec::new();
        for (node, score) in seeds {
            if (*node as usize) < self.nodes && *score > 0.0 {
                if teleport[*node as usize] == 0.0 {
                    active.push(*node);
                }
                teleport[*node as usize] += score / total;
            }
        }
        rank.clone_from(&teleport);
        let mut next = vec![0f32; self.nodes];
        let mut touched = vec![false; self.nodes];
        for _ in 0..iterations {
            let mut reached: Vec<UnitId> = Vec::with_capacity(active.len() * 2);
            for &node in &active {
                let i = node as usize;
                let mass = rank[i];
                if mass < PRUNE {
                    continue;
                }
                let out = self.out.of(node);
                let inn = self.inn.of(node);
                let sum: f32 = out
                    .iter()
                    .map(|(_, r)| weight_of(*r, Direction::Out))
                    .chain(inn.iter().map(|(_, r)| weight_of(*r, Direction::In)))
                    .sum();
                let spread = |target: UnitId,
                              share: f32,
                              next: &mut [f32],
                              touched: &mut [bool],
                              reached: &mut Vec<UnitId>| {
                    let t = target as usize;
                    next[t] += share;
                    if !touched[t] {
                        touched[t] = true;
                        reached.push(target);
                    }
                };
                if sum <= 0.0 {
                    spread(
                        node,
                        (1.0 - restart) * mass,
                        &mut next,
                        &mut touched,
                        &mut reached,
                    );
                    continue;
                }
                let scale = (1.0 - restart) * mass / sum;
                for (target, relation) in out {
                    let w = weight_of(*relation, Direction::Out);
                    if w > 0.0 {
                        spread(*target, scale * w, &mut next, &mut touched, &mut reached);
                    }
                }
                for (target, relation) in inn {
                    let w = weight_of(*relation, Direction::In);
                    if w > 0.0 {
                        spread(*target, scale * w, &mut next, &mut touched, &mut reached);
                    }
                }
            }
            for &node in &active {
                rank[node as usize] = 0.0;
            }
            for (node, t) in teleport.iter().enumerate() {
                if *t > 0.0 {
                    next[node] += restart * t;
                    if !touched[node] {
                        touched[node] = true;
                        reached.push(node as UnitId);
                    }
                }
            }
            for &node in &reached {
                let i = node as usize;
                rank[i] = next[i];
                next[i] = 0.0;
                touched[i] = false;
            }
            active = reached;
        }
        let mut ranked: Vec<(UnitId, f32)> = active
            .into_iter()
            .map(|n| (n, rank[n as usize]))
            .filter(|(_, s)| *s > 0.0)
            .collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        ranked
    }
}

const PRUNE: f32 = 1e-6;

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(src: UnitId, dst: UnitId, relation: Relation) -> Edge {
        Edge { src, dst, relation }
    }

    fn hierarchy() -> CodeGraph {
        CodeGraph::build(
            6,
            &[
                edge(1, 0, Relation::Inherits),
                edge(2, 0, Relation::Inherits),
                edge(3, 1, Relation::Inherits),
                edge(4, 3, Relation::Calls),
                edge(5, 4, Relation::Calls),
            ],
        )
    }

    #[test]
    fn subclasses_walk_direct_or_transitively() {
        let graph = hierarchy();
        assert_eq!(graph.subclasses(0, Depth::Direct), vec![1, 2]);
        assert_eq!(graph.subclasses(0, Depth::Transitive), vec![1, 2, 3]);
        assert_eq!(graph.ancestors(3), vec![1, 0]);
    }

    #[test]
    fn neighbors_filter_by_relation_and_direction() {
        let graph = hierarchy();
        assert_eq!(
            graph
                .neighbors(4, Direction::Out, &[Relation::Calls])
                .collect::<Vec<_>>(),
            vec![3]
        );
        assert_eq!(
            graph.neighbors(4, Direction::In, &[]).collect::<Vec<_>>(),
            vec![5]
        );
        assert_eq!(graph.neighbors(99, Direction::In, &[]).count(), 0);
        assert_eq!(graph.edge_count(), 5);
    }

    #[test]
    fn pagerank_follows_only_weighted_edges() {
        let graph = hierarchy();
        let calls_only = EdgeWeights::new(&[(Relation::Calls, Direction::Out, 1.0)]);
        let ranked = graph.personalized_pagerank(&[(5, 1.0)], &calls_only, 0.3, 10);
        let ids: Vec<UnitId> = ranked.iter().map(|(n, _)| *n).collect();
        assert!(ids.contains(&4) && ids.contains(&3));
        assert!(!ids.contains(&0));
        let hierarchy_weights = EdgeWeights::new(&[(Relation::Inherits, Direction::In, 1.0)]);
        let ranked = graph.personalized_pagerank(&[(0, 1.0)], &hierarchy_weights, 0.3, 10);
        let ids: Vec<UnitId> = ranked.iter().map(|(n, _)| *n).collect();
        assert!(ids.contains(&1) && ids.contains(&2) && ids.contains(&3));
        assert!(!ids.contains(&4));
    }

    #[test]
    fn pagerank_without_seeds_is_empty() {
        let ranked = hierarchy().personalized_pagerank(&[], &EdgeWeights::default(), 0.3, 5);
        assert!(ranked.is_empty());
    }
}
