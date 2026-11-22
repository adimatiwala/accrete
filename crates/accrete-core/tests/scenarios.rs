//! Week 2 behavioural scenarios (accrete.md, "Behavioural evaluation").
//!
//! Each scenario is a tiny hand-built world from `tests/scenarios/`. Tests
//! check *outcomes within a window*, not exact traces, so they keep passing
//! while utility weights are retuned as long as behaviour stays sensible.

use std::path::PathBuf;

use accrete_core::agents::agent::GoalKind;
use accrete_core::agents::pathfinding::{AgentBelief, PathScratch};
use accrete_core::inventory::Resource;
use accrete_core::scenario::Scenario;
use accrete_core::simulation::Intent;
use accrete_core::{Config, Pos, Simulation};

fn scenario_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("scenarios")
        .join(name)
}

fn config() -> Config {
    let mut c = Config::default();
    c.sim.check_invariants = true;
    c
}

fn load(name: &str) -> Simulation {
    let sc = Scenario::load(&scenario_path(name)).expect("scenario parses");
    sc.build(&config()).expect("scenario builds")
}

#[test]
fn hungry_agent_walks_to_known_food_and_eats() {
    let mut sim = load("hungry_known_food.toml");
    let start_food = sim.agents[0].inventory.food;
    assert_eq!(start_food, 0.0);
    let mut ate_at = None;
    for t in 0..12 {
        sim.step();
        if sim.agents[0].stats.food_eaten > 0.0 {
            ate_at = Some(t);
            break;
        }
    }
    assert!(ate_at.is_some(), "did not eat within 12 ticks");
    assert!(sim.agents[0].stats.food_gathered > 0.0);
}

#[test]
fn hungry_agent_without_known_food_explores() {
    let mut sim = load("hungry_no_food.toml");
    let mut explored = false;
    let mut idle = 0;
    for _ in 0..12 {
        sim.step();
        if sim.agents[0].goal.kind == GoalKind::Explore {
            explored = true;
        }
        if sim.intent_of(sim.agents[0].id) == Some(Intent::Idle) {
            idle += 1;
        }
    }
    assert!(explored, "hungry agent with no known food never explored");
    assert!(idle <= 2, "idled {idle} ticks instead of exploring");
    assert!(
        sim.agents[0].memory.known_count() > 40,
        "exploring should reveal new land"
    );
}

#[test]
fn exhausted_agent_rests_before_energy_runs_out() {
    let mut sim = load("exhausted.toml");
    let mut rested = false;
    for _ in 0..24 {
        sim.step();
        assert!(sim.agents[0].energy > 0.0, "energy reached 0");
        if sim.intent_of(sim.agents[0].id) == Some(Intent::Rest) {
            rested = true;
        }
    }
    assert!(rested);
}

#[test]
fn food_behind_a_ridge_is_unknown_until_seen() {
    let mut sim = load("hidden_food.toml");
    let hidden = Pos::new(6, 2);
    let a = &sim.agents[0];
    assert!(
        !a.memory.resources.iter().any(|m| m.pos == hidden),
        "the ridge should hide the food"
    );
    assert!(
        !a.memory.knows(hidden),
        "the tile behind the ridge should be unknown"
    );
    // Until the agent has seen the food, no goal may target it.
    for _ in 0..48 {
        sim.step();
        let a = &sim.agents[0];
        let knows_food = a.memory.resources.iter().any(|m| m.pos == hidden);
        if a.goal.kind == GoalKind::GatherFood && a.goal.target == hidden {
            assert!(knows_food, "agent was drawn to food it had not seen");
        }
    }
}

#[test]
fn routes_use_only_known_tiles() {
    let sim = load("unknown_shortcut.toml");
    let a = &sim.agents[0];
    let mut scratch = PathScratch::default();
    // The far end of the top row is beyond sight (13 tiles away): unknown.
    let far = Pos::new(13, 2);
    assert!(!a.memory.knows(far));
    let r = scratch.find_path(
        &sim.world,
        &AgentBelief(&a.memory),
        a.pos,
        far,
        &sim.config.movement,
        4000,
    );
    assert!(r.is_err(), "routed to an unknown tile");
    // A known target: every step must be known and passable.
    let known = Pos::new(5, 2);
    if a.memory.knows(known) {
        let path = scratch
            .find_path(
                &sim.world,
                &AgentBelief(&a.memory),
                a.pos,
                known,
                &sim.config.movement,
                4000,
            )
            .expect("known target reachable");
        for p in &path.steps {
            assert!(a.memory.knows(*p), "route crosses unknown tile {p:?}");
            assert!(sim.world.tile(*p).is_land(), "route crosses water {p:?}");
        }
    }
}

#[test]
fn scarce_food_is_split_without_duplication() {
    let mut sim = load("shared_scarce_food.toml");
    let tile = Pos::new(4, 1);
    let initial = sim.world.tile(tile).food;
    for _ in 0..4 {
        sim.step();
    }
    let gathered: f32 = sim.agents.iter().map(|a| a.stats.food_gathered).sum();
    assert!(
        gathered <= initial + 1e-4,
        "gathered {gathered} > {initial}"
    );
    assert!(sim.world.tile(tile).food >= 0.0);
    assert!(
        sim.agents.iter().all(|a| a.stats.food_gathered > 0.0),
        "both agents should get a share"
    );
    // After the tile is empty, neither waits on it forever.
    for _ in 0..24 {
        sim.step();
    }
    for a in &sim.agents {
        assert!(
            !(a.goal.kind == GoalKind::GatherFood && a.goal.target == tile),
            "agent {} still waiting on the empty tile",
            a.id
        );
    }
    sim.check_invariants().expect("conservation");
}

#[test]
fn overcrowded_home_sheds_a_resident() {
    let mut sim = load("overcrowded_home.toml");
    assert_eq!(sim.world.homes[0].residents.len(), 6);
    let mut moved_out = false;
    for _ in 0..(14 * 24) {
        sim.step();
        if sim.world.homes[0].residents.len() < 6 {
            moved_out = true;
            break;
        }
    }
    assert!(moved_out, "nobody left the overcrowded home within 14 days");
}

#[test]
fn eligible_but_hungry_agent_eats_first() {
    let mut sim = load("reproduce_but_hungry.toml");
    sim.step();
    assert_eq!(
        sim.intent_of(sim.agents[0].id),
        Some(Intent::Eat),
        "should eat before reproducing"
    );
}

#[test]
fn full_inventory_is_deposited() {
    let mut sim = load("full_inventory.toml");
    let mut deposited = false;
    for _ in 0..3 {
        sim.step();
        if sim.world.homes[0].stored().food > 0.0 {
            deposited = true;
            break;
        }
    }
    assert!(
        deposited,
        "agent with full hands did not use the chest at home"
    );
    assert!(sim.agents[0].inventory.get(Resource::Food) < 30.0);
    sim.check_invariants().expect("conservation");
}

// ---------------------------------------------------------------------------
// Week 3 scenarios: trade, reciprocity, road building.
// ---------------------------------------------------------------------------

fn trades(sim: &Simulation) -> Vec<(u32, u32)> {
    sim.events
        .events
        .iter()
        .filter_map(|e| match e.kind {
            accrete_core::simulation::events::EventKind::Traded { partner, .. } => {
                Some((e.agent.unwrap(), partner))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn complementary_surpluses_trade() {
    let mut sim = load("trade_complementary.toml");
    let before: f32 = sim
        .agents
        .iter()
        .map(|a| a.inventory.food + a.inventory.wood)
        .sum();
    for _ in 0..3 {
        sim.step();
    }
    assert!(
        !trades(&sim).is_empty(),
        "food-rich and wood-rich neighbours did not trade"
    );
    assert!(
        sim.agents[0].inventory.wood > 0.0,
        "the food holder should have received wood"
    );
    let after: f32 = sim
        .agents
        .iter()
        .map(|a| a.inventory.food + a.inventory.wood)
        .sum();
    // Only eating and spoilage can reduce the total; trade never creates goods.
    assert!(after <= before + 1e-3);
    sim.check_invariants().expect("conservation");
}

#[test]
fn identical_surpluses_do_not_trade() {
    let mut sim = load("trade_identical.toml");
    for _ in 0..24 {
        sim.step();
    }
    assert!(
        trades(&sim).is_empty(),
        "agents with identical surpluses traded"
    );
}

#[test]
fn low_reciprocity_partner_gets_worse_terms() {
    let sim = load("trade_reciprocity.toml");
    // Agents 1 (distrusted by 0) and 2 (trusted by 0) make the same offer to 0.
    let bad = sim
        .best_offer(1, 0, sim.tick)
        .expect("distrusted agent can still trade");
    let good = sim
        .best_offer(2, 0, sim.tick)
        .expect("trusted agent can trade");
    let rate = |o: &accrete_core::simulation::social::TradeOffer| o.get_qty / o.give_qty;
    assert!(
        rate(&bad) < rate(&good),
        "distrusted partner should get worse terms"
    );
    // When agent 0 chooses whom to approach, it prefers the trusted partner.
    let me = &sim.agents[0];
    assert!(me.memory.reciprocity(2) > me.memory.reciprocity(1));
}

#[test]
fn busy_household_trail_becomes_a_road() {
    let mut sim = load("road_busy_trail.toml");
    let tile = Pos::new(2, 2);
    let mut built = false;
    for _ in 0..24 {
        sim.step();
        if sim.world.tile(tile).path_level == accrete_core::world::PathLevel::Road {
            built = true;
            break;
        }
    }
    assert!(
        built,
        "ImproveTrail was not chosen on a busy household trail"
    );
    sim.check_invariants().expect("materials accounted for");
}

#[test]
fn selfish_agent_with_few_crossings_does_not_build() {
    let mut c = config();
    c.infrastructure.household_weight = 0.0;
    let sc = Scenario::load(&scenario_path("road_selfish_few.toml")).unwrap();
    let mut sim = sc.build(&c).unwrap();
    for _ in 0..24 {
        sim.step();
        assert_ne!(sim.agents[0].goal.kind, GoalKind::ImproveTrail);
    }
    assert_ne!(
        sim.world.tile(Pos::new(2, 2)).path_level,
        accrete_core::world::PathLevel::Road
    );
}

#[test]
fn blocked_gap_is_noticed_and_avoided() {
    use accrete_core::config::{DisruptionKind, ScriptedDisruption};
    use accrete_core::world::Disruption;
    let mut sim = load("blocked_route.toml");
    let gap = accrete_core::Pos::new(5, 3);
    // Block the near gap after the agent's first observation, so it starts
    // out believing the gap is open.
    let block = ScriptedDisruption {
        start_day: 0,
        days: 30,
        kind: DisruptionKind::Block,
        slow_factor: 3,
        center: [5, 3],
        radius: 0,
        busiest_tiles: 0,
    };
    sim.disruptions.update(0, &mut sim.world, &[block]);
    assert_eq!(sim.world.disruption(gap), Disruption::Blocked);
    assert_eq!(
        sim.agents[0].memory.believed_disruption(gap),
        Disruption::None
    );

    let mut reached = false;
    for _ in 0..48 {
        sim.step();
        assert_ne!(sim.agents[0].pos, gap, "walked onto a blocked tile");
        if sim.agents[0].stats.food_gathered > 0.0 {
            reached = true;
            break;
        }
    }
    assert_eq!(
        sim.agents[0].memory.believed_disruption(gap),
        Disruption::Blocked,
        "the agent learned the gap is blocked"
    );
    assert!(
        reached,
        "the agent did not reach the food through the other gap"
    );
}

// ---------------------------------------------------------------------------
// Tools and crafting (A.26).
// ---------------------------------------------------------------------------

fn load_with_tools(name: &str) -> Simulation {
    let mut c = config();
    c.tools.enabled = true;
    Scenario::load(&scenario_path(name))
        .expect("scenario parses")
        .build(&c)
        .expect("scenario builds")
}

#[test]
fn agent_without_a_tool_crafts_one() {
    let mut sim = load_with_tools("tools_craft.toml");
    for _ in 0..12 {
        sim.step();
        if sim.agents[0].inventory.tools > 0.0 {
            break;
        }
    }
    let a = &sim.agents[0];
    assert!(
        a.inventory.tools > 0.9,
        "no tool crafted: {:?}",
        a.inventory
    );
    assert_eq!(a.inventory.wood, 0.0, "crafting consumes the wood");
    assert_eq!(a.inventory.stone, 0.0, "crafting consumes the stone");
    assert!(a.craft.practice > 0.0, "crafting is practised");
    sim.check_invariants().expect("tools are conserved");
}

#[test]
fn crafting_is_unavailable_when_tools_are_disabled() {
    let mut sim = load("tools_craft.toml");
    for _ in 0..12 {
        sim.step();
        assert_ne!(sim.agents[0].goal.kind, GoalKind::Craft);
    }
    assert_eq!(sim.agents[0].inventory.tools, 0.0);
}

#[test]
fn a_tool_speeds_gathering_and_wears_out() {
    let mut sim = load_with_tools("tools_gather.toml");
    // Same skills for a fair comparison.
    for a in sim.agents.iter_mut() {
        for s in a.skills.iter_mut() {
            s.aptitude = 1.0;
        }
    }
    for _ in 0..3 {
        sim.step();
    }
    let (with, without) = (&sim.agents[0], &sim.agents[1]);
    assert!(
        with.stats.food_gathered > without.stats.food_gathered * 1.3,
        "tool holder gathered {} vs {}",
        with.stats.food_gathered,
        without.stats.food_gathered
    );
    assert!(with.inventory.tools < 1.0, "the tool wears with use");
    sim.check_invariants()
        .expect("worn tools are accounted for");
}

// ---------------------------------------------------------------------------
// Culture: payoff-biased social learning (A.27).
// ---------------------------------------------------------------------------

fn culture_pair(enabled: bool) -> Simulation {
    let mut c = config();
    c.culture.enabled = enabled;
    let mut sim = Scenario::load(&scenario_path("culture_pair.toml"))
        .unwrap()
        .build(&c)
        .unwrap();
    // Agent 1 says foraging goes much better for it.
    sim.agents[0].forage_rate = 1.0;
    sim.agents[1].forage_rate = 2.0;
    sim
}

#[test]
fn the_worse_forager_copies_the_better_one() {
    let mut sim = culture_pair(true);
    let before = (sim.agents[0].traits, sim.agents[1].traits);
    for _ in 0..8 {
        sim.step();
    }
    let (learner, model) = (sim.agents[0].traits, sim.agents[1].traits);
    assert!(
        learner.curiosity > before.0.curiosity,
        "moved toward the better forager"
    );
    assert!(learner.caution > before.0.caution);
    // The better forager does not copy the worse one.
    assert_eq!(model, before.1);
    assert!(sim.today.trait_copies > 0, "learning events are counted");
}

#[test]
fn no_learning_when_culture_is_disabled() {
    let mut sim = culture_pair(false);
    let before = sim.agents[0].traits;
    for _ in 0..8 {
        sim.step();
    }
    assert_eq!(sim.agents[0].traits, before);
}
