//! Measurements of emergent structure. Measurement only: never read by agents.

use crate::agents::agent::Agent;
use crate::config::TickRates;
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
    let vecs: Vec<[f32; 3]> = agents
        .iter()
        .filter(|a| !a.is_juvenile(rates))
        .map(|a| {
            [
                a.skills[0].practice,
                a.skills[1].practice,
                a.skills[2].practice,
            ]
        })
        .filter(|v| v.iter().sum::<f32>() >= 0.02)
        .collect();
    if vecs.len() < 2 {
        return 0.0;
    }
    let mut mean = [0.0f32; 3];
    for v in &vecs {
        for k in 0..3 {
            mean[k] += v[k] / vecs.len() as f32;
        }
    }
    let norm = |v: &[f32; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let nm = norm(&mean).max(1e-9);
    let mut sim = 0.0f64;
    for v in &vecs {
        let dot = v[0] * mean[0] + v[1] * mean[1] + v[2] * mean[2];
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
