//! Downhill river routing.
//!
//! **Local minima** are handled with priority-flood depression filling
//! (Barnes et al. 2014, with an epsilon gradient): starting from every outlet,
//! tiles are flooded in order of elevation, and each land tile's *filled*
//! elevation is raised to at least `epsilon` above the tile that reached it.
//! Every land tile then has a strictly lower neighbour, so water can always
//! drain and no pit traps a river.
//!
//! **Outlets** are ocean/lake tiles and land tiles on the **map boundary**; a
//! boundary tile with no lower neighbour drains off the map.
//!
//! Flow direction is D8: each land tile drains to its lowest-filled neighbour.
//! Accumulation sums rain (1 + moisture per tile) downstream, processed from
//! the highest filled elevation down. A tile is a river when its accumulation
//! reaches `river_threshold`. Because accumulation never decreases downstream,
//! every river continues until it reaches an outlet.

use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, VecDeque};

use crate::config::WorldConfig;
use crate::geom::Dims;
use crate::world::NO_TILE;
use crate::world::terrain::Terrain;

pub struct Drainage {
    pub filled: Vec<f32>,
    pub flow_to: Vec<u32>,
    pub flow: Vec<f32>,
    pub is_river: Vec<bool>,
    pub river_distance: Vec<u8>,
}

/// Heap key ordered by (filled elevation, tile index), both ascending.
#[derive(Clone, Copy, PartialEq)]
struct Key(f32, u32);
impl Eq for Key {}
impl PartialOrd for Key {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Key {
    fn cmp(&self, o: &Self) -> Ordering {
        self.0.total_cmp(&o.0).then(self.1.cmp(&o.1))
    }
}

pub fn route(dims: Dims, t: &Terrain, c: &WorldConfig) -> Drainage {
    let n = dims.len();
    let land = |i: usize| t.biome[i].is_land();

    // --- Priority flood ----------------------------------------------------
    let mut filled = t.elevation.clone();
    let mut done = vec![false; n];
    let mut heap = BinaryHeap::new();
    for i in 0..n {
        if !land(i) || dims.is_border(dims.pos(i as u32)) {
            done[i] = true;
            heap.push(Reverse(Key(filled[i], i as u32)));
        }
    }
    while let Some(Reverse(Key(fe, i))) = heap.pop() {
        for q in dims.neighbors8(dims.pos(i)) {
            let j = dims.idx(q) as usize;
            if done[j] {
                continue;
            }
            done[j] = true;
            filled[j] = filled[j].max(fe + c.river_epsilon);
            heap.push(Reverse(Key(filled[j], j as u32)));
        }
    }

    // --- D8 flow directions ------------------------------------------------
    let mut flow_to = vec![NO_TILE; n];
    for i in 0..n {
        if !land(i) {
            continue;
        }
        let p = dims.pos(i as u32);
        let mut best = NO_TILE;
        let mut best_e = filled[i];
        for q in dims.neighbors8(p) {
            let j = dims.idx(q);
            let e = filled[j as usize];
            if e < best_e {
                best_e = e;
                best = j;
            }
        }
        // Only boundary tiles may lack a lower neighbour (they drain off-map).
        debug_assert!(best != NO_TILE || dims.is_border(p));
        flow_to[i] = best;
    }

    // --- Accumulation, highest first ---------------------------------------
    let mut order: Vec<u32> = (0..n as u32).filter(|&i| land(i as usize)).collect();
    order.sort_by(|&a, &b| {
        filled[b as usize]
            .total_cmp(&filled[a as usize])
            .then(a.cmp(&b))
    });
    let mut flow: Vec<f32> = (0..n)
        .map(|i| if land(i) { 1.0 + t.moisture[i] } else { 0.0 })
        .collect();
    for &i in &order {
        let to = flow_to[i as usize];
        if to != NO_TILE && land(to as usize) {
            flow[to as usize] += flow[i as usize];
        }
    }
    let is_river: Vec<bool> = (0..n)
        .map(|i| land(i) && flow[i] >= c.river_threshold)
        .collect();

    // --- Distance to river (8-connected BFS, capped) -----------------------
    let mut river_distance = vec![u8::MAX; n];
    let mut queue = VecDeque::new();
    for (i, &r) in is_river.iter().enumerate() {
        if r {
            river_distance[i] = 0;
            queue.push_back(i as u32);
        }
    }
    while let Some(i) = queue.pop_front() {
        let d = river_distance[i as usize];
        if d == u8::MAX - 1 {
            continue;
        }
        for q in dims.neighbors8(dims.pos(i)) {
            let j = dims.idx(q) as usize;
            if river_distance[j] == u8::MAX {
                river_distance[j] = d + 1;
                queue.push_back(j as u32);
            }
        }
    }

    Drainage {
        filled,
        flow_to,
        flow,
        is_river,
        river_distance,
    }
}
