//! Utility-based decisions.
//!
//! Each tick an agent scores every option it can see and picks the best:
//!
//! ```text
//! U(a, s) = Σ_i w_i(s) · V_i(a, s) − C(a, s)
//! ```
//!
//! where the `V_i` are need-relief terms (hunger, energy, food security, a
//! home, storage, offspring, new land) weighted by the agent's state and
//! traits, and `C` is the time the option takes (travel + work) priced at the
//! agent's value of time. Every term is recorded so the inspector can explain
//! the choice. The current goal gets a small commitment bonus (hysteresis) to
//! prevent dithering between near-equal options.
//!
//! **Information rule:** options are built only from the agent's memory, the
//! tiles it knows (terrain is static, so a known tile's terrain, fertility and
//! lushness are known), its own household, and its own body. Nothing here
//! reads undiscovered resources, other households' private state, settlement
//! detection, or population-wide statistics. The only global input is the
//! population safety cap, an engine limit that makes `Reproduce` unavailable.

use serde::Serialize;

use crate::agents::agent::{Agent, GoalKind, HomePlan};
use crate::agents::memory::freshness;
use crate::agents::pathfinding::line_cost_estimate;
use crate::config::{Config, TickRates};
use crate::geom::Pos;
use crate::hash::{mix_words, mix64};
use crate::inventory::Resource;
use crate::rng::unit_f32_from_u64;
use crate::world::visibility::SIGHT_RADIUS;
use crate::world::{Biome, Home, HomeId, World};

pub const MAX_TERMS: usize = 7;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Term {
    pub name: &'static str,
    pub value: f32,
}

/// A candidate's named utility terms, stored inline (A.31): candidates are
/// built for every option of every agent every tick, and a heap allocation
/// each was a measurable share of run time. Reads as a slice.
#[derive(Clone, Copy, Debug)]
pub struct Terms {
    len: u8,
    items: [Term; MAX_TERMS],
}

impl Default for Terms {
    fn default() -> Self {
        Self {
            len: 0,
            items: [Term {
                name: "",
                value: 0.0,
            }; MAX_TERMS],
        }
    }
}

impl Terms {
    fn push(&mut self, t: Term) {
        self.items[self.len as usize] = t;
        self.len += 1;
    }
}

impl std::ops::Deref for Terms {
    type Target = [Term];
    fn deref(&self) -> &[Term] {
        &self.items[..self.len as usize]
    }
}

impl PartialEq for Terms {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl Serialize for Terms {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        (**self).serialize(s)
    }
}

/// One scored option.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Candidate {
    pub kind: GoalKind,
    pub target: Pos,
    pub join: Option<HomeId>,
    pub score: f32,
    pub est_ticks: f32,
    pub terms: Terms,
}

impl Candidate {
    fn new(kind: GoalKind, target: Pos) -> Self {
        Self {
            kind,
            target,
            join: None,
            score: 0.0,
            est_ticks: 0.0,
            terms: Terms::default(),
        }
    }

    fn term(mut self, name: &'static str, value: f32) -> Self {
        debug_assert!(value.is_finite(), "term {name} not finite");
        self.score += value;
        if self.terms.len() < MAX_TERMS {
            self.terms.push(Term { name, value });
        }
        self
    }

    fn eta(mut self, ticks: f32) -> Self {
        self.est_ticks = ticks;
        self
    }
}

/// Why the agent chose what it did (latest decision only).
#[derive(Clone, Debug, PartialEq, Serialize, Default)]
pub struct Trace {
    pub tick: u64,
    /// Best first; the chosen option is `options[0]` unless routing failed.
    pub options: Vec<Candidate>,
    pub chosen: Option<Candidate>,
}

/// Read-only context for one decision.
pub struct Ctx<'a> {
    pub world: &'a World,
    pub config: &'a Config,
    pub rates: &'a TickRates,
    pub tick: u64,
    /// Engine safety limit: births are unavailable at the population cap.
    pub population_at_cap: bool,
    /// All agents, sorted by id. Decisions may read only the agent's *own
    /// household* from this (residents share their frequent-tile lists).
    pub agents: &'a [Agent],
}

impl Ctx<'_> {
    pub fn home<'h>(&'h self, agent: &Agent) -> Option<&'h Home> {
        agent.home.map(|h| &self.world.homes[h as usize])
    }

    /// Fellow residents of the agent's home (household knowledge only).
    pub fn housemates<'h>(&'h self, agent: &'h Agent) -> impl Iterator<Item = &'h Agent> + 'h {
        let ids: &[u32] = self.home(agent).map_or(&[], |h| h.residents.as_slice());
        ids.iter()
            .filter(move |&&id| id != agent.id)
            .filter_map(move |&id| {
                self.agents
                    .binary_search_by_key(&id, |a| a.id)
                    .ok()
                    .map(|i| &self.agents[i])
            })
    }
}

/// The agent wants a shelter improvement at home: the home is full or
/// crowded and can still be improved.
pub fn wants_shelter(agent: &Agent, home: Option<&Home>, cx: &Ctx) -> bool {
    let ic = &cx.config.infrastructure;
    if !ic.shelters_enabled || agent.is_juvenile(cx.rates) {
        return false;
    }
    match home {
        None => false,
        Some(h) => {
            u32::from(h.shelter_level) < ic.max_shelter
                && h.residents.len() as u32
                    >= h.comfortable_capacity(cx.config.homes.comfortable_capacity)
        }
    }
}

/// Perceived road value of the worn trail at `p`, in ticks over the road
/// horizon: `(benefit, cost)` where
/// `benefit = time_saved_per_crossing * (own + household_weight * sociability * household)`
/// using the agent's and its housemates' frequent-tile counts (about the
/// crossings in the last 30 days, taken as the expectation for the next 30),
/// plus an optional estimate of other travellers from observed trail wear,
/// and `cost = labour (less for diligent agents) + the materials' gathering time`.
pub fn road_value(agent: &Agent, cx: &Ctx, p: Pos) -> (f32, f32) {
    let ic = &cx.config.infrastructure;
    let mc = &cx.config.movement;
    let t = cx.world.tile(p);
    let trail =
        crate::world::movement::cost_for(t.biome, t.is_river, crate::world::PathLevel::Trail, mc);
    let road =
        crate::world::movement::cost_for(t.biome, t.is_river, crate::world::PathLevel::Road, mc);
    let (Some(trail), Some(road)) = (trail, road) else {
        return (0.0, f32::INFINITY);
    };
    let saved = trail.saturating_sub(road) as f32 / mc.mp_per_tick as f32;
    let horizon = ic.road_horizon_days / 30.0;
    let own = agent.memory.crossings_at(p) * horizon;
    let household: f32 = cx
        .housemates(agent)
        .map(|o| o.memory.crossings_at(p))
        .sum::<f32>()
        * horizon;
    let mut benefit = saved * (own + ic.household_weight * agent.traits.sociability * household);
    // Wear is visible here and remembered for other tiles (A.33). Under
    // steady use it is roughly crossings-per-day / (1 - decay).
    if ic.public_weight > 0.0 {
        let wear = if agent.pos == p || omniscient(cx.config) {
            t.traffic
        } else {
            agent.memory.believed_traffic(p)
        };
        let per_30_days = wear * (1.0 - cx.config.trails.decay) * 30.0;
        let others = (per_30_days - agent.memory.crossings_at(p)).max(0.0) * horizon;
        benefit += saved * ic.public_weight * agent.traits.sociability * others;
    }
    let cost = ic.road_ticks as f32 * (1.3 - 0.6 * agent.traits.diligence) + ic.road_material_ticks;
    (benefit, cost)
}

/// Building materials the agent wants to *carry*: `(wood, stone)`, for
/// storage it will build, materials still missing from the household's
/// stores for a shelter (too heavy to carry at once, so they are staged in the
/// home chests), and roads on the busiest worn trails it uses.
pub fn material_wants(agent: &Agent, cx: &Ctx) -> (f32, f32) {
    let (w, s, _, _) = material_plan(agent, cx);
    (w, s)
}

/// `(wood_want, stone_want, wood_keep, stone_keep)`: what the agent wants to
/// hold, and how much of it should stay in hand (the rest is for the
/// household's stores).
pub fn material_plan(agent: &Agent, cx: &Ctx) -> (f32, f32, f32, f32) {
    let c = cx.config;
    let ic = &c.infrastructure;
    let home = cx.home(agent);
    let mut wood = 0.0;
    let mut stone = 0.0;
    let mut wood_keep = 0.0;
    let mut stone_keep = 0.0;
    if wants_storage(agent, home, cx) {
        wood += c.inventory.storage_wood;
        wood_keep += c.inventory.storage_wood;
    }
    if let Some(h) = home.filter(|h| wants_shelter(agent, Some(h), cx) && !h.chests.is_empty()) {
        // Household knowledge: residents know what their own chests hold.
        let stored = h.stored();
        wood += (ic.shelter_wood - stored.wood).max(0.0);
        stone += (ic.shelter_stone - stored.stone).max(0.0);
    }
    if ic.roads_enabled && !agent.is_juvenile(cx.rates) {
        let prospects = agent
            .memory
            .frequent
            .iter()
            .filter(|f| {
                cx.world.contains(f.pos)
                    && believed_path(agent, cx, f.pos) == crate::world::PathLevel::Trail
                    && {
                        let (b, cost) = road_value(agent, cx, f.pos);
                        b > cost
                    }
            })
            .count()
            .min(3) as f32;
        wood += ic.road_wood * prospects;
        stone += ic.road_stone * prospects;
        wood_keep += ic.road_wood * prospects;
        stone_keep += ic.road_stone * prospects;
    }
    if wants_to_craft(agent, cx) {
        wood += c.tools.craft_wood;
        stone += c.tools.craft_stone;
        wood_keep += c.tools.craft_wood;
        stone_keep += c.tools.craft_stone;
    }
    (wood, stone, wood_keep, stone_keep)
}

/// The agent's believed exchange rate, food per tool: learned from its own
/// trades and from conversation, else the prior.
pub fn tool_price_belief(agent: &Agent, cx: &Ctx) -> f32 {
    if agent.memory.tool_price > 0.0 {
        agent.memory.tool_price
    } else {
        cx.config.tools.price_prior
    }
}

/// Tools one crafting batch yields for this agent (its crafting skill).
pub fn craft_output(agent: &Agent, cx: &Ctx) -> f32 {
    cx.config.tools.output_per_batch * agent.craft.level()
}

/// An adult crafts when it lacks a tool, or when it believes a batch would
/// fetch more than `surplus_threshold_food` in exchange. Skilled crafters and
/// high believed prices make crafting for others worthwhile.
pub fn wants_to_craft(agent: &Agent, cx: &Ctx) -> bool {
    let tc = &cx.config.tools;
    tc.enabled
        && !agent.is_juvenile(cx.rates)
        && (agent.inventory.tools < tc.wanted
            || tool_price_belief(agent, cx) * craft_output(agent, cx) >= tc.surplus_threshold_food)
}

/// Estimated ticks to walk from `a` to `b` along a straight line, at believed costs.
pub fn travel_ticks(agent: &Agent, cx: &Ctx, a: Pos, b: Pos) -> f32 {
    if a == b {
        return 0.0;
    }
    let mc = &cx.config.movement;
    let mp = if omniscient(cx.config) {
        line_cost_estimate(cx.world, &crate::agents::pathfinding::Omniscient, a, b, mc)
    } else {
        agent
            .line_memo
            .lock()
            .line_cost(cx.tick, cx.world, &agent.memory, a, b, mc)
    };
    mp as f32 / mc.mp_per_tick as f32
}

/// Whether agents run as the omniscient baseline (evaluation only): they
/// then plan, score and route with the true world instead of memory.
pub fn omniscient(c: &crate::config::Config) -> bool {
    c.sim.agent_kind == crate::config::AgentKind::Omniscient
}

/// The path level the agent believes `p` has: from its memory, or the true
/// level for the omniscient baseline.
pub fn believed_path(agent: &Agent, cx: &Ctx, p: Pos) -> crate::world::PathLevel {
    if omniscient(cx.config) {
        cx.world.tile(p).path_level
    } else {
        agent.memory.believed_path(p)
    }
}

/// Desired carried reserve of food (units).
pub fn reserve_target(agent: &Agent, cx: &Ctx) -> f32 {
    let d = &cx.config.decision;
    agent.daily_food_need(cx.config, cx.rates)
        * (d.reserve_days_base + agent.traits.caution * d.reserve_days_caution)
}

pub fn eat_threshold(agent: &Agent, cx: &Ctx) -> f32 {
    let d = &cx.config.decision;
    d.eat_threshold_base - agent.traits.caution * d.eat_threshold_caution
}

pub fn rest_threshold(agent: &Agent, cx: &Ctx) -> f32 {
    let d = &cx.config.decision;
    d.rest_threshold_base - agent.traits.diligence * d.rest_threshold_diligence
        + agent.traits.caution * d.rest_threshold_caution
}

/// Reasons `Reproduce` is unavailable (empty = eligible, apart from being at home).
pub fn reproduce_blockers(agent: &Agent, cx: &Ctx) -> Vec<&'static str> {
    let c = cx.config;
    let r = cx.rates;
    let mut why = Vec::new();
    if agent.age_ticks < r.maturity_ticks {
        why.push("juvenile");
    }
    if agent.age_ticks >= r.elder_ticks {
        why.push("past elder age");
    }
    let Some(home) = cx.home(agent) else {
        why.push("no home");
        return why;
    };
    if agent.health < agent.health_ceiling(r) - c.life.reproduce_health_margin {
        why.push("health below ceiling margin");
    }
    let stored = if agent.pos == home.pos {
        home.stored().food
    } else {
        agent.memory.home_food_seen
    };
    if agent.inventory.food < c.life.reproduce_food_carried && stored < c.life.reproduce_food_stored
    {
        why.push("no food surplus");
    }
    if let Some(last) = agent.last_birth_tick {
        if cx.tick < last + r.reproduce_cooldown_ticks {
            why.push("cooldown");
        }
    }
    if cx.tick < agent.next_reproduce_try {
        why.push("recently tried");
    }
    if is_crowded(home, cx) {
        why.push("home overcrowded");
    }
    if cx.population_at_cap {
        why.push("population cap");
    }
    why
}

/// A home is crowded once it has been over capacity for longer than the grace period.
pub fn is_crowded(home: &Home, cx: &Ctx) -> bool {
    match home.over_capacity_since {
        Some(since) => cx.tick.saturating_sub(since) > cx.rates.crowding_grace_ticks,
        None => false,
    }
}

/// Deterministic per-agent jitter in `[-amp/2, amp/2]`, independent of
/// iteration order and of every RNG stream.
fn jitter(agent: &Agent, tick: u64, kind: GoalKind, target: Pos, amp: f32) -> f32 {
    let h = mix_words(&[
        u64::from(agent.id),
        tick,
        kind as u64,
        (target.x as u32 as u64) << 32 | target.y as u32 as u64,
    ]);
    (unit_f32_from_u64(h) - 0.5) * amp
}

/// Score all options. Returns candidates sorted best first.
pub fn evaluate(agent: &Agent, cx: &Ctx) -> Vec<Candidate> {
    let c = cx.config;
    let d = &c.decision;
    let r = cx.rates;
    let t = agent.traits;
    let tc = d.time_cost;
    let juvenile = agent.is_juvenile(r);
    let need = agent.daily_food_need(c, r);
    let reserve = reserve_target(agent, cx);
    let food = agent.inventory.food;
    let hunger = agent.hunger;
    let energy = agent.energy;
    let cap = agent.carry_capacity(c, r);
    let home = cx.home(agent);
    let home_pos = home.map(|h| h.pos);
    let at_home = home_pos == Some(agent.pos);
    let leash_ok = |p: Pos| match (juvenile, home_pos) {
        (true, Some(h)) => p.chebyshev(h) <= c.life.juvenile_leash as i32,
        _ => true,
    };
    let travel = |p: Pos| travel_ticks(agent, cx, agent.pos, p);
    let trip_home_from = |p: Pos| match home_pos {
        Some(h) => travel_ticks(agent, cx, p, h) * d.home_pull,
        None => 0.0,
    };

    let mut out: Vec<Candidate> = Vec::with_capacity(24);

    // --- Eat ----------------------------------------------------------------
    let eat_thr = eat_threshold(agent, cx);
    if food >= 0.5 && hunger >= eat_thr {
        let urgency = (hunger - eat_thr) / (1.0 - eat_thr).max(1e-3);
        // Eating takes one tick, so once hungry it should beat almost anything.
        out.push(
            Candidate::new(GoalKind::Eat, agent.pos)
                .term("base", d.eat_weight * 0.5)
                .term("hunger", d.eat_weight * 2.0 * urgency),
        );
    }

    // --- Rest ---------------------------------------------------------------
    let rest_thr = rest_threshold(agent, cx);
    let resting = agent.goal.kind == GoalKind::Rest;
    let fatigue = if resting && energy < d.rest_until {
        0.5 + (d.rest_until - energy)
    } else if energy < rest_thr {
        0.5 + (rest_thr - energy) / rest_thr.max(1e-3)
    } else {
        0.0
    };
    // With nothing better to do an agent rests (leisure) rather than idling.
    let leisure = 0.03 + 0.12 * (1.0 - energy);
    let crowd_factor = home.map_or(1.0, |h| {
        if is_crowded(h, cx) {
            let extra = h.residents.len() as f32
                - h.comfortable_capacity(c.homes.comfortable_capacity) as f32;
            1.0 - (c.homes.crowding_rest_penalty * extra).min(c.homes.crowding_rest_penalty_max)
        } else {
            1.0
        }
    });
    let home_rest_bonus = (c.needs.rest_per_tick_home / c.needs.rest_per_tick) * crowd_factor - 1.0;
    if fatigue > 0.0 || leisure > 0.0 {
        let mut cand = Candidate::new(GoalKind::Rest, agent.pos)
            .term("fatigue", d.rest_weight * fatigue)
            .term("leisure", d.rest_weight * leisure);
        if at_home {
            cand = cand.term(
                "home bed",
                d.rest_weight * (fatigue + leisure) * home_rest_bonus,
            );
        }
        out.push(cand);
    }
    if let Some(hp) = home_pos {
        if !at_home && fatigue > 0.0 {
            let tt = travel(hp);
            // Only walk home if energy will last the trip. Once committed, keep
            // going while any energy remains (no flip-flopping on the margin).
            let committed = agent.goal.kind == GoalKind::Rest && agent.goal.target == hp;
            let reserve_needed = if committed { 0.0 } else { 0.05 };
            if tt <= 6.0 && energy - tt * c.needs.energy_per_active_tick > reserve_needed {
                out.push(
                    Candidate::new(GoalKind::Rest, hp)
                        .term("fatigue", d.rest_weight * fatigue)
                        .term("home bed", d.rest_weight * fatigue * home_rest_bonus)
                        .term("travel", -tc * tt * 2.0)
                        .eta(tt),
                );
            }
        }
    }

    // --- Gather food --------------------------------------------------------
    let free_food = agent.inventory.room_for(Resource::Food, cap, &c.inventory);
    let household_extra = match home {
        Some(h) if !h.chests.is_empty() && !juvenile => {
            let residents = h.residents.len().max(1) as f32;
            let target = d.household_store_days * need * residents;
            let deficit = (target - agent.memory.home_food_seen)
                .max(0.0)
                .min(agent.memory.home_room_seen);
            deficit * (0.25 + 0.5 * t.diligence + 0.25 * t.sociability)
        }
        _ => 0.0,
    };
    let mut want_food = (reserve - food).max(0.0) + household_extra;
    if food < 1.0 && hunger >= eat_thr {
        want_food = want_food.max(need);
    }
    let mut best_food_score = f32::NEG_INFINITY;
    if want_food >= 0.5 && free_food >= 0.5 {
        let rate = agent.gather_rate(Resource::Food, c, r).max(0.05);
        let urgency = 1.0 + 2.0 * (hunger - 0.3).max(0.0) + if food < need { 0.5 } else { 0.0 };
        // (target, gain, work) for every remembered food place.
        let mut prelim: Vec<(Pos, f32, f32)> = Vec::new();
        for m in agent
            .memory
            .resources
            .iter()
            .filter(|m| m.resource == Resource::Food)
        {
            if m.unreachable_until > cx.tick || !leash_ok(m.pos) {
                continue;
            }
            let fresh = freshness(m.seen_tick, cx.tick, r.freshness_half_life_ticks);
            let expect = (m.amount * (0.5 + 0.5 * fresh))
                .min(free_food)
                .min(want_food);
            if expect < 0.5 {
                continue;
            }
            let work = (expect / rate).ceil();
            let gain = d.food_weight * (expect / reserve).min(1.5) * urgency;
            prelim.push((m.pos, gain, work));
        }
        let keys: Vec<(Pos, f32, usize)> = prelim
            .iter()
            .enumerate()
            .map(|(i, &(p, gain, work))| (p, gain - tc * work, i))
            .collect();
        let food_cands = shortlist(agent, cx, &keys, GoalKind::GatherFood, 1.0, |i| {
            let (p, gain, work) = prelim[i];
            let tt = travel(p);
            let back = trip_home_from(p);
            Candidate::new(GoalKind::GatherFood, p)
                .term("food", gain)
                .term("travel", -tc * tt)
                .term("work", -tc * work)
                .term("home trip", -tc * back)
                .eta(tt + work)
        });
        for cand in &food_cands {
            best_food_score = best_food_score.max(cand.score);
        }
        out.extend(food_cands);
    }

    // --- Gather building materials (wood, stone) ---------------------------
    let (wood_want, stone_want, wood_keep, stone_keep) = if juvenile {
        (0.0, 0.0, 0.0, 0.0)
    } else {
        material_plan(agent, cx)
    };
    let wood = agent.inventory.wood;
    let stone = agent.inventory.stone;
    for (res, want, held, kind) in [
        (Resource::Wood, wood_want, wood, GoalKind::GatherWood),
        (Resource::Stone, stone_want, stone, GoalKind::GatherStone),
    ] {
        let short = (want - held).max(0.0);
        if short <= 0.0 {
            continue;
        }
        let free = agent.inventory.room_for(res, cap, &c.inventory);
        let rate = agent.gather_rate(res, c, r).max(0.05);
        let mut prelim: Vec<(Pos, f32, f32)> = Vec::new();
        for m in agent.memory.resources.iter().filter(|m| m.resource == res) {
            if m.unreachable_until > cx.tick || !leash_ok(m.pos) {
                continue;
            }
            // Gather at least one tick's worth so a tiny shortfall still gets filled.
            let expect = m.amount.min(free).min(short.max(rate));
            if expect < 0.5 {
                continue;
            }
            let work = (expect / rate).ceil();
            // Value of this trip: the share of a full load (or of what is
            // still missing, if less) that it brings back.
            let load = short.min(free.max(rate));
            let value = d.materials_weight * (0.4 + 0.6 * t.diligence) * (expect / load).min(1.0);
            prelim.push((m.pos, value, work));
        }
        let keys: Vec<(Pos, f32, usize)> = prelim
            .iter()
            .enumerate()
            .map(|(i, &(p, value, work))| (p, value - tc * work, i))
            .collect();
        out.extend(shortlist(agent, cx, &keys, kind, 1.0, |i| {
            let (p, value, work) = prelim[i];
            let tt = travel(p);
            let back = trip_home_from(p);
            Candidate::new(kind, p)
                .term("materials", value)
                .term("travel", -tc * tt)
                .term("work", -tc * work)
                .term("home trip", -tc * back)
                .eta(tt + work)
        }));
    }

    // --- Craft tools (A.26) -------------------------------------------------
    let tcfg = &c.tools;
    if tcfg.enabled
        && !juvenile
        && wood >= tcfg.craft_wood
        && stone >= tcfg.craft_stone
        && hunger < eat_thr
        && wants_to_craft(agent, cx)
    {
        let target = home.map_or(agent.pos, |h| h.pos);
        let output = craft_output(agent, cx);
        let own = (tcfg.wanted - agent.inventory.tools).max(0.0).min(output);
        let surplus = (output - own).max(0.0);
        let price = tool_price_belief(agent, cx);
        let tt = travel(target);
        out.push(
            Candidate::new(GoalKind::Craft, target)
                .term("own tool", d.tool_weight * own)
                .term(
                    "tools to trade",
                    d.tool_trade_weight
                        * (price * surplus / reserve).min(1.0)
                        * (0.5 + 0.5 * t.diligence),
                )
                .term("travel", -tc * tt)
                .term("work", -tc * tcfg.craft_ticks as f32)
                .eta(tt + tcfg.craft_ticks as f32),
        );
    }

    // --- Improve the trail underfoot into a road ---------------------------
    let ic = &c.infrastructure;
    if ic.roads_enabled
        && !juvenile
        && cx.world.tile(agent.pos).path_level == crate::world::PathLevel::Trail
        && wood >= ic.road_wood
        && stone >= ic.road_stone
        && hunger < eat_thr
    {
        let (benefit, cost) = road_value(agent, cx, agent.pos);
        if benefit > cost {
            // Labour is priced at the agent's current value of time, which is
            // higher when hungry or tired, so struggling agents do not build.
            let vt = tc * (1.0 + 2.0 * (hunger - 0.3).max(0.0) + 2.0 * (0.5 - energy).max(0.0));
            out.push(
                Candidate::new(GoalKind::ImproveTrail, agent.pos)
                    .term("time saved", vt * benefit)
                    .term("labour+materials", -vt * cost)
                    .term("diligence", 0.1 * t.diligence)
                    .eta(ic.road_ticks as f32),
            );
        }
    }

    // --- Explore ------------------------------------------------------------
    let starving_for_options = best_food_score <= 0.0 && food < need;
    let desperation = if starving_for_options && !juvenile {
        d.explore_desperation * (0.5 + hunger)
    } else if starving_for_options {
        0.5 * d.explore_desperation * (0.5 + hunger)
    } else {
        0.0
    };
    let insecurity = (1.0 - food / reserve.max(1e-3)).max(0.0);
    // Land hunger: an adult who feels the land around home is crowded looks
    // further afield for somewhere better to live.
    let land_hunger = match agent.home_plan {
        Some(p) if !juvenile => {
            d.explore_weight * (1.0 - p.home_land).clamp(0.0, 1.0) * (0.5 + t.curiosity)
        }
        _ => 0.0,
    };
    let home_pull_scale = if land_hunger > 0.0 { 0.5 } else { 1.0 };
    let frontiers: Vec<(Pos, f32)> = frontier_targets(agent, cx)
        .into_iter()
        .filter(|&(f, _)| leash_ok(f) && !agent.memory.is_failed_target(f, cx.tick))
        .collect();
    let curiosity_w = d.explore_weight * (0.3 + 0.7 * t.curiosity);
    let caution_term = -0.4 * t.caution * insecurity * d.explore_weight;
    let travel_coef = 1.0 - 0.5 * t.curiosity;
    let keys: Vec<(Pos, f32, usize)> = frontiers
        .iter()
        .enumerate()
        .map(|(i, &(f, novelty))| {
            (
                f,
                curiosity_w * novelty + desperation + land_hunger + caution_term,
                i,
            )
        })
        .collect();
    out.extend(shortlist(
        agent,
        cx,
        &keys,
        GoalKind::Explore,
        travel_coef,
        |i| {
            let (f, novelty) = frontiers[i];
            let tt = travel(f);
            let back = trip_home_from(f);
            Candidate::new(GoalKind::Explore, f)
                .term("curiosity", curiosity_w * novelty)
                .term("desperation", desperation)
                .term("land hunger", land_hunger)
                .term("caution", caution_term)
                .term("travel", -tc * travel_coef * tt)
                .term("home trip", -tc * back * home_pull_scale)
                .eta(tt)
        },
    ));

    // --- Visit a market (A.33) ----------------------------------------------
    let mk = &c.markets;
    if mk.enabled
        && !juvenile
        && hunger < eat_thr
        && energy > 0.35
        && !agent.memory.trade_spots.is_empty()
    {
        // Something to swap: food beyond a comfortable reserve, spare
        // materials, spare tools; or something wanted that trading supplies.
        let spare_food = ((food - 1.5 * reserve) / reserve.max(1.0)).clamp(0.0, 1.0);
        let spare_materials = ((wood + stone - 4.0) / 4.0).clamp(0.0, 1.0);
        let spare_tools = (agent.inventory.tools - c.tools.wanted).clamp(0.0, 1.0);
        let want_tool = if c.tools.enabled && agent.inventory.tools < c.tools.wanted {
            1.0
        } else {
            0.0
        };
        let motive = (spare_food + spare_materials + spare_tools + want_tool).min(2.0)
            * (0.5 + 0.5 * t.sociability);
        if motive > 0.0 {
            for s in &agent.memory.trade_spots {
                if !leash_ok(s.pos) {
                    continue;
                }
                let draw = s.score / (s.score + mk.half_score);
                let waited = if agent.goal.kind == GoalKind::Market
                    && agent.goal.target == s.pos
                    && agent.pos == s.pos
                {
                    (cx.tick - agent.goal.since) as f32 / mk.wait_ticks.max(1) as f32
                } else {
                    0.0
                };
                let tt = travel(s.pos);
                let back = trip_home_from(s.pos);
                out.push(
                    Candidate::new(GoalKind::Market, s.pos)
                        .term("market", mk.weight * draw * motive)
                        .term("waited", -0.3 * waited * mk.weight)
                        .term("travel", -tc * tt)
                        .term("home trip", -tc * back)
                        .eta(tt + mk.wait_ticks as f32),
                );
            }
        }
    }

    // --- Home: found, join, or move -----------------------------------------
    if !juvenile {
        if let Some(plan) = agent.home_plan {
            let settled_long_enough =
                cx.tick.saturating_sub(agent.born_tick) >= r.settle_after_ticks;
            match (home, plan.current) {
                (None, _) if settled_long_enough => {
                    let tt = travel(plan.target);
                    let mut cand = Candidate::new(GoalKind::EstablishHome, plan.target)
                        .term("want home", d.home_weight * 0.5)
                        .term("site", d.home_weight * plan.score)
                        .term("travel", -tc * tt)
                        .eta(tt);
                    cand.join = plan.join;
                    out.push(cand);
                }
                (Some(h), Some(current))
                    if plan.target != h.pos || plan.join.is_some_and(|j| j != h.id) =>
                {
                    let margin = d.move_home_margin * (0.5 + t.caution);
                    let gain = plan.score - current;
                    if gain > margin {
                        let tt = travel(plan.target);
                        let mut cand = Candidate::new(GoalKind::EstablishHome, plan.target)
                            .term("better site", d.home_weight * gain)
                            .term("attachment", -d.home_weight * margin)
                            .term("travel", -tc * tt)
                            .eta(tt);
                        cand.join = plan.join;
                        out.push(cand);
                    }
                }
                _ => {}
            }
        }
    }

    if let Some(h) = home {
        let tt_home = travel(h.pos);
        // --- Build storage --------------------------------------------------
        if wants_storage(agent, Some(h), cx) && wood >= c.inventory.storage_wood {
            out.push(
                Candidate::new(GoalKind::BuildStorage, h.pos)
                    .term("storage", d.storage_weight * (0.6 + 0.4 * t.diligence))
                    .term("travel", -tc * tt_home)
                    .eta(tt_home + c.inventory.storage_ticks as f32),
            );
        }
        // --- Improve shelter ------------------------------------------------
        let stored_now = h.stored();
        if wants_shelter(agent, Some(h), cx)
            && wood + stored_now.wood >= ic.shelter_wood
            && stone + stored_now.stone >= ic.shelter_stone
        {
            let excess = h.residents.len() as f32 + 1.0
                - h.comfortable_capacity(c.homes.comfortable_capacity) as f32;
            out.push(
                Candidate::new(GoalKind::ImproveShelter, h.pos)
                    .term(
                        "room for family",
                        d.shelter_weight * (0.5 + 0.25 * excess.max(0.0)),
                    )
                    .term("diligence", d.shelter_weight * 0.3 * t.diligence)
                    .term("travel", -tc * tt_home)
                    .eta(tt_home + ic.shelter_ticks as f32),
            );
        }
        // --- Deposit ----------------------------------------------------------
        if !h.chests.is_empty() && agent.memory.home_room_seen >= 1.0 {
            let surplus_food = (food - reserve * 1.2).max(0.0);
            // Materials beyond what the agent itself will build with go to the
            // household's stores (e.g. toward a shelter).
            let surplus_wood = (wood - wood_keep).max(0.0);
            let surplus_stone = (stone - stone_keep).max(0.0);
            let surplus = surplus_food + surplus_wood * 2.0 + surplus_stone * 3.0;
            let fullness = agent.inventory.weight(&c.inventory) / cap;
            if surplus >= 2.0 {
                out.push(
                    Candidate::new(GoalKind::Deposit, h.pos)
                        .term(
                            "surplus",
                            d.deposit_weight * 0.6 * (surplus / reserve).min(1.0),
                        )
                        .term("full hands", d.deposit_weight * 0.6 * fullness)
                        .term("travel", -tc * tt_home)
                        .eta(tt_home),
                );
            }
        }
        // --- Withdraw ---------------------------------------------------------
        if agent.memory.home_food_seen >= 1.0 && food < reserve * 0.5 {
            let amount = (reserve - food).min(agent.memory.home_food_seen);
            out.push(
                Candidate::new(GoalKind::Withdraw, h.pos)
                    .term(
                        "food",
                        d.withdraw_weight * (amount / reserve) * (1.0 + hunger),
                    )
                    .term("travel", -tc * tt_home)
                    .eta(tt_home),
            );
        }
        // --- Reproduce --------------------------------------------------------
        // Another mouth to feed: the agent weighs how many days of food its
        // household holds per person (including the newborn) against the
        // security it wants, which grows with caution.
        if hunger < 0.35 && energy > 0.35 && reproduce_blockers(agent, cx).is_empty() {
            let residents = h.residents.len() as f32;
            let stored = if at_home {
                h.stored().food
            } else {
                agent.memory.home_food_seen
            };
            let days_per_person = (stored + food) / ((residents + 1.0) * need);
            let wanted = d.reproduce_security_days * (0.75 + 0.5 * t.caution);
            let security = days_per_person / wanted;
            // Can the land I know around home feed one more? (perceived food
            // potential per person who would share it, counting the child).
            let land = land_food_ratio(agent, cx, h.pos, 1.0);
            // Shortfalls weigh heavily (a child the household cannot feed is a
            // burden); comfortable surplus adds a little.
            // How well foraging has actually been going (experience, not theory).
            let returns = agent.forage_rate / d.reproduce_min_return.max(1e-3);
            let shortfall = d.reproduce_security_weight * (1.0 - security).max(0.0)
                + d.reproduce_land_weight * (1.0 - land).max(0.0)
                + d.reproduce_return_weight * (1.0 - returns).max(0.0);
            let comfort = 0.3 * ((security.min(land) - 1.0).clamp(0.0, 0.5));
            out.push(
                Candidate::new(GoalKind::Reproduce, h.pos)
                    .term(
                        "offspring",
                        d.reproduce_weight * (0.7 + 0.3 * t.sociability),
                    )
                    .term("comfort", d.reproduce_weight * comfort)
                    .term("shortfall", -d.reproduce_weight * shortfall)
                    .term("travel", -tc * tt_home)
                    .eta(tt_home),
            );
        }
    }

    // --- Idle ---------------------------------------------------------------
    out.push(Candidate::new(GoalKind::Idle, agent.pos));
    // A place the agent recently failed to route to (a flooded home, say) is
    // not retried until its cooldown ends.
    out.retain(|c| c.target == agent.pos || !agent.memory.is_failed_target(c.target, cx.tick));

    // --- Hysteresis and tie-breaking jitter ---------------------------------
    for cand in out.iter_mut() {
        let continuing = if cand.kind == GoalKind::Rest {
            agent.goal.kind == GoalKind::Rest && agent.goal.target == cand.target
        } else {
            agent.goal.same_intent(cand.kind, cand.target, cand.join)
        };
        if continuing && cand.kind != GoalKind::Idle {
            let bonus = d.commitment * (1.0 + t.diligence);
            cand.score += bonus;
            if cand.terms.len() < MAX_TERMS {
                cand.terms.push(Term {
                    name: "commitment",
                    value: bonus,
                });
            }
        }
        cand.score += jitter(agent, cx.tick, cand.kind, cand.target, d.jitter);
    }
    sort_candidates(&mut out);
    // Keep the trace readable: at most a few options of each kind (after
    // hysteresis, so the current goal is never dropped before it is compared).
    let mut per_kind = [0u8; GoalKind::COUNT];
    out.retain(|c| {
        let k = &mut per_kind[c.kind as usize];
        *k += 1;
        *k <= 3
    });
    out
}

/// Exact scores for the best few of many similar options, without computing
/// every exact travel estimate. Each option `(target, base, i)` scores
/// `base - tc * travel_coef * travel - (non-negative extras)`; travel is at
/// least the octile distance at the cheapest movement cost, which gives an
/// upper bound used to visit options best-first and stop once no remaining
/// option can make the top `SHORTLIST`. The current goal's target is always
/// scored exactly (it receives a commitment bonus later). `exact(i)` builds
/// the full candidate for option `i`.
const SHORTLIST: usize = 4;

fn shortlist(
    agent: &Agent,
    cx: &Ctx,
    prelim: &[(Pos, f32, usize)],
    kind: GoalKind,
    travel_coef: f32,
    exact: impl Fn(usize) -> Candidate,
) -> Vec<Candidate> {
    let tc = cx.config.decision.time_cost * travel_coef;
    let min_tick_per_tenth =
        cx.config.movement.min_mp as f32 / cx.config.movement.mp_per_tick as f32 / 10.0;
    let mut bounded: Vec<(f32, Pos, usize)> = prelim
        .iter()
        .map(|&(p, base, i)| {
            let lower_travel = agent.pos.octile10(p) as f32 * min_tick_per_tenth;
            (base - tc * lower_travel, p, i)
        })
        .collect();
    bounded.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut out: Vec<Candidate> = Vec::new();
    // Margin for the jitter added after scoring, so near-ties are not lost.
    let margin = cx.config.decision.jitter;
    // Best SHORTLIST exact scores so far, descending.
    let mut top = [f32::NEG_INFINITY; SHORTLIST];
    for &(bound, p, i) in &bounded {
        let is_current = agent.goal.kind == kind && agent.goal.target == p;
        if bound + margin < top[SHORTLIST - 1] && !is_current {
            continue;
        }
        let cand = exact(i);
        let sc = cand.score;
        if let Some(k) = top.iter().position(|&t| sc > t) {
            top[k..].rotate_right(1);
            top[k] = sc;
        }
        out.push(cand);
    }
    out
}

fn sort_candidates(v: &mut [Candidate]) {
    v.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(a.kind.cmp(&b.kind))
            .then(a.target.cmp(&b.target))
    });
}

/// The agent would like (more) storage at home.
pub fn wants_storage(agent: &Agent, home: Option<&Home>, cx: &Ctx) -> bool {
    let c = &cx.config.inventory;
    if agent.is_juvenile(cx.rates) {
        return false;
    }
    match home {
        None => false,
        Some(h) => {
            let n = h.chests.len() as u32;
            if n >= c.max_chests {
                return false;
            }
            if n == 0 {
                return true;
            }
            let used: f32 = h.chests.iter().map(|ch| ch.weight(c)).sum();
            used >= 0.75 * c.chest_capacity * n as f32
        }
    }
}

/// Unit direction vectors (×100) for exploration rays, 16 compass points.
const RAYS: [(i32, i32); 16] = [
    (100, 0),
    (92, 38),
    (71, 71),
    (38, 92),
    (0, 100),
    (-38, 92),
    (-71, 71),
    (-92, 38),
    (-100, 0),
    (-92, -38),
    (-71, -71),
    (-38, -92),
    (0, -100),
    (38, -92),
    (71, -71),
    (92, -38),
];

/// Frontier tiles found by casting rays from the agent through known land
/// until the first unknown tile. Returns `(frontier, novelty)` where novelty
/// is the unknown fraction of the 5×5 block around the frontier.
pub fn frontier_targets(agent: &Agent, cx: &Ctx) -> Vec<(Pos, f32)> {
    let mut out: Vec<(Pos, f32)> = Vec::new();
    // Persist with the current exploration target while it is still a frontier.
    if agent.goal.kind == GoalKind::Explore && agent.goal.target != agent.pos {
        let f = agent.goal.target;
        if let Some(n) = frontier_novelty(agent, cx, f) {
            out.push((f, n));
        }
    }
    let cached = agent
        .frontier_cache
        .as_ref()
        .filter(|c| c.pos == agent.pos && c.known == agent.memory.known_count());
    let rays = match cached {
        Some(c) => c.targets.clone(),
        None => frontier_rays(agent, cx),
    };
    for (f, n) in rays {
        if !out.iter().any(|(q, _)| *q == f) {
            out.push((f, n));
        }
    }
    out
}

/// Compute the agent's exploration-ray cache for its current position and knowledge.
pub fn compute_frontier_cache(agent: &Agent, cx: &Ctx) -> crate::agents::agent::FrontierCache {
    crate::agents::agent::FrontierCache {
        pos: agent.pos,
        known: agent.memory.known_count(),
        targets: frontier_rays(agent, cx),
    }
}

/// Cast rays from the agent through known land; see [`frontier_targets`].
fn frontier_rays(agent: &Agent, cx: &Ctx) -> Vec<(Pos, f32)> {
    let world = cx.world;
    let max = cx.config.decision.explore_max_dist as i32;
    let n_rays = (cx.config.decision.explore_rays as usize).clamp(1, RAYS.len());
    let step = RAYS.len() / n_rays;
    let mut out: Vec<(Pos, f32)> = Vec::new();
    for k in 0..n_rays {
        let (dx, dy) = RAYS[(k * step + (agent.id as usize % step.max(1))) % RAYS.len()];
        let end = agent.pos.offset(dx * max / 100, dy * max / 100);
        let mut last_known: Option<Pos> = None;
        let mut found: Option<Pos> = None;
        crate::geom::bresenham(agent.pos, end, |p| {
            if !world.contains(p) {
                return false;
            }
            if !agent.memory.knows(p) {
                found = last_known;
                return false;
            }
            if !world.tile(p).is_land() {
                return false;
            }
            last_known = Some(p);
            true
        });
        if let Some(f) = found {
            if f == agent.pos || out.iter().any(|(q, _)| *q == f) {
                continue;
            }
            if let Some(n) = frontier_novelty(agent, cx, f) {
                out.push((f, n));
            }
        }
    }
    out
}

/// If `f` is a known passable tile next to unknown ground, the unknown
/// fraction of the 5x5 block around it; otherwise `None`.
fn frontier_novelty(agent: &Agent, cx: &Ctx, f: Pos) -> Option<f32> {
    let world = cx.world;
    if !world.contains(f) || !agent.memory.knows(f) || !world.tile(f).is_land() {
        return None;
    }
    let frontier = world.dims.neighbors8(f).any(|q| !agent.memory.knows(q));
    if !frontier {
        return None;
    }
    let mut unknown = 0;
    let mut total = 0;
    for oy in -2..=2 {
        for ox in -2..=2 {
            let q = f.offset(ox, oy);
            if world.contains(q) {
                total += 1;
                if !agent.memory.knows(q) {
                    unknown += 1;
                }
            }
        }
    }
    Some(unknown as f32 / total.max(1) as f32)
}

// ---------------------------------------------------------------------------
// Where to live
// ---------------------------------------------------------------------------

/// What an agent knows about a home it might live in: exact for its own home
/// (household knowledge), as last seen for others.
#[derive(Clone, Copy, Debug)]
pub struct JoinInfo {
    pub id: HomeId,
    pub residents: u32,
    pub has_storage: bool,
    pub comfortable: u32,
    pub own: bool,
    pub crowded: bool,
}

impl JoinInfo {
    pub fn own(h: &Home, cx: &Ctx) -> Self {
        Self {
            id: h.id,
            residents: h.residents.len() as u32,
            has_storage: !h.chests.is_empty(),
            comfortable: h.comfortable_capacity(cx.config.homes.comfortable_capacity),
            own: true,
            crowded: is_crowded(h, cx),
        }
    }

    pub fn remembered(m: &crate::agents::memory::HomeMemory) -> Self {
        Self {
            id: m.home,
            residents: m.residents,
            has_storage: m.has_storage,
            // As last seen: shelter levels are visible, but may have changed since.
            comfortable: m.comfortable,
            own: false,
            crowded: false,
        }
    }
}

/// How well placed `site` is for trading (A.33): remembered places, each by
/// its draw, fading linearly to nothing at `access_radius`; capped at 1.
fn market_access(agent: &Agent, c: &Config, site: Pos) -> f32 {
    let m = &c.markets;
    let r = m.access_radius.max(1) as f32;
    agent
        .memory
        .trade_spots
        .iter()
        .map(|s| {
            let near = (1.0 - s.pos.chebyshev(site) as f32 / r).max(0.0);
            near * s.score / (s.score + m.half_score)
        })
        .sum::<f32>()
        .min(1.0)
}

/// Score a potential home site from the agent's own knowledge.
///
/// - **Food:** perceived sustainable food (daily regrowth potential of known
///   fertile tiles within `site_radius`, distance-weighted) per person who
///   would share it (self + residents of known homes nearby).
/// - **River:** small bonus within 2 tiles of a river.
/// - **Neighbours:** residents of known homes within 3 tiles, weighted by
///   sociability.
/// - **Crowding:** residents beyond comfortable capacity, tolerated more by
///   sociable agents; a home crowded beyond the grace period costs extra.
/// - **Storage:** a home with chests is worth more.
/// - **Terrain:** hills and mountains are poor places to live.
/// - **Markets:** access to remembered trading places when enabled.
pub fn site_score(agent: &Agent, cx: &Ctx, site: Pos, join: Option<JoinInfo>) -> Option<f32> {
    let world = cx.world;
    let c = cx.config;
    let d = &c.decision;
    if !world.contains(site) || !agent.memory.knows(site) {
        return None;
    }
    let tile = world.tile(site);
    if !tile.is_land() || tile.is_river {
        return None;
    }
    let joining_own = join.is_some_and(|j| j.own);
    // A tile the agent believes is full (as last seen). If it filled up since,
    // `EstablishHome` finds out on arrival and the agent re-plans.
    if join.is_none() && agent.memory.believed_homes_on(site) >= c.homes.max_homes_per_tile {
        return None;
    }

    let ratio = land_food_ratio(agent, cx, site, 0.0);
    let neighbours = known_people_within(agent, cx, site, 3);
    // Satisficing: land that can feed everyone is what matters; abundance
    // beyond that adds only a little.
    let mut score = ratio.min(1.0) + 0.15 * (ratio - 1.0).clamp(0.0, 1.0);

    if tile.river_distance <= 2 {
        score += 0.1;
    }
    score += d.social_weight * agent.traits.sociability * (neighbours / 8.0).min(1.0);
    // Market access (A.33): good trading places within reach.
    if c.markets.enabled {
        score += c.markets.home_weight * market_access(agent, c, site);
    }
    // Kin ties: living near the home one grew up in.
    if let Some((_, birth_pos)) = agent.birth_home {
        if birth_pos.chebyshev(site) <= 3 {
            score += d.kin_weight * (0.5 + 0.5 * agent.traits.sociability);
        }
    }
    match tile.biome {
        Biome::Hills => score -= 0.05,
        Biome::Mountains => score -= 0.25,
        _ => {}
    }
    if let Some(j) = join {
        let after = j.residents as f32 + if joining_own { 0.0 } else { 1.0 };
        let excess = (after - j.comfortable as f32).max(0.0);
        score -= d.crowding_weight * excess * (1.2 - agent.traits.sociability) * 0.5;
        if j.crowded {
            score -= d.crowding_weight * 0.5 * (1.2 - agent.traits.sociability);
        }
        if j.has_storage {
            score += 0.15;
        }
    }
    Some(score)
}

/// Perceived daily food potential of the known land within `site_radius` of
/// `site` (regrowth capacity of known fertile tiles, distance-weighted).
pub fn land_potential(agent: &Agent, cx: &Ctx, site: Pos) -> f32 {
    let world = cx.world;
    let c = cx.config;
    let r = c.homes.site_radius as i32;
    let mut potential = 0.0f32;
    for dy in -r..=r {
        for dx in -r..=r {
            let q = site.offset(dx, dy);
            if !world.contains(q) || !agent.memory.knows(q) {
                continue;
            }
            let qt = world.tile(q);
            if qt.food_capacity <= 0.0 {
                continue;
            }
            let dist = dx.abs().max(dy.abs());
            let w = 1.0 - dist as f32 / (r + 1) as f32;
            potential += c.resources.food_regrowth * qt.fertility * qt.food_capacity * w;
        }
    }
    potential
}

/// People (other than the agent) the agent believes live within `radius` of
/// `site`: residents of remembered homes, and its own household exactly.
pub fn known_people_within(agent: &Agent, cx: &Ctx, site: Pos, radius: i32) -> f32 {
    let mut people = 0.0f32;
    for hm in &agent.memory.homes {
        if Some(hm.home) == agent.home {
            continue;
        }
        if hm.pos.chebyshev(site) <= radius {
            people += hm.residents as f32;
        }
    }
    if let Some(o) = cx.home(agent) {
        if o.pos.chebyshev(site) <= radius {
            people += o.residents.iter().filter(|&&id| id != agent.id).count() as f32;
        }
    }
    people
}

/// Perceived food potential per person who would share the land around
/// `site` (the agent, the people it knows nearby, and `extra` newcomers),
/// relative to `site_food_per_person`. 1.0 = "just enough".
pub fn land_food_ratio(agent: &Agent, cx: &Ctx, site: Pos, extra: f32) -> f32 {
    let r = cx.config.homes.site_radius as i32;
    let people = 1.0 + extra + known_people_within(agent, cx, site, r);
    land_potential(agent, cx, site) / (people * cx.config.homes.site_food_per_person)
}

/// Re-evaluate where the agent would prefer to live. Candidate sites: the
/// current tile, the current home, known homes with room, and the richest
/// remembered food places. Each is scored by `site_score` minus the cost of
/// getting there.
pub fn plan_home(agent: &Agent, cx: &Ctx) -> Option<HomePlan> {
    let world = cx.world;
    let c = cx.config;
    let tc = c.decision.time_cost;
    let own = cx.home(agent);
    let current = own.and_then(|h| site_score(agent, cx, h.pos, Some(JoinInfo::own(h, cx))));

    let mut best: Option<(f32, Pos, Option<HomeId>, f32)> = None;
    let consider =
        |pos: Pos, join: Option<JoinInfo>, best: &mut Option<(f32, Pos, Option<HomeId>, f32)>| {
            if let Some(s) = site_score(agent, cx, pos, join) {
                let tt = travel_ticks(agent, cx, agent.pos, pos);
                let total = s - tc * tt * 0.5;
                let better = match best {
                    None => true,
                    Some((bt, bp, bj, _)) => {
                        total > *bt || (total == *bt && (pos, join.map(|h| h.id)) < (*bp, *bj))
                    }
                };
                if better {
                    *best = Some((total, pos, join.map(|h| h.id), s));
                }
            }
        };

    if own.is_none() {
        consider(agent.pos, None, &mut best);
    }
    for hm in &agent.memory.homes {
        if Some(hm.home) == agent.home {
            continue;
        }
        let info = JoinInfo::remembered(hm);
        // Join only homes the agent remembers as having room.
        if info.residents < info.comfortable {
            consider(hm.pos, Some(info), &mut best);
        }
    }
    let mut foods: Vec<_> = agent
        .memory
        .resources
        .iter()
        .filter(|m| m.resource == Resource::Food)
        .collect();
    foods.sort_by(|a, b| b.amount.total_cmp(&a.amount).then(a.pos.cmp(&b.pos)));
    for m in foods.into_iter().take(6) {
        consider(m.pos, None, &mut best);
    }
    if let Some(h) = own {
        // Moving out of a crowded home: a fresh site on the same tile or next to it.
        if is_crowded(h, cx) {
            for q in world.dims.neighbors8(h.pos) {
                consider(q, None, &mut best);
            }
        }
    }

    let home_land = land_food_ratio(agent, cx, own.map_or(agent.pos, |h| h.pos), 0.0);
    best.map(|(_, target, join, score)| HomePlan {
        target,
        join,
        score,
        current,
        home_land,
        evaluated_tick: cx.tick,
    })
}

/// Stable hash helper used by the engine to stagger periodic work per agent.
pub fn phase(agent_id: u32, period: u32) -> u64 {
    mix64(u64::from(agent_id)) % u64::from(period.max(1))
}

#[allow(dead_code)]
const _ASSERT_RADIUS: () = assert!(SIGHT_RADIUS == 6);
