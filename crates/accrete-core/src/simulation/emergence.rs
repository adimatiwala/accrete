//! Measurements of emergent structure. Measurement only: never read by agents.

use crate::agents::agent::Agent;
use crate::config::TickRates;
use crate::geom::Pos;
use crate::world::{Home, World};

/// Clark-Evans nearest-neighbour ratio `R = mean_nn / (0.5 / sqrt(density))`
/// over occupied homes, with density = homes per land tile. `R ≈ 1` for a
/// random (Poisson) pattern, `< 1` clustered, `> 1` dispersed. Homes sharing a
/// tile have distance 0. Returns 1.0 when there are fewer than two homes.
pub fn clark_evans(world: &World) -> f32 {
    let homes: Vec<&Home> = world.homes.iter().filter(|h| !h.is_vacant()).collect();
    let n = homes.len();
    if n < 2 {
        return 1.0;
    }
    let land = world.tiles.iter().filter(|t| t.is_land()).count().max(1) as f32;
    let mut sum = 0.0f64;
    for (i, a) in homes.iter().enumerate() {
        let mut best = i32::MAX;
        for (j, b) in homes.iter().enumerate() {
            if i != j {
                best = best.min(a.pos.dist2(b.pos));
            }
        }
        sum += f64::from((best as f32).sqrt());
    }
    let mean = (sum / n as f64) as f32;
    let expected = 0.5 / (n as f32 / land).sqrt();
    mean / expected
}

/// Standing food as a fraction of capacity on food tiles within 3 tiles of an
/// occupied home, divided by the same fraction on all other food tiles.
pub fn depletion_ratio(world: &World) -> f32 {
    let mut near = vec![false; world.dims.len()];
    for h in world.homes.iter().filter(|h| !h.is_vacant()) {
        for dy in -3..=3 {
            for dx in -3..=3 {
                let p = h.pos.offset(dx, dy);
                if world.contains(p) {
                    near[world.dims.idx(p) as usize] = true;
                }
            }
        }
    }
    let (mut nf, mut nc, mut ff, mut fc) = (0f64, 0f64, 0f64, 0f64);
    for (i, t) in world.tiles.iter().enumerate() {
        if t.food_capacity <= 0.0 {
            continue;
        }
        if near[i] {
            nf += f64::from(t.food);
            nc += f64::from(t.food_capacity);
        } else {
            ff += f64::from(t.food);
            fc += f64::from(t.food_capacity);
        }
    }
    if nc <= 0.0 || fc <= 0.0 || ff <= 0.0 {
        return 1.0;
    }
    ((nf / nc) / (ff / fc)) as f32
}

/// Division of labour among adults: `1 - mean cosine similarity` between
/// each adult's practice vector (food, wood, stone) and the population's mean
/// practice vector. 0 when everyone practises the same mix; higher when
/// individuals specialise in different resources. Adults with almost no
/// practice are skipped.
pub fn specialization(agents: &[Agent], rates: &TickRates) -> f32 {
    // Practice vectors over food, wood, stone and crafting (A.26).
    let vecs: Vec<[f32; 4]> = agents
        .iter()
        .filter(|a| !a.is_juvenile(rates))
        .map(|a| {
            [
                a.skills[0].practice,
                a.skills[1].practice,
                a.skills[2].practice,
                a.craft.practice,
            ]
        })
        .filter(|v| v.iter().sum::<f32>() >= 0.02)
        .collect();
    if vecs.len() < 2 {
        return 0.0;
    }
    let mut mean = [0.0f32; 4];
    for v in &vecs {
        for k in 0..4 {
            mean[k] += v[k] / vecs.len() as f32;
        }
    }
    let norm = |v: &[f32; 4]| (v.iter().map(|x| x * x).sum::<f32>()).sqrt();
    let nm = norm(&mean).max(1e-9);
    let mut sim = 0.0f64;
    for v in &vecs {
        let dot: f32 = v.iter().zip(&mean).map(|(a, b)| a * b).sum();
        sim += f64::from(dot / (norm(v).max(1e-9) * nm));
    }
    (1.0 - sim / vecs.len() as f64) as f32
}

pub fn mean_home_age_days(world: &World, tick: u64) -> f32 {
    let ages: Vec<f32> = world
        .homes
        .iter()
        .filter(|h| !h.is_vacant())
        .map(|h| (tick.saturating_sub(h.founded_tick)) as f32 / 24.0)
        .collect();
    if ages.is_empty() {
        0.0
    } else {
        ages.iter().sum::<f32>() / ages.len() as f32
    }
}

// ---------------------------------------------------------------------------
// Observatory (A.24): inequality, settlement sizes, trade network, culture.
// All measurement only; deterministic (fixed-order sums, `total_cmp` sorts).
// ---------------------------------------------------------------------------

/// Gini coefficient of non-negative values (0 = equal, → 1 = concentrated).
/// Returns 0 for fewer than two values or a zero total.
pub fn gini(values: &[f64]) -> f32 {
    let n = values.len();
    if n < 2 {
        return 0.0;
    }
    let mut v: Vec<f64> = values.iter().map(|x| x.max(0.0)).collect();
    v.sort_by(f64::total_cmp);
    let total: f64 = v.iter().sum();
    if total <= 0.0 {
        return 0.0;
    }
    let mut acc = 0.0f64;
    for (i, x) in v.iter().enumerate() {
        acc += (2.0 * (i as f64 + 1.0) - n as f64 - 1.0) * x;
    }
    (acc / (n as f64 * total)) as f32
}

/// Household wealth: goods held by residents plus the home's chests, in
/// weight units, for every occupied home (in home-id order).
pub fn household_wealth(
    world: &World,
    agents: &[Agent],
    inv: &crate::config::InventoryConfig,
) -> Vec<f64> {
    let mut out = Vec::new();
    for h in world.homes.iter().filter(|h| !h.is_vacant()) {
        let mut w = f64::from(h.stored().weight(inv));
        for &id in &h.residents {
            if let Ok(i) = agents.binary_search_by_key(&id, |a| a.id) {
                w += f64::from(agents[i].inventory.weight(inv));
            }
        }
        out.push(w);
    }
    out
}

/// Residents per cluster of occupied homes linked within `radius` tiles
/// (single linkage, as in settlement detection), largest first.
pub fn cluster_sizes(world: &World, radius: i32) -> Vec<(u32, u32)> {
    let homes: Vec<(crate::world::HomeId, crate::geom::Pos, u32)> = world
        .homes
        .iter()
        .filter(|h| !h.is_vacant())
        .map(|h| (h.id, h.pos, h.residents.len() as u32))
        .collect();
    let mut sizes: Vec<(u32, u32)> = crate::simulation::settlement::clusters(&homes, radius)
        .iter()
        .map(|g| (g.len() as u32, g.iter().map(|&i| homes[i].2).sum()))
        .collect();
    sizes.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.cmp(&a.0)));
    sizes
}

/// Global Moran's I of a per-agent trait over occupied homes. A home's value
/// is its residents' mean; weights are `1 / (1 + d)` for homes within
/// `range` tiles (Chebyshev), else 0. Returns `None` with fewer than three
/// homes, no neighbouring pairs, or no variance.
pub fn morans_i(
    world: &World,
    agents: &[Agent],
    range: i32,
    trait_of: impl Fn(&Agent) -> f32,
) -> Option<f32> {
    let mut pts: Vec<(crate::geom::Pos, f64)> = Vec::new();
    for h in world.homes.iter().filter(|h| !h.is_vacant()) {
        let (mut sum, mut n) = (0.0f64, 0u32);
        for &id in &h.residents {
            if let Ok(i) = agents.binary_search_by_key(&id, |a| a.id) {
                sum += f64::from(trait_of(&agents[i]));
                n += 1;
            }
        }
        if n > 0 {
            pts.push((h.pos, sum / f64::from(n)));
        }
    }
    let n = pts.len();
    if n < 3 {
        return None;
    }
    let mean = pts.iter().map(|p| p.1).sum::<f64>() / n as f64;
    let var: f64 = pts.iter().map(|p| (p.1 - mean) * (p.1 - mean)).sum();
    if var <= 1e-12 {
        return None;
    }
    let (mut num, mut wsum) = (0.0f64, 0.0f64);
    for i in 0..n {
        for j in 0..n {
            if i == j {
                continue;
            }
            let d = pts[i].0.chebyshev(pts[j].0);
            if d > range {
                continue;
            }
            let w = 1.0 / (1.0 + f64::from(d));
            wsum += w;
            num += w * (pts[i].1 - mean) * (pts[j].1 - mean);
        }
    }
    if wsum <= 0.0 {
        return None;
    }
    Some(((n as f64 / wsum) * (num / var)) as f32)
}

/// One completed barter, kept for measurement only.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TradeRecord {
    pub tick: u64,
    pub a: u32,
    pub b: u32,
    /// Proposer's tile at the time of exchange.
    pub pos: Pos,
    /// The two traders lived in different homes (or at least one was homeless).
    pub cross_household: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TradeNetwork {
    pub trades: u32,
    pub pairs: u32,
    pub traders: u32,
    pub mean_degree: f32,
    pub degree_gini: f32,
    pub cross_household_share: f32,
    pub repeat_share: f32,
    pub tiles: u32,
    pub tile_hhi: f32,
}

/// Network and location statistics over trades with `tick >= since`.
pub fn trade_network(log: &[TradeRecord], since: u64) -> TradeNetwork {
    use std::collections::BTreeMap;
    let mut pairs: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    let mut tiles: BTreeMap<Pos, u32> = BTreeMap::new();
    let mut cross = 0u32;
    let mut trades = 0u32;
    for r in log.iter().filter(|r| r.tick >= since) {
        let key = if r.a < r.b { (r.a, r.b) } else { (r.b, r.a) };
        *pairs.entry(key).or_insert(0) += 1;
        *tiles.entry(r.pos).or_insert(0) += 1;
        trades += 1;
        cross += u32::from(r.cross_household);
    }
    if trades == 0 {
        return TradeNetwork::default();
    }
    let mut degree: BTreeMap<u32, u32> = BTreeMap::new();
    for &(a, b) in pairs.keys() {
        *degree.entry(a).or_insert(0) += 1;
        *degree.entry(b).or_insert(0) += 1;
    }
    let degrees: Vec<f64> = degree.values().map(|&d| f64::from(d)).collect();
    let repeats: u32 = pairs.values().map(|&c| c - 1).sum();
    let tile_hhi: f64 = tiles
        .values()
        .map(|&count| {
            let share = f64::from(count) / f64::from(trades);
            share * share
        })
        .sum();
    TradeNetwork {
        trades,
        pairs: pairs.len() as u32,
        traders: degree.len() as u32,
        mean_degree: (degrees.iter().sum::<f64>() / degrees.len() as f64) as f32,
        degree_gini: gini(&degrees),
        cross_household_share: cross as f32 / trades as f32,
        repeat_share: repeats as f32 / trades as f32,
        tiles: tiles.len() as u32,
        tile_hhi: tile_hhi as f32,
    }
}

#[cfg(test)]
mod observatory_tests {
    use super::*;

    #[test]
    fn gini_known_values() {
        assert_eq!(gini(&[1.0, 1.0, 1.0, 1.0]), 0.0);
        // One holder of everything among n: (n - 1) / n.
        assert!((gini(&[0.0, 0.0, 0.0, 4.0]) - 0.75).abs() < 1e-6);
        assert!((gini(&[1.0, 2.0, 3.0, 4.0]) - 0.25).abs() < 1e-6);
        assert_eq!(gini(&[5.0]), 0.0);
    }

    #[test]
    fn trade_network_counts_pairs_degrees_and_repeats() {
        let r = |tick, a, b, x, cross| TradeRecord {
            tick,
            a,
            b,
            pos: Pos::new(x, 0),
            cross_household: cross,
        };
        let log = [
            r(1, 1, 2, 1, true),
            r(2, 2, 1, 1, true),
            r(3, 1, 3, 2, false),
            r(0, 4, 5, 3, true),
        ];
        let n = trade_network(&log, 1);
        assert_eq!((n.trades, n.pairs, n.traders), (3, 2, 3));
        assert!((n.mean_degree - 4.0 / 3.0).abs() < 1e-6);
        assert!((n.repeat_share - 1.0 / 3.0).abs() < 1e-6);
        assert!((n.cross_household_share - 2.0 / 3.0).abs() < 1e-6);
        assert_eq!(n.tiles, 2);
        assert!((n.tile_hhi - 5.0 / 9.0).abs() < 1e-6);
        assert_eq!(trade_network(&log, 100), TradeNetwork::default());
    }
}
