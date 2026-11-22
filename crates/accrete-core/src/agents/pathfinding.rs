//! A* over the tiles an agent *knows*, using the costs it *believes*.
//!
//! - Unknown tiles are never part of a route.
//! - Movement is 8-connected; a diagonal step is allowed only when both
//!   orthogonal neighbours are known and passable (no corner cutting), and
//!   costs `diagonal_num / 10` × the tile's MP.
//! - Believed cost = terrain cost (static, known once seen) with the path
//!   level the agent last saw on that tile.
//! - Each search expands at most `astar_max_expansions` nodes.
//! - Ties in the open list are broken by (f, h, tile index), so results are
//!   fully deterministic.
//!
//! The scratch buffers are reused between searches via generation stamps and
//! are not part of the simulation state.

use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

use crate::agents::memory::AgentMemory;
use crate::config::MovementConfig;
use crate::geom::{DIRS8, Pos, is_diagonal};
use crate::world::World;
use crate::world::movement::{cost_for, step_cost};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathError {
    /// The goal is not a known, passable tile.
    BadGoal,
    /// No known route exists.
    NoRoute,
    /// The expansion limit was hit before reaching the goal.
    Limit,
}

#[derive(Clone, Debug)]
pub struct Path {
    /// Tiles to enter, in order (excludes the start, includes the goal).
    pub steps: Vec<Pos>,
    /// Total believed MP cost.
    pub cost: u32,
    pub expansions: u32,
}

#[derive(Default)]
pub struct PathScratch {
    g: Vec<u32>,
    parent: Vec<u32>,
    stamp: Vec<u32>,
    closed: Vec<u32>,
    generation: u32,
    heap: BinaryHeap<Reverse<Node>>,
    pub total_expansions: u64,
    pub searches: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Node {
    f: u32,
    h: u32,
    idx: u32,
}

impl PartialOrd for Node {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Node {
    fn cmp(&self, o: &Self) -> Ordering {
        self.f
            .cmp(&o.f)
            .then(self.h.cmp(&o.h))
            .then(self.idx.cmp(&o.idx))
    }
}

/// Knowledge source for a search: what is known and what it costs to enter.
pub trait Belief {
    fn knows(&self, p: Pos) -> bool;
    /// Orthogonal MP cost to enter `p`, or `None` if believed impassable.
    fn cost(&self, world: &World, p: Pos, c: &MovementConfig) -> Option<u32>;
}

/// An agent's belief: its known-tile bitset and changed-tile memories.
pub struct AgentBelief<'a>(pub &'a AgentMemory);

impl Belief for AgentBelief<'_> {
    #[inline]
    fn knows(&self, p: Pos) -> bool {
        self.0.knows(p)
    }
    #[inline]
    fn cost(&self, world: &World, p: Pos, c: &MovementConfig) -> Option<u32> {
        let t = world.tile(p);
        crate::world::movement::with_disruption(
            cost_for(t.biome, t.is_river, self.0.believed_path(p), c),
            self.0.believed_disruption(p),
        )
    }
}

/// Perfect knowledge (omniscient baseline, tests, and evaluation only).
pub struct Omniscient;

impl Belief for Omniscient {
    #[inline]
    fn knows(&self, _p: Pos) -> bool {
        true
    }
    #[inline]
    fn cost(&self, world: &World, p: Pos, c: &MovementConfig) -> Option<u32> {
        world.enter_cost(p, c)
    }
}

impl PathScratch {
    fn reset(&mut self, n: usize) {
        if self.g.len() != n {
            self.g = vec![0; n];
            self.parent = vec![0; n];
            self.stamp = vec![0; n];
            self.closed = vec![0; n];
            self.generation = 0;
        }
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.stamp.iter_mut().for_each(|s| *s = 0);
            self.closed.iter_mut().for_each(|s| *s = 0);
            self.generation = 1;
        }
        self.heap.clear();
    }

    pub fn find_path(
        &mut self,
        world: &World,
        belief: &impl Belief,
        from: Pos,
        to: Pos,
        c: &MovementConfig,
        max_expansions: u32,
    ) -> Result<Path, PathError> {
        let dims = world.dims;
        if !dims.contains(to) || !belief.knows(to) || belief.cost(world, to, c).is_none() {
            return Err(PathError::BadGoal);
        }
        if from == to {
            return Ok(Path {
                steps: Vec::new(),
                cost: 0,
                expansions: 0,
            });
        }
        self.reset(dims.len());
        self.searches += 1;
        let gen_ = self.generation;
        // Admissible heuristic: octile distance at the cheapest possible tile cost.
        let hmin = c.min_mp.min(c.plains_mp);
        let heur = |p: Pos| (p.octile10(to) as u32) * hmin / 10;

        let start = dims.idx(from);
        let goal = dims.idx(to);
        self.g[start as usize] = 0;
        self.stamp[start as usize] = gen_;
        self.parent[start as usize] = start;
        self.heap.push(Reverse(Node {
            f: heur(from),
            h: heur(from),
            idx: start,
        }));
        let mut expansions = 0u32;
        while let Some(Reverse(node)) = self.heap.pop() {
            let i = node.idx as usize;
            if self.closed[i] == gen_ {
                continue;
            }
            self.closed[i] = gen_;
            if node.idx == goal {
                let mut steps = Vec::new();
                let mut cur = goal;
                while cur != start {
                    steps.push(dims.pos(cur));
                    cur = self.parent[cur as usize];
                }
                steps.reverse();
                self.total_expansions += u64::from(expansions);
                return Ok(Path {
                    steps,
                    cost: self.g[goal as usize],
                    expansions,
                });
            }
            expansions += 1;
            if expansions > max_expansions {
                self.total_expansions += u64::from(expansions);
                return Err(PathError::Limit);
            }
            let p = dims.pos(node.idx);
            let gp = self.g[i];
            for &(dx, dy) in &DIRS8 {
                let q = p.offset(dx, dy);
                if !dims.contains(q) || !belief.knows(q) {
                    continue;
                }
                let Some(tc) = belief.cost(world, q, c) else {
                    continue;
                };
                let diag = is_diagonal(dx, dy);
                if diag {
                    let a = p.offset(dx, 0);
                    let b = p.offset(0, dy);
                    let ok = |r: Pos| belief.knows(r) && belief.cost(world, r, c).is_some();
                    if !ok(a) || !ok(b) {
                        continue;
                    }
                }
                let j = dims.idx(q) as usize;
                if self.closed[j] == gen_ {
                    continue;
                }
                let ng = gp + step_cost(tc, diag, c);
                if self.stamp[j] != gen_ || ng < self.g[j] {
                    self.stamp[j] = gen_;
                    self.g[j] = ng;
                    self.parent[j] = node.idx;
                    let h = heur(q);
                    self.heap.push(Reverse(Node {
                        f: ng + h,
                        h,
                        idx: j as u32,
                    }));
                }
            }
        }
        self.total_expansions += u64::from(expansions);
        Err(PathError::NoRoute)
    }
}

/// Believed MP cost of a straight (Bresenham) line from `a` to `b`, used to
/// estimate travel time when scoring options without running A*. Unknown
/// tiles are assumed to cost `unknown_mp`; believed-impassable tiles add a
/// detour penalty of `blocked_mp`.
pub fn line_cost_estimate(
    world: &World,
    belief: &impl Belief,
    a: Pos,
    b: Pos,
    c: &MovementConfig,
) -> u32 {
    const UNKNOWN_MP: u32 = 15;
    const BLOCKED_MP: u32 = 40;
    let mut total = 0u32;
    let mut prev = a;
    crate::geom::bresenham(a, b, |p| {
        if p != a {
            let diag = p.x != prev.x && p.y != prev.y;
            let base = if !belief.knows(p) {
                UNKNOWN_MP
            } else {
                belief.cost(world, p, c).unwrap_or(BLOCKED_MP)
            };
            total += step_cost(base, diag, c);
            prev = p;
        }
        true
    });
    total
}
