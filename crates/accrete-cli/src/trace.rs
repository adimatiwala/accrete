//! `accrete trace`: tick-by-tick explanation of individual agents' decisions.

use accrete_core::config::TICKS_PER_DAY;
use accrete_core::{Config, Simulation};

pub fn trace(seed: u64, config: &Config, day: u32, ticks: u32, agents: &[u32], options: bool) {
    let mut sim = Simulation::new(seed, config.clone());
    sim.run(u64::from(day) * TICKS_PER_DAY);
    let ids: Vec<u32> = if agents.is_empty() {
        // Agents flagged by a pathology detector in the last day, else the first three.
        let since = sim.tick.saturating_sub(2 * TICKS_PER_DAY);
        let mut flagged: Vec<u32> = sim
            .pathology_log
            .iter()
            .filter(|(t, _, _)| *t >= since)
            .map(|(_, id, _)| *id)
            .filter(|id| sim.agent(*id).is_some())
            .collect();
        let recent: Vec<_> = sim.pathology_log.iter().rev().take(5).collect();
        println!("recent pathologies: {recent:?}");
        flagged.sort_unstable();
        flagged.dedup();
        flagged.truncate(3);
        if flagged.is_empty() {
            sim.agents.iter().take(3).map(|a| a.id).collect()
        } else {
            flagged
        }
    } else {
        agents.to_vec()
    };
    for &id in &ids {
        if let Some(a) = sim.agent(id) {
            println!(
                "agent {id}: age {:.0} d, traits cur {:.2} cau {:.2} soc {:.2} dil {:.2}, home {:?}, food {:.1}, known {} tiles, {} food memories",
                a.age_days(),
                a.traits.curiosity,
                a.traits.caution,
                a.traits.sociability,
                a.traits.diligence,
                a.home.map(|h| sim.world.homes[h as usize].pos),
                a.inventory.food,
                a.memory.known_count(),
                a.memory
                    .resource_count(accrete_core::inventory::Resource::Food)
            );
        }
    }
    for _ in 0..ticks {
        sim.step();
        let tick = sim.tick - 1;
        for &id in &ids {
            let Some(idx) = sim.agents.iter().position(|a| a.id == id) else {
                println!("t{tick} a{id}: dead");
                continue;
            };
            let a = &sim.agents[idx];
            let intent = sim.intent_of(id);
            let tr = sim.traces.get(&id);
            let chosen = tr.and_then(|t| t.chosen.as_ref());
            println!(
                "t{tick} a{id} @({},{}) H {:.2} E {:.2} hp {:.2} food {:.1} wood {:.1} | goal {} -> ({},{}) | intent {:?} | score {:.2} {}",
                a.pos.x,
                a.pos.y,
                a.hunger,
                a.energy,
                a.health,
                a.inventory.food,
                a.inventory.wood,
                a.goal.kind.name(),
                a.goal.target.x,
                a.goal.target.y,
                intent,
                chosen.map_or(0.0, |c| c.score),
                chosen.map_or(String::new(), |c| c
                    .terms
                    .iter()
                    .map(|t| format!("{}={:+.2}", t.name, t.value))
                    .collect::<Vec<_>>()
                    .join(" "))
            );
            if options {
                if let Some(t) = tr {
                    for o in t.options.iter().take(6) {
                        println!(
                            "      {:>14} ({:>3},{:>3}) {:+.3} eta {:>5.1} [{}]",
                            o.kind.name(),
                            o.target.x,
                            o.target.y,
                            o.score,
                            o.est_ticks,
                            o.terms
                                .iter()
                                .map(|t| format!("{}={:+.2}", t.name, t.value))
                                .collect::<Vec<_>>()
                                .join(" ")
                        );
                    }
                }
            }
        }
    }
}

/// Distribution of the perceptions that drive settling and reproduction.
pub fn diag(seed: u64, config: &Config, day: u32) {
    use accrete_core::agents::decision::{
        Ctx, known_people_within, land_food_ratio, land_potential,
    };
    let mut sim = Simulation::new(seed, config.clone());
    sim.run(u64::from(day) * TICKS_PER_DAY);
    let cx = Ctx {
        world: &sim.world,
        config: &sim.config,
        rates: &sim.rates,
        tick: sim.tick,
        population_at_cap: false,
        agents: &sim.agents,
    };
    let mut rows: Vec<(f32, f32, f32, usize, usize)> = Vec::new();
    for a in &sim.agents {
        if let Some(h) = cx.home(a) {
            let r = cx.config.homes.site_radius as i32;
            let actual: usize = sim
                .world
                .homes
                .iter()
                .filter(|o| o.pos.chebyshev(h.pos) <= r)
                .map(|o| o.residents.len())
                .sum();
            rows.push((
                land_food_ratio(a, &cx, h.pos, 1.0),
                land_potential(a, &cx, h.pos),
                known_people_within(a, &cx, h.pos, r),
                actual,
                a.memory.homes.len(),
            ));
        }
    }
    home_structure(&sim);
    {
        use accrete_core::agents::decision::{material_wants, wants_shelter};
        let (mut ws, mut wood_ok, mut stone_ok, mut want_stone, mut stone_mem, mut goals) =
            (0, 0, 0, 0, 0, [0u32; 14]);
        for a in &sim.agents {
            if wants_shelter(a, cx.home(a), &cx) {
                ws += 1;
                if a.inventory.wood >= 20.0 {
                    wood_ok += 1;
                }
                if a.inventory.stone >= 10.0 {
                    stone_ok += 1;
                }
            }
            let (_, st) = material_wants(a, &cx);
            if st > 0.0 {
                want_stone += 1;
            }
            if a.memory
                .resources
                .iter()
                .any(|m| m.resource == accrete_core::inventory::Resource::Stone)
            {
                stone_mem += 1;
            }
            goals[a.goal.kind as usize] += 1;
        }
        println!(
            "  want shelter {ws} (wood ok {wood_ok}, stone ok {stone_ok}); want stone {want_stone}; know stone {stone_mem}; goals {goals:?}"
        );
        let full = sim
            .world
            .homes
            .iter()
            .filter(|h| h.residents.len() >= 4)
            .count();
        println!(
            "  homes with >=4 residents: {full}; droughts so far: {}",
            sim.weather.history.len()
        );
    }
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    let n = rows.len();
    for q in [0.0, 0.1, 0.25, 0.5, 0.75, 0.9, 1.0] {
        let i = ((n as f32 - 1.0) * q) as usize;
        let r = rows[i];
        println!(
            "q{q:.2}: ratio {:.2} potential {:.1} known-people {:.0} actual-people {} homes-in-memory {}",
            r.0, r.1, r.2, r.3, r.4
        );
    }
}

/// Spatial structure of occupied homes: nearest-neighbour distances and
/// cluster sizes under single linkage at several radii.
pub fn home_structure(sim: &Simulation) {
    use accrete_core::simulation::settlement::clusters;
    let homes: Vec<_> = sim
        .world
        .homes
        .iter()
        .filter(|h| !h.is_vacant())
        .map(|h| (h.id, h.pos, h.residents.len() as u32))
        .collect();
    let mut nn: Vec<i32> = homes
        .iter()
        .map(|a| {
            homes
                .iter()
                .filter(|b| b.0 != a.0)
                .map(|b| a.1.chebyshev(b.1))
                .min()
                .unwrap_or(99)
        })
        .collect();
    nn.sort_unstable();
    let pct = |q: f32| {
        nn.get(((nn.len() as f32 - 1.0) * q) as usize)
            .copied()
            .unwrap_or(0)
    };
    println!(
        "  homes {}: nearest-neighbour distance p10 {} p50 {} p90 {}; shared tiles {}",
        homes.len(),
        pct(0.1),
        pct(0.5),
        pct(0.9),
        nn.iter().filter(|&&d| d == 0).count()
    );
    for r in [2, 3, 5] {
        let mut sizes: Vec<(usize, u32)> = clusters(&homes, r)
            .iter()
            .map(|g| (g.len(), g.iter().map(|&i| homes[i].2).sum()))
            .collect();
        sizes.sort_by(|a, b| b.cmp(a));
        let top: Vec<String> = sizes
            .iter()
            .take(8)
            .map(|(h, p)| format!("{h}h/{p}p"))
            .collect();
        println!(
            "  linkage R={r}: {} clusters; largest {}",
            sizes.len(),
            top.join(" ")
        );
    }
}
