//! Week 1 acceptance tests: deterministic, valid, varied worlds.

mod common;

use accrete_core::world::spawn::food_within;
use accrete_core::world::validate::validate_world;
use accrete_core::world::visibility::{SIGHT_RADIUS, brute_force_visible};
use accrete_core::{Config, Pos, generate_world};

#[test]
fn same_seed_same_world() {
    let c = Config::default();
    let a = generate_world(7, &c);
    let b = generate_world(7, &c);
    assert_eq!(a.state_hash(), b.state_hash());
    assert_eq!(a.vis.masks, b.vis.masks);
}

#[test]
fn different_seeds_differ() {
    let c = Config::default();
    let a = generate_world(1, &c);
    let b = generate_world(2, &c);
    assert_ne!(a.state_hash(), b.state_hash());
    let differing = a
        .tiles
        .iter()
        .zip(&b.tiles)
        .filter(|(x, y)| x.biome != y.biome)
        .count();
    assert!(
        differing > a.tiles.len() / 5,
        "worlds too similar: {differing} biome differences"
    );
}

#[test]
fn worlds_are_valid_across_seeds() {
    let c = Config::default();
    for seed in [1u64, 2, 3, 42, 1234, 0xdead_beef] {
        let w = generate_world(seed, &c);
        let errs = validate_world(&w, &c);
        assert!(errs.is_empty(), "seed {seed}: {errs:#?}");
    }
}

#[test]
fn rivers_exist_and_reach_outlets() {
    let c = Config::default();
    let w = generate_world(1, &c);
    let rivers = w.tiles.iter().filter(|t| t.is_river).count();
    assert!(rivers > 100, "only {rivers} river tiles");
    // Following any river downstream always ends at water or the map edge.
    for i in 0..w.tiles.len() {
        if !w.tiles[i].is_river {
            continue;
        }
        let mut cur = i as u32;
        for _ in 0..w.tiles.len() {
            let to = w.flow_to[cur as usize];
            if to == accrete_core::world::NO_TILE {
                assert!(w.dims.is_border(w.dims.pos(cur)), "river ends inland");
                break;
            }
            if !w.tiles[to as usize].is_land() {
                break;
            }
            cur = to;
        }
    }
}

#[test]
fn resources_follow_biomes() {
    use accrete_core::world::Biome;
    let c = Config::default();
    let w = generate_world(3, &c);
    for t in &w.tiles {
        if t.stone > 0.0 {
            assert!(matches!(t.biome, Biome::Hills | Biome::Mountains));
        }
        if t.food_capacity > 0.0 {
            assert!(t.fertility >= c.resources.min_food_fertility && !t.is_river);
        }
        if t.biome == Biome::Forest {
            assert_eq!(t.wood_capacity, c.resources.forest_wood);
        }
    }
    assert!(w.tiles.iter().any(|t| t.stone > 0.0), "no stone deposits");
}

#[test]
fn fertility_follows_river_distance() {
    use accrete_core::world::resources::river_bonus;
    let c = Config::default();
    let w = generate_world(1, &c);
    assert_eq!(river_bonus(1, &c.resources), 0.35);
    assert_eq!(river_bonus(4, &c.resources), 0.05);
    assert_eq!(river_bonus(5, &c.resources), 0.0);
    // Mean fertility falls with distance from rivers on plains.
    let mean = |d: u8| {
        let v: Vec<f32> = w
            .tiles
            .iter()
            .filter(|t| t.biome == accrete_core::world::Biome::Plains && t.river_distance == d)
            .map(|t| t.fertility)
            .collect();
        v.iter().sum::<f32>() / v.len().max(1) as f32
    };
    assert!(mean(1) > mean(3) && mean(3) > mean(8));
}

#[test]
fn visibility_masks_match_brute_force() {
    let c = Config::default();
    let w = generate_world(5, &c);
    let mut checked = 0;
    for i in (0..w.dims.len() as u32).step_by(97) {
        let from = w.dims.pos(i);
        for dy in -SIGHT_RADIUS..=SIGHT_RADIUS {
            for dx in -SIGHT_RADIUS..=SIGHT_RADIUS {
                let to = from.offset(dx, dy);
                let expected = brute_force_visible(&w, from, to, &c.sight);
                assert_eq!(
                    w.vis.is_visible(&w, from, to),
                    expected,
                    "{from:?} -> {to:?}"
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 20_000);
    // Adjacent tiles and the tile itself are always visible.
    let p = Pos::new(64, 64);
    for q in w.dims.neighbors8(p) {
        assert!(w.vis.is_visible(&w, p, q));
    }
    assert!(w.vis.is_visible(&w, p, p));
}

#[test]
fn spawns_are_viable_and_spread_out() {
    let c = Config::default();
    let w = generate_world(1, &c);
    assert_eq!(w.spawns.len(), c.spawn.agents as usize);
    for (k, &s) in w.spawns.iter().enumerate() {
        let t = w.tile(s);
        assert!(t.is_land() && !t.is_river);
        assert!(food_within(&w, s, c.spawn.food_radius as i32) >= c.spawn.min_nearby_food);
        for &o in &w.spawns[..k] {
            assert!(
                o.chebyshev(s) >= c.spawn.min_spacing as i32,
                "spawns too close"
            );
        }
    }
}

#[test]
fn golden_world_hash() {
    let c = Config::default();
    let w = generate_world(common::PRIMARY_SEED, &c);
    common::check_golden("world_seed1", w.state_hash());
}
