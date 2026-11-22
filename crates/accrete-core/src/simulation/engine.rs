//! The simulation controller: a fixed-tick pipeline.
//!
//! Each tick (1 hour):
//!
//! 1. **Observe** — agents that moved, or have stood still for
//!    `reobserve_ticks`, read their line-of-sight mask and update memory.
//! 2. **Plan homes** — periodically (staggered per agent), re-evaluate where
//!    to live.
//! 3. **Propose** — every agent scores its options against a world that is
//!    read-only during this phase, picks a goal, plans a route, and emits one
//!    *intent* for the tick. Order does not matter: no agent sees another's
//!    choice.
//! 4. **Resolve conflicts** — gathering on a shared tile is split
//!    proportionally to each request (no ID bias, no duplication). Chest
//!    operations, home founding/joining, and births are resolved in a fresh
//!    random permutation from the `conflicts` stream, and births roll on the
//!    `births` stream in ascending parent-ID order.
//! 5. **Apply** — movement (integer MP), eating, resting, needs, aging,
//!    frailty, and deaths; then meetings (sharing and barter) among the
//!    survivors.
//! 6. **Daily** (last tick of each day) — regrowth, spoilage, skill decay,
//!    crowding, traffic decay, settlement detection, metrics.
//! 7. Newborns join the agent list (they act from the next tick).

use std::collections::BTreeMap;

use crate::agents::agent::{
    Agent, AgentStats, Goal, GoalKind, PathologyState, Route, Skill, Traits,
};
use crate::agents::decision::{self, Candidate, Ctx, Trace, reproduce_blockers};
use crate::agents::memory::{AgentMemory, HomeSighting};
use crate::agents::pathfinding::{AgentBelief, Omniscient, PathScratch};
use crate::config::{AgentKind, Config, TICKS_PER_DAY, TickRates};
use crate::geom::{Pos, is_diagonal};
use crate::hash::{StableHash, StableHasher};
use crate::inventory::{Inventory, Resource};
use crate::rng::{Rng, Stream};
use crate::simulation::disruption::DisruptionState;
use crate::simulation::emergence;
use crate::simulation::events::{
    DeathCause, DeathRecord, EventKind, EventLog, Ledger, PathologyKind,
};
use crate::simulation::metrics::{DailyMetrics, DayCounters};
use crate::simulation::settlement::{SettlementTracker, SizeClass};
use crate::simulation::weather::WeatherState;
use crate::world::movement::step_cost;
use crate::world::{Home, HomeId, Pile, World, generate_world, resources};

/// What an agent does this tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Intent {
    Idle,
    Move,
    Gather(Resource),
    Eat,
    Rest,
    Establish { pos: Pos, join: Option<HomeId> },
    BuildStorage,
    Deposit,
    Withdraw,
    Reproduce,
    ImproveTrail,
    ImproveShelter,
    Craft,
}

pub struct Simulation {
    pub config: Config,
    pub rates: TickRates,
    pub tick: u64,
    pub world: World,
    pub agents: Vec<Agent>,
    pub next_id: u32,
    pub rng_agents: Rng,
    pub rng_births: Rng,
    pub rng_conflicts: Rng,
    pub rng_weather: Rng,
    pub ledger: Ledger,
    pub events: EventLog,
    pub deaths: Vec<DeathRecord>,
    /// Recently dead agents, kept for inspection (bounded).
    pub graveyard: Vec<Agent>,
    pub metrics: Vec<DailyMetrics>,
    pub pathologies: [u64; 4],
    /// Every flagged pathology event (bounded), for inspection.
    pub pathology_log: Vec<(u64, u32, PathologyKind)>,
    pub settlements: SettlementTracker,
    pub today: DayCounters,
    /// Barters of the last 30 days, for trade-network measurement only.
    /// Agents never read it, and it is not part of the state hash.
    pub trade_log: Vec<crate::simulation::emergence::TradeRecord>,
    pub hit_cap: bool,
    /// Latest decision trace per agent (debugging; not part of the state hash).
    pub traces: BTreeMap<u32, Trace>,
    /// Latest intent per agent as `(agent id, intent)`, sorted by id
    /// (debugging; use `intent_of`).
    pub intents: Vec<(u32, Intent)>,
    pub weather: WeatherState,
    /// Route disruptions in progress and past (authoritative; the world keeps
    /// a derived per-tile index).
    pub disruptions: DisruptionState,
    pub(crate) scratch: PathScratch,
    pub(crate) total_births: u32,
    pub(crate) total_deaths: [u32; 2],
    pub(crate) lifespans: (f64, u32),
}

const GRAVEYARD_CAP: usize = 500;

impl Simulation {
    /// Generate a world from `seed` and place the initial agents.
    pub fn new(seed: u64, config: Config) -> Self {
        let world = generate_world(seed, &config);
        Self::from_world(world, config)
    }

    /// Start a simulation on an existing world (e.g. a hand-built scenario),
    /// placing one initial agent on each of `world.spawns`.
    pub fn from_world(world: World, config: Config) -> Self {
        let seed = world.seed;
        let mut sim = Self::from_world_parts(world, config, Vec::new());
        let mut founders = Rng::for_stream(seed, Stream::Founders);
        let spawns = sim.world.spawns.clone();
        for p in spawns {
            let a = sim.make_founder(p, &mut founders);
            sim.agents.push(a);
        }
        sim.reset_ledger();
        for i in 0..sim.agents.len() {
            observe(&mut sim.agents[i], &sim.world, &sim.config, &sim.rates, 0);
        }
        sim
    }

    /// A simulation at tick 0 with the given world and agents, fresh RNG
    /// streams for the world's seed, and empty bookkeeping. Callers fill in
    /// the rest (`from_world` adds founders; snapshots restore everything).
    pub(crate) fn from_world_parts(world: World, config: Config, agents: Vec<Agent>) -> Self {
        let rates = TickRates::new(&config);
        let seed = world.seed;
        Self {
            rates,
            tick: 0,
            agents,
            next_id: 0,
            rng_agents: Rng::for_stream(seed, Stream::Agents),
            rng_births: Rng::for_stream(seed, Stream::Births),
            rng_conflicts: Rng::for_stream(seed, Stream::Conflicts),
            rng_weather: Rng::for_stream(seed, Stream::Weather),
            ledger: Ledger::default(),
            events: EventLog::new(config.sim.event_log_capacity),
            deaths: Vec::new(),
            graveyard: Vec::new(),
            metrics: Vec::new(),
            pathologies: [0; 4],
            pathology_log: Vec::new(),
            settlements: SettlementTracker::default(),
            today: DayCounters::default(),
            trade_log: Vec::new(),
            hit_cap: false,
            traces: BTreeMap::new(),
            intents: Vec::new(),
            scratch: PathScratch::default(),
            weather: WeatherState::default(),
            disruptions: DisruptionState::default(),
            total_births: 0,
            total_deaths: [0; 2],
            lifespans: (0.0, 0),
            world,
            config,
        }
    }

    /// Start resource accounting afresh from the current state.
    pub fn reset_ledger(&mut self) {
        self.ledger = Ledger::default();
        let totals = self.world.resource_totals();
        let carried = self.carried_totals();
        for r in Resource::GOODS {
            self.ledger.initial[r.index()] = f64::from(totals.get(r)) + carried[r.index()];
        }
    }

    fn make_founder(&mut self, pos: Pos, rng: &mut Rng) -> Agent {
        let c = &self.config;
        let id = self.next_id;
        self.next_id += 1;
        let traits = Traits {
            curiosity: rng.unit_f32(),
            caution: rng.unit_f32(),
            sociability: rng.unit_f32(),
            diligence: rng.unit_f32(),
        };
        let mut skills = [Skill {
            aptitude: 1.0,
            practice: 0.0,
        }; 3];
        for s in skills.iter_mut() {
            s.aptitude = rng.range_f32(c.skills.aptitude_min, c.skills.aptitude_max);
        }
        let age_ticks = u64::from(rng.range_u32_inclusive(
            self.rates.maturity_ticks as u32,
            self.rates.elder_ticks as u32,
        ));
        let hunger = rng.range_f32(c.spawn.initial_hunger_min, c.spawn.initial_hunger_max);
        let energy = rng.range_f32(0.7, 1.0);
        let mut memory = AgentMemory::new(self.world.dims);
        let r = c.spawn.initial_known_radius as i32;
        for dy in -r..=r {
            for dx in -r..=r {
                let q = pos.offset(dx, dy);
                if self.world.contains(q) && dx * dx + dy * dy <= r * r {
                    memory.learn(q);
                }
            }
        }
        Agent {
            id,
            pos,
            hunger,
            energy,
            health: 1.0,
            frailty: 0.0,
            age_ticks,
            born_tick: 0,
            last_birth_tick: None,
            next_reproduce_try: 0,
            parent: None,
            inventory: Inventory {
                food: c.spawn.initial_food,
                ..Inventory::EMPTY
            },
            movement_points: 0,
            memory,
            traits,
            skills,
            goal: Goal::idle(pos, 0),
            route: None,
            home: None,
            home_since: 0,
            birth_home: None,
            home_plan: None,
            last_observe_tick: 0,
            moved: true,
            frailty_crowding: 0.0,
            stats: AgentStats::default(),
            pathology: PathologyState::default(),
            practiced_today: 0,
            frontier_cache: None,
            line_memo: Default::default(),
            craft: Skill {
                aptitude: craft_aptitude(self.world.seed, id, c),
                practice: 0.0,
            },
            forage_rate: c.skills.base_food,
            forage_today_food: 0.0,
            forage_today_ticks: 0,
            last_share_tick: 0,
        }
    }

    pub fn day(&self) -> u32 {
        (self.tick / TICKS_PER_DAY) as u32
    }

    pub fn population(&self) -> usize {
        self.agents.len()
    }

    pub fn agent(&self, id: u32) -> Option<&Agent> {
        self.agents
            .binary_search_by_key(&id, |a| a.id)
            .ok()
            .map(|i| &self.agents[i])
    }

    fn carried_totals(&self) -> [f64; 4] {
        let mut t = [0.0f64; 4];
        for a in &self.agents {
            for r in Resource::GOODS {
                t[r.index()] += f64::from(a.inventory.get(r));
            }
        }
        t
    }

    /// Run `n` ticks.
    pub fn run(&mut self, n: u64) {
        for _ in 0..n {
            self.step();
        }
    }

    /// Advance one tick.
    pub fn step(&mut self) {
        let tick = self.tick;
        let n = self.agents.len();

        // 1. Observe.
        for i in 0..n {
            let a = &mut self.agents[i];
            if a.moved || tick >= a.last_observe_tick + u64::from(self.config.sight.reobserve_ticks)
            {
                observe(a, &self.world, &self.config, &self.rates, tick);
            }
            a.memory.expire(tick);
        }

        // 2. Plan homes (staggered).
        let reconsider = self.config.homes.reconsider_ticks.max(1);
        for i in 0..n {
            let a = &self.agents[i];
            if a.is_juvenile(&self.rates) {
                continue;
            }
            let phase = decision::phase(a.id, reconsider);
            let due = (tick + phase) % u64::from(reconsider) == 0;
            let homeless_unplanned =
                a.home.is_none() && a.home_plan.is_none() && (tick + phase) % 6 == 0;
            if due || homeless_unplanned {
                let cx = self.ctx(tick);
                let plan = decision::plan_home(a, &cx);
                self.agents[i].home_plan = plan;
            }
        }

        // Births blocked by the safety cap are logged (once per agent per day).
        if self.agents.len() as u32 >= self.config.life.population_cap && tick % TICKS_PER_DAY == 0
        {
            let blocked: Vec<(u32, Pos)> = {
                let cx = self.ctx(tick);
                self.agents
                    .iter()
                    .filter(|a| reproduce_blockers(a, &cx) == ["population cap"])
                    .map(|a| (a.id, a.pos))
                    .collect()
            };
            for (id, pos) in blocked {
                self.today.blocked_births += 1;
                self.events.push(
                    tick,
                    Some(id),
                    pos,
                    EventKind::BirthBlockedByCap { parent: id },
                );
            }
        }

        // 3. Propose.
        let mut intents = vec![Intent::Idle; n];
        #[allow(clippy::needless_range_loop)] // `propose` needs `&mut self`
        for i in 0..n {
            intents[i] = self.propose(i, tick);
            let k = self.agents[i].goal.kind as usize;
            self.today.goal_ticks[k] += 1;
        }

        // 4. Resolve contested actions.
        self.resolve_gathering(&intents, tick);
        let (newborns, birth_blocked_by_cap) = self.resolve_household(&intents, tick);

        // 5. Apply individual actions and needs.
        for (i, &intent) in intents.iter().enumerate() {
            self.apply(i, intent, tick);
        }
        // Keep this tick's intents by agent ID: deaths below shift indices.
        let intents_by_id: Vec<(u32, Intent)> = self
            .agents
            .iter()
            .map(|a| a.id)
            .zip(intents.iter().copied())
            .collect();
        // Agents who died this tick are removed before meeting anyone, so the
        // dead never trade or share knowledge.
        self.process_deaths(tick);
        // Meetings: knowledge sharing and barter among co-located agents.
        self.meetings(tick);

        // 7. Newborns join (before daily metrics so they are counted).
        for b in newborns {
            self.agents.push(b);
        }
        let at_cap = self.agents.len() as u32 >= self.config.life.population_cap;
        if (at_cap || birth_blocked_by_cap) && !self.hit_cap {
            // Reaching the safety cap flags the run: it usually means mistuning.
            self.hit_cap = true;
            self.events
                .push(tick, None, Pos::default(), EventKind::PopulationCapReached);
        }
        debug_assert!(self.agents.windows(2).all(|w| w[0].id < w[1].id));

        // 6. Daily.
        if (tick + 1) % TICKS_PER_DAY == 0 {
            self.daily(tick);
        }
        if self.config.sim.check_invariants {
            if let Err(e) = self.check_invariants() {
                panic!("invariant violated at tick {tick}: {e}");
            }
        }
        self.intents = intents_by_id;
        self.tick += 1;
    }

    /// The intent agent `id` acted on in the latest tick (debugging).
    pub fn intent_of(&self, id: u32) -> Option<Intent> {
        self.intents
            .binary_search_by_key(&id, |&(aid, _)| aid)
            .ok()
            .map(|k| self.intents[k].1)
    }

    pub(crate) fn ctx(&self, tick: u64) -> Ctx<'_> {
        Ctx {
            world: &self.world,
            config: &self.config,
            rates: &self.rates,
            tick,
            population_at_cap: self.agents.len() as u32 >= self.config.life.population_cap,
            agents: &self.agents,
        }
    }

    // -----------------------------------------------------------------------
    // Propose
    // -----------------------------------------------------------------------

    fn propose(&mut self, i: usize, tick: u64) -> Intent {
        // Fast path: an agent resting in place keeps resting between periodic
        // re-checks, unless a need crosses its threshold. This changes only how
        // often a resting agent re-decides, not what it would decide.
        {
            let a = &self.agents[i];
            if a.goal.kind == GoalKind::Rest && a.pos == a.goal.target {
                let cx = self.ctx(tick);
                let hungry = a.hunger >= decision::eat_threshold(a, &cx);
                let tired = a.energy < self.config.decision.rest_until;
                let period = if tired { 4 } else { 2 };
                if !hungry && (tick - a.goal.since) % period != 0 {
                    return Intent::Rest;
                }
            }
        }
        // Exploration rays only change when the agent moves or learns tiles.
        let fresh_rays = {
            let a = &self.agents[i];
            let known = a.memory.known_count();
            let stale = !a
                .frontier_cache
                .as_ref()
                .is_some_and(|c| c.pos == a.pos && c.known == known);
            stale.then(|| decision::compute_frontier_cache(a, &self.ctx(tick)))
        };
        if let Some(fc) = fresh_rays {
            self.agents[i].frontier_cache = Some(fc);
        }
        let cands = {
            let cx = self.ctx(tick);
            let a = &self.agents[i];
            if self.config.sim.agent_kind == AgentKind::Random {
                random_choice(decision::evaluate(a, &cx), &mut self.rng_agents)
            } else {
                decision::evaluate(a, &cx)
            }
        };
        let mut chosen: Option<Candidate> = None;
        for cand in cands.iter().take(6) {
            if cand.target == self.agents[i].pos || self.ensure_route(i, cand.target, tick) {
                chosen = Some(cand.clone());
                break;
            }
        }
        let chosen = chosen.unwrap_or_else(|| {
            cands
                .iter()
                .find(|c| c.kind == GoalKind::Idle)
                .cloned()
                .expect("idle is always available")
        });

        let agent = &mut self.agents[i];
        if !agent
            .goal
            .same_intent(chosen.kind, chosen.target, chosen.join)
        {
            // A switch is an *abandonment* only when the previous goal was
            // unfinished and still on offer: the agent changed its mind rather
            // than completing (or losing) the goal.
            let prev = agent.goal;
            let still_offered = cands.iter().any(|c| {
                prev.same_intent(c.kind, c.target, c.join)
                    || (prev.kind == GoalKind::Rest
                        && c.kind == GoalKind::Rest
                        && c.target == prev.target)
            });
            let abandoned =
                prev.target != agent.pos && prev.kind != GoalKind::Idle && still_offered;
            agent.stats.goal_switches += 1;
            if abandoned {
                let ps = &mut agent.pathology;
                ps.switches.push(tick);
                // Oscillating = *more than* `oscillation_switches` abandonments
                // within the window (accrete.md, pathology detectors), so keep
                // one more than the limit and flag when all of them fit.
                let limit = self.config.pathology.oscillation_switches as usize + 1;
                if ps.switches.len() > limit {
                    ps.switches.remove(0);
                }
                if ps.switches.len() == limit
                    && tick - ps.switches[0]
                        < u64::from(self.config.pathology.oscillation_window_ticks)
                {
                    ps.switches.clear();
                    self.pathologies[0] += 1;
                    log_pathology(
                        &mut self.pathology_log,
                        tick,
                        agent.id,
                        PathologyKind::Oscillating,
                    );
                    self.events.push(
                        tick,
                        Some(agent.id),
                        agent.pos,
                        EventKind::Pathology {
                            kind: PathologyKind::Oscillating,
                        },
                    );
                }
            }
            agent.goal = Goal {
                kind: chosen.kind,
                target: chosen.target,
                join: chosen.join,
                since: tick,
                work: 0,
            };
        }
        if chosen.target == agent.pos {
            agent.route = None;
        }
        self.traces.insert(
            agent.id,
            Trace {
                tick,
                options: cands.into_iter().take(8).collect(),
                chosen: Some(chosen.clone()),
            },
        );

        if agent.pos != agent.goal.target {
            return Intent::Move;
        }
        match agent.goal.kind {
            GoalKind::Idle | GoalKind::Explore => Intent::Idle,
            GoalKind::GatherFood => Intent::Gather(Resource::Food),
            GoalKind::GatherWood => Intent::Gather(Resource::Wood),
            GoalKind::GatherStone => Intent::Gather(Resource::Stone),
            GoalKind::Eat => Intent::Eat,
            GoalKind::Rest => Intent::Rest,
            GoalKind::EstablishHome => Intent::Establish {
                pos: agent.goal.target,
                join: agent.goal.join,
            },
            GoalKind::BuildStorage => Intent::BuildStorage,
            GoalKind::Deposit => Intent::Deposit,
            GoalKind::Withdraw => Intent::Withdraw,
            GoalKind::Reproduce => Intent::Reproduce,
            GoalKind::ImproveTrail => Intent::ImproveTrail,
            GoalKind::ImproveShelter => Intent::ImproveShelter,
            GoalKind::Craft => Intent::Craft,
        }
    }

    /// Make sure agent `i` has a usable route to `target`. Returns false (and
    /// marks the target unreachable) when no known route exists.
    fn ensure_route(&mut self, i: usize, target: Pos, tick: u64) -> bool {
        let mc = &self.config.movement;
        let agent = &self.agents[i];
        if let Some(r) = &agent.route {
            let fresh = tick < r.planned_tick + u64::from(mc.replan_ticks);
            let next_ok = r
                .remaining()
                .first()
                .is_some_and(|&p| self.world.step_in_cost(agent.pos, p, mc).is_some());
            // A stale but valid route is re-planned: the agent may know a better way now.
            if r.target == target && !r.done() && next_ok && fresh {
                return true;
            }
        }
        let res = if self.config.sim.agent_kind == AgentKind::Omniscient {
            self.scratch.find_path(
                &self.world,
                &Omniscient,
                agent.pos,
                target,
                mc,
                mc.astar_max_expansions,
            )
        } else {
            self.scratch.find_path(
                &self.world,
                &AgentBelief(&agent.memory),
                agent.pos,
                target,
                mc,
                mc.astar_max_expansions,
            )
        };
        self.today.astar_searches += 1;
        let agent = &mut self.agents[i];
        match res {
            Ok(path) => {
                self.today.astar_expansions += u64::from(path.expansions);
                agent.route = Some(Route {
                    target,
                    steps: path.steps,
                    next: 0,
                    planned_tick: tick,
                    cost: path.cost,
                });
                true
            }
            Err(_) => {
                let until = tick + u64::from(self.config.memory.unreachable_cooldown_ticks);
                agent.memory.mark_unreachable(target, until);
                if agent.home_plan.is_some_and(|p| p.target == target) {
                    agent.home_plan = None;
                }
                self.today.route_failures += 1;
                self.events.push(
                    tick,
                    Some(agent.id),
                    agent.pos,
                    EventKind::RouteFailed { target },
                );
                false
            }
        }
    }

    // -----------------------------------------------------------------------
    // Resolve
    // -----------------------------------------------------------------------

    /// Split each tile's resource among all agents gathering it this tick,
    /// proportionally to their requests.
    fn resolve_gathering(&mut self, intents: &[Intent], tick: u64) {
        let c = &self.config;
        // (pos, resource, agent index, request)
        let mut reqs: Vec<(Pos, Resource, usize, f32)> = Vec::new();
        for (i, intent) in intents.iter().enumerate() {
            if let Intent::Gather(r) = *intent {
                let a = &self.agents[i];
                let room = a
                    .inventory
                    .room_for(r, a.carry_capacity(c, &self.rates), &c.inventory);
                let bonus = if c.tools.enabled && a.inventory.tools > 0.0 {
                    c.tools.gather_bonus
                } else {
                    1.0
                };
                let want = (a.gather_rate(r, c, &self.rates) * bonus).min(room);
                reqs.push((a.pos, r, i, want.max(0.0)));
            }
        }
        reqs.sort_by(|a, b| (a.0, a.1, a.2).cmp(&(b.0, b.1, b.2)));
        let practice_step = c.skills.practice_per_tick;
        let (tools_on, tool_wear) = (c.tools.enabled, 1.0 / c.tools.durability_ticks.max(1e-3));
        let practice_max = c.skills.practice_max;
        let mut k = 0;
        while k < reqs.len() {
            let (pos, res) = (reqs[k].0, reqs[k].1);
            let mut end = k;
            let mut total = 0.0f32;
            while end < reqs.len() && reqs[end].0 == pos && reqs[end].1 == res {
                total += reqs[end].3;
                end += 1;
            }
            let available = self.world.resource_at(pos, res);
            let factor = if total > available && total > 0.0 {
                available / total
            } else {
                1.0
            };
            for &(_, _, i, want) in &reqs[k..end] {
                let got = self.take_from_tile(pos, res, want * factor);
                let a = &mut self.agents[i];
                a.inventory.put(res, got);
                self.ledger.gathered[res.index()] += f64::from(got);
                a.practiced_today |= 1 << res.index();
                let s = &mut a.skills[res.index()];
                s.practice = (s.practice + practice_step).min(practice_max);
                if got > 0.0 && tools_on && a.inventory.tools > 0.0 {
                    // A tool wears with every productive gathering tick.
                    let worn = a.inventory.take(Resource::Tools, tool_wear);
                    self.ledger.construction[Resource::Tools.index()] += f64::from(worn);
                }
                if got > 0.0 {
                    a.pathology.last_progress_tick = tick;
                    if res == Resource::Food {
                        a.stats.food_gathered += got;
                        a.forage_today_food += got;
                        self.today.food_gathered += f64::from(got);
                        if let Some(h) = a.home {
                            let d = self.world.homes[h as usize].pos.chebyshev(pos);
                            self.today.forage_distance_sum += f64::from(d);
                            self.today.forage_distance_n += 1;
                        }
                    }
                    self.events.push(
                        tick,
                        Some(a.id),
                        pos,
                        EventKind::Gathered {
                            resource: res,
                            amount: got,
                        },
                    );
                }
            }
            k = end;
        }
    }

    /// Remove up to `amount` of `r` from the pile and then the tile at `pos`.
    fn take_from_tile(&mut self, pos: Pos, r: Resource, amount: f32) -> f32 {
        let mut got = 0.0;
        if let Some(pi) = self.world.piles.iter().position(|p| p.pos == pos) {
            got += self.world.piles[pi].goods.take(r, amount);
            if self.world.piles[pi].goods.is_empty() {
                self.world.piles.remove(pi);
            }
        }
        let rest = amount - got;
        if rest > 0.0 {
            let t = self.world.tile_mut(pos);
            let slot = match r {
                Resource::Food => &mut t.food,
                Resource::Wood => &mut t.wood,
                Resource::Stone => &mut t.stone,
                Resource::Tools => return got,
            };
            let take = rest.min(*slot);
            *slot -= take;
            if *slot < 1e-6 {
                got += *slot;
                *slot = 0.0;
            }
            got += take;
        }
        got
    }

    /// Chest operations, homes, storage building, and births, in a random
    /// permutation (no ID bias). Returns newborns, ordered by parent ID, and
    /// whether any birth was blocked by the population cap.
    fn resolve_household(&mut self, intents: &[Intent], tick: u64) -> (Vec<Agent>, bool) {
        let mut order: Vec<usize> = intents
            .iter()
            .enumerate()
            .filter(|(_, it)| {
                matches!(
                    it,
                    Intent::Deposit
                        | Intent::Withdraw
                        | Intent::Establish { .. }
                        | Intent::BuildStorage
                        | Intent::ImproveTrail
                        | Intent::ImproveShelter
                        | Intent::Craft
                )
            })
            .map(|(i, _)| i)
            .collect();
        self.rng_conflicts.shuffle(&mut order);
        for i in order {
            match intents[i] {
                Intent::Deposit => self.do_deposit(i, tick),
                Intent::Withdraw => self.do_withdraw(i, tick),
                Intent::Establish { pos, join } => self.do_establish(i, pos, join, tick),
                Intent::BuildStorage => self.do_build_storage(i, tick),
                Intent::ImproveTrail => self.do_improve_trail(i, tick),
                Intent::ImproveShelter => self.do_improve_shelter(i, tick),
                Intent::Craft => self.do_craft(i, tick),
                _ => unreachable!(),
            }
        }

        // Births: ascending parent ID (agents are stored in ID order).
        let mut newborns = Vec::new();
        let mut blocked_by_cap = false;
        for (i, intent) in intents.iter().enumerate() {
            if *intent != Intent::Reproduce {
                continue;
            }
            let blockers = {
                let cx = self.ctx(tick);
                reproduce_blockers(&self.agents[i], &cx)
            };
            let at_home = self.agents[i]
                .home
                .is_some_and(|h| self.world.homes[h as usize].pos == self.agents[i].pos);
            let pop_now = self.agents.len() + newborns.len();
            if pop_now as u32 >= self.config.life.population_cap {
                // `step` flags the run and logs `PopulationCapReached`.
                blocked_by_cap = true;
                self.today.blocked_births += 1;
                let a = &self.agents[i];
                self.events.push(
                    tick,
                    Some(a.id),
                    a.pos,
                    EventKind::BirthBlockedByCap { parent: a.id },
                );
                continue;
            }
            if !blockers.is_empty() || !at_home {
                continue;
            }
            self.agents[i].pathology.last_progress_tick = tick;
            if !self.rng_births.chance(self.config.life.reproduce_chance) {
                let a = &mut self.agents[i];
                a.next_reproduce_try = tick + self.rates.reproduce_retry_ticks;
                self.events
                    .push(tick, Some(a.id), a.pos, EventKind::BirthRollFailed);
                continue;
            }
            let baby = self.make_newborn(i, tick);
            newborns.push(baby);
        }
        (newborns, blocked_by_cap)
    }

    fn do_deposit(&mut self, i: usize, tick: u64) {
        let c = &self.config;
        let cx_reserve = {
            let cx = self.ctx(tick);
            decision::reserve_target(&self.agents[i], &cx)
        };
        let (wood_keep, stone_keep) = {
            let cx = self.ctx(tick);
            let (_, _, wk, sk) = decision::material_plan(&self.agents[i], &cx);
            (wk, sk)
        };
        let a = &mut self.agents[i];
        let Some(hid) = a.home else { return };
        let home = &mut self.world.homes[hid as usize];
        if home.pos != a.pos {
            return;
        }
        let mut moved = Inventory::EMPTY;
        let keep_food = cx_reserve * 1.0;
        let plan = [
            (Resource::Food, (a.inventory.food - keep_food).max(0.0)),
            (Resource::Wood, (a.inventory.wood - wood_keep).max(0.0)),
            (Resource::Stone, (a.inventory.stone - stone_keep).max(0.0)),
        ];
        for (r, mut amount) in plan {
            for chest in home.chests.iter_mut() {
                if amount <= 0.0 {
                    break;
                }
                let m = a.inventory.transfer_to(
                    chest,
                    r,
                    amount,
                    c.inventory.chest_capacity,
                    &c.inventory,
                );
                amount -= m;
                moved.put(r, m);
            }
        }
        for r in Resource::GOODS {
            self.ledger.transferred[r.index()] += f64::from(moved.get(r));
        }
        let stored = home.stored();
        let room: f32 = home
            .chests
            .iter()
            .map(|ch| (c.inventory.chest_capacity - ch.weight(&c.inventory)).max(0.0))
            .sum();
        a.memory.home_food_seen = stored.food;
        a.memory.home_room_seen = room;
        if !moved.is_empty() {
            a.pathology.last_progress_tick = tick;
            self.events.push(
                tick,
                Some(a.id),
                a.pos,
                EventKind::Deposited {
                    home: hid,
                    goods: moved,
                },
            );
        }
        a.goal = Goal::idle(a.pos, tick);
    }

    fn do_withdraw(&mut self, i: usize, tick: u64) {
        let c = &self.config;
        let reserve = {
            let cx = self.ctx(tick);
            decision::reserve_target(&self.agents[i], &cx)
        };
        let a = &mut self.agents[i];
        let Some(hid) = a.home else { return };
        let home = &mut self.world.homes[hid as usize];
        if home.pos != a.pos {
            return;
        }
        let cap = a.carry_capacity(c, &self.rates);
        let mut want = (reserve - a.inventory.food).max(0.0);
        let mut got = 0.0;
        for chest in home.chests.iter_mut() {
            if want <= 0.0 {
                break;
            }
            let m = chest.transfer_to(&mut a.inventory, Resource::Food, want, cap, &c.inventory);
            want -= m;
            got += m;
        }
        self.ledger.transferred[0] += f64::from(got);
        a.memory.home_food_seen = home.stored().food;
        a.memory.home_room_seen = home
            .chests
            .iter()
            .map(|ch| (c.inventory.chest_capacity - ch.weight(&c.inventory)).max(0.0))
            .sum();
        if got > 0.0 {
            a.pathology.last_progress_tick = tick;
            self.events.push(
                tick,
                Some(a.id),
                a.pos,
                EventKind::Withdrew {
                    home: hid,
                    food: got,
                },
            );
        }
        a.goal = Goal::idle(a.pos, tick);
    }

    fn do_establish(&mut self, i: usize, pos: Pos, join: Option<HomeId>, tick: u64) {
        let c = &self.config.clone();
        let aid = self.agents[i].id;
        if self.agents[i].pos != pos {
            return;
        }
        let old = self.agents[i].home;
        let target_home = match join {
            Some(h) if Some(h) != old => {
                let home = &self.world.homes[h as usize];
                let room = (home.residents.len() as u32)
                    < home.comfortable_capacity(c.homes.comfortable_capacity);
                if room && home.pos == pos {
                    Some(h)
                } else if self.founding_here_beats_staying(i, pos, tick) {
                    // The home filled up since the agent planned to join it.
                    // "Their house is full, so I'll build next door": found a new
                    // home on this tile, but only because founding here scores at
                    // least as well as staying put.
                    self.events.push(
                        tick,
                        Some(aid),
                        pos,
                        EventKind::JoinFellBackToFounding { home: h },
                    );
                    None
                } else {
                    // Founding here is worse than staying: give up this plan and
                    // re-plan from what the agent now sees.
                    let a = &mut self.agents[i];
                    a.home_plan = None;
                    a.goal = Goal::idle(a.pos, tick);
                    return;
                }
            }
            _ => None,
        };
        let new_id = match target_home {
            Some(h) => {
                let home = &mut self.world.homes[h as usize];
                insert_sorted(&mut home.residents, aid);
                self.today.home_joins += 1;
                self.events
                    .push(tick, Some(aid), pos, EventKind::JoinedHome { home: h });
                h
            }
            None => {
                if self.world.homes_on(pos).len() as u32 >= c.homes.max_homes_per_tile {
                    // Tile full: give up this plan; it will be re-planned.
                    let a = &mut self.agents[i];
                    a.home_plan = None;
                    a.goal = Goal::idle(a.pos, tick);
                    return;
                }
                let id = self.world.homes.len() as HomeId;
                self.world.homes.push(Home {
                    id,
                    pos,
                    residents: vec![aid],
                    chests: Vec::new(),
                    shelter_level: 0,
                    founded_tick: tick,
                    over_capacity_since: None,
                });
                let idx = self.world.dims.idx(pos) as usize;
                if self.world.homes_at.len() == self.world.dims.len() {
                    self.world.homes_at[idx].push(id);
                }
                self.today.homes_founded += 1;
                self.agents[i].stats.homes_founded += 1;
                self.events
                    .push(tick, Some(aid), pos, EventKind::FoundedHome { home: id });
                id
            }
        };
        if let Some(o) = old {
            let from = self.world.homes[o as usize].pos;
            self.today.home_move_distance_sum += f64::from(from.chebyshev(pos));
            self.leave_home(aid, o, tick);
            self.today.home_moves += 1;
            self.agents[i].stats.home_moves += 1;
        }
        let a = &mut self.agents[i];
        a.home = Some(new_id);
        a.home_since = tick;
        a.home_plan = None;
        a.pathology.last_progress_tick = tick;
        let home = &self.world.homes[new_id as usize];
        a.memory.home_food_seen = home.stored().food;
        a.memory.home_room_seen = home
            .chests
            .iter()
            .map(|ch| (c.inventory.chest_capacity - ch.weight(&c.inventory)).max(0.0))
            .sum();
        a.memory.note_home(
            new_id,
            pos,
            home_sighting(&self.world, new_id, c.homes.comfortable_capacity),
            tick,
            c.memory.home_slots as usize,
        );
        a.goal = Goal::idle(a.pos, tick);
    }

    /// Fallback for a failed join: does founding a new home on `pos` score at
    /// least as well, by the agent's own site scoring, as staying in its
    /// current home? A homeless agent only needs `pos` to be a valid site.
    fn founding_here_beats_staying(&self, i: usize, pos: Pos, tick: u64) -> bool {
        let cx = self.ctx(tick);
        let a = &self.agents[i];
        let Some(found) = decision::site_score(a, &cx, pos, None) else {
            return false;
        };
        match cx.home(a) {
            None => true,
            Some(h) => decision::site_score(a, &cx, h.pos, Some(decision::JoinInfo::own(h, &cx)))
                .is_none_or(|stay| found >= stay),
        }
    }

    fn leave_home(&mut self, aid: u32, h: HomeId, tick: u64) {
        let home = &mut self.world.homes[h as usize];
        home.residents.retain(|&r| r != aid);
        let pos = home.pos;
        self.events
            .push(tick, Some(aid), pos, EventKind::LeftHome { home: h });
        if home.residents.is_empty() {
            home.over_capacity_since = None;
            self.events
                .push(tick, None, pos, EventKind::HomeVacant { home: h });
        }
    }

    fn do_build_storage(&mut self, i: usize, tick: u64) {
        let c = &self.config;
        let a = &mut self.agents[i];
        let Some(hid) = a.home else { return };
        let home = &mut self.world.homes[hid as usize];
        if home.pos != a.pos
            || home.chests.len() as u32 >= c.inventory.max_chests
            || a.inventory.wood < c.inventory.storage_wood
        {
            a.goal = Goal::idle(a.pos, tick);
            return;
        }
        a.goal.work += 1;
        a.pathology.last_progress_tick = tick;
        if a.goal.work >= c.inventory.storage_ticks {
            let used = a.inventory.take(Resource::Wood, c.inventory.storage_wood);
            self.ledger.construction[Resource::Wood.index()] += f64::from(used);
            home.chests.push(Inventory::EMPTY);
            a.memory.home_room_seen += c.inventory.chest_capacity;
            self.today.storage_built += 1;
            self.events.push(
                tick,
                Some(a.id),
                a.pos,
                EventKind::BuiltStorage { home: hid },
            );
            a.goal = Goal::idle(a.pos, tick);
        }
    }

    fn make_newborn(&mut self, pi: usize, tick: u64) -> Agent {
        let c = self.config.clone();
        let id = self.next_id;
        self.next_id += 1;
        let parent_home = self.agents[pi].home.expect("eligible parent has a home");
        let home_pos = self.world.homes[parent_home as usize].pos;
        let j = c.life.trait_inherit_jitter;
        let rng = &mut self.rng_births;
        let p = self.agents[pi].traits;
        let mut jit = |v: f32| (v + rng.range_f32(-j, j)).clamp(0.0, 1.0);
        let traits = Traits {
            curiosity: jit(p.curiosity),
            caution: jit(p.caution),
            sociability: jit(p.sociability),
            diligence: jit(p.diligence),
        };
        let mut skills = self.agents[pi].skills;
        for s in skills.iter_mut() {
            s.aptitude = (s.aptitude + rng.range_f32(-j, j))
                .clamp(c.skills.aptitude_min, c.skills.aptitude_max);
            s.practice = 0.0;
        }
        let memory = AgentMemory::inherit_from(
            &self.agents[pi].memory,
            self.world.dims,
            home_pos,
            c.memory.newborn_known_radius as i32,
            c.memory.newborn_resource_share,
            tick,
            self.rates.freshness_half_life_ticks,
        );

        // Food for the newborn: from the parent's hands, then the home chests.
        let mut inv = Inventory::EMPTY;
        let mut want = c.life.newborn_food.min(c.inventory.carry_juvenile);
        let from_parent = self.agents[pi].inventory.take(Resource::Food, want);
        inv.food += from_parent;
        want -= from_parent;
        if want > 0.0 {
            for chest in self.world.homes[parent_home as usize].chests.iter_mut() {
                let m = chest.take(Resource::Food, want);
                inv.food += m;
                want -= m;
            }
        }
        self.ledger.transferred[0] += f64::from(inv.food);

        let parent = &mut self.agents[pi];
        parent.energy = (parent.energy - c.life.birth_energy_cost).max(0.0);
        parent.last_birth_tick = Some(tick);
        parent.stats.births += 1;
        parent.goal = Goal::idle(parent.pos, tick);
        let parent_id = parent.id;
        let parent_forage_rate = parent.forage_rate;
        let parent_craft_aptitude = parent.craft.aptitude;
        insert_sorted(&mut self.world.homes[parent_home as usize].residents, id);
        self.total_births += 1;
        self.today.births += 1;
        self.events.push(
            tick,
            Some(id),
            home_pos,
            EventKind::Born {
                parent: parent_id,
                home: parent_home,
                food: inv.food,
            },
        );
        let mut baby = Agent {
            id,
            pos: home_pos,
            hunger: 0.1,
            energy: 1.0,
            health: 1.0,
            frailty: 0.0,
            age_ticks: 0,
            born_tick: tick + 1,
            last_birth_tick: None,
            next_reproduce_try: 0,
            parent: Some(parent_id),
            inventory: inv,
            movement_points: 0,
            memory,
            traits,
            skills,
            goal: Goal::idle(home_pos, tick + 1),
            route: None,
            home: Some(parent_home),
            home_since: tick + 1,
            birth_home: Some((parent_home, home_pos)),
            home_plan: None,
            last_observe_tick: tick,
            moved: true,
            frailty_crowding: 0.0,
            stats: AgentStats::default(),
            pathology: PathologyState {
                last_progress_tick: tick + 1,
                ..PathologyState::default()
            },
            practiced_today: 0,
            frontier_cache: None,
            line_memo: Default::default(),
            craft: Skill {
                aptitude: inherited_craft_aptitude(parent_craft_aptitude, self.world.seed, id, &c),
                practice: 0.0,
            },
            forage_rate: parent_forage_rate,
            forage_today_food: 0.0,
            forage_today_ticks: 0,
            last_share_tick: tick,
        };
        baby.memory.note_home(
            parent_home,
            home_pos,
            home_sighting(&self.world, parent_home, c.homes.comfortable_capacity),
            tick,
            c.memory.home_slots as usize,
        );
        baby
    }

    // -----------------------------------------------------------------------
    // Apply
    // -----------------------------------------------------------------------

    fn apply(&mut self, i: usize, intent: Intent, tick: u64) {
        let c = &self.config;
        let r = &self.rates;
        let world = &mut self.world;
        let a = &mut self.agents[i];

        let mut resting = false;
        match intent {
            Intent::Move => {
                a.movement_points += c.movement.mp_per_tick;
                a.stats.ticks_moving += 1;
                let mut moved_any = false;
                if let Some(route) = a.route.as_mut() {
                    while let Some(&next) = route.remaining().first() {
                        let Some(cost) = world.step_in_cost(a.pos, next, &c.movement) else {
                            // Blocked (e.g. a route disruption the agent had
                            // not seen): it now knows, and re-plans.
                            let path = world.tile(next).path_level;
                            let d = world.disruption(next);
                            a.memory.note_changed(
                                next,
                                path,
                                tick,
                                c.memory.changed_slots as usize,
                            );
                            a.memory.note_disruption(
                                next,
                                d,
                                tick,
                                a.pos,
                                c.memory.disruption_slots as usize,
                            );
                            a.route = None;
                            break;
                        };
                        let diag = is_diagonal(next.x - a.pos.x, next.y - a.pos.y);
                        let need = step_cost(cost, diag, &c.movement);
                        if need > a.movement_points {
                            break;
                        }
                        a.movement_points -= need;
                        a.pos = next;
                        route.next += 1;
                        moved_any = true;
                        a.stats.tiles_moved += 1;
                        self.today.tiles_moved += 1;
                        let tile = world.tile_mut(next);
                        tile.traffic += 1.0;
                        tile.crossed_today = true;
                        a.memory
                            .cross(next, c.infrastructure.frequent_slots as usize);
                    }
                } else {
                    a.movement_points = 0;
                }
                if a.route.as_ref().is_some_and(|r| r.done()) {
                    a.route = None;
                    a.movement_points = 0;
                }
                if moved_any {
                    a.moved = true;
                    a.pathology.last_progress_tick = tick;
                } else if a.route.is_some() && a.movement_points > 0 {
                    // Still on a route, waiting for enough MP for the next
                    // tile. A blocked step clears the route above, so it does
                    // not count as progress (or the stuck detector never fires).
                    a.pathology.last_progress_tick = tick;
                }
            }
            Intent::Eat => {
                let per = c.needs.hunger_per_food;
                let ideal = (a.hunger / per + 0.5).floor().max(1.0);
                let amount = a
                    .inventory
                    .take(Resource::Food, ideal.min(c.needs.max_meal));
                a.hunger = (a.hunger - amount * per).max(0.0);
                self.ledger.eaten += f64::from(amount);
                self.today.food_eaten += f64::from(amount);
                a.stats.food_eaten += amount;
                a.pathology.last_progress_tick = tick;
                self.events
                    .push(tick, Some(a.id), a.pos, EventKind::Ate { amount });
                a.goal = Goal::idle(a.pos, tick);
            }
            Intent::Rest => {
                resting = true;
                let mut gain = c.needs.rest_per_tick;
                if let Some(h) = a.home.map(|h| &world.homes[h as usize]) {
                    if h.pos == a.pos {
                        gain =
                            c.needs.rest_per_tick_home * (1.0 + 0.1 * f32::from(h.shelter_level));
                        let crowded = h
                            .over_capacity_since
                            .is_some_and(|s| tick.saturating_sub(s) > r.crowding_grace_ticks);
                        if crowded {
                            let extra = h.residents.len() as f32
                                - h.comfortable_capacity(c.homes.comfortable_capacity) as f32;
                            gain *= 1.0
                                - (c.homes.crowding_rest_penalty * extra)
                                    .min(c.homes.crowding_rest_penalty_max);
                        }
                    }
                }
                a.energy = (a.energy + gain).min(1.0);
                a.pathology.last_progress_tick = tick;
            }
            Intent::Idle => {
                a.pathology.idle_today += 1;
            }
            _ => {}
        }
        if intent != Intent::Move {
            a.movement_points = 0;
        }

        // Time spent on food trips (travel and gathering) for experienced returns.
        if a.goal.kind == GoalKind::GatherFood
            && matches!(intent, Intent::Move | Intent::Gather(Resource::Food))
        {
            a.forage_today_ticks += 1;
        }

        // --- Needs ---------------------------------------------------------
        let juvenile = a.is_juvenile(r);
        let hunger_rate = if juvenile {
            r.hunger_per_tick * c.needs.juvenile_hunger_factor
        } else {
            r.hunger_per_tick
        };
        a.hunger = (a.hunger + hunger_rate).min(1.0);
        if !resting {
            a.energy = (a.energy - c.needs.energy_per_active_tick).max(0.0);
        }
        let deprived = a.hunger >= c.needs.very_hungry || a.energy <= 0.0;
        if a.hunger >= 1.0 {
            a.health -= r.health_loss_starving_per_tick;
        } else if deprived {
            a.health -= r.health_loss_deprived_per_tick;
        } else {
            a.health += r.health_recovery_per_tick;
        }
        if deprived || a.health < c.needs.low_health {
            a.frailty += r.frailty_per_deprived_tick;
            a.stats.ticks_deprived += 1;
        }
        if let Some(h) = a.home.map(|h| &world.homes[h as usize]) {
            if h.over_capacity_since
                .is_some_and(|s| tick.saturating_sub(s) > r.crowding_grace_ticks)
            {
                a.frailty += r.frailty_per_crowded_tick;
                a.frailty_crowding += r.frailty_per_crowded_tick;
            }
        }
        a.age_ticks += 1;
        let ceiling = a.health_ceiling(r);
        a.health = a.health.min(ceiling).max(0.0);

        // --- Stuck detector ----------------------------------------------
        if !resting
            && tick.saturating_sub(a.pathology.last_progress_tick)
                >= u64::from(c.pathology.stuck_ticks)
        {
            if !a.pathology.stuck_flagged {
                a.pathology.stuck_flagged = true;
                self.pathologies[1] += 1;
                log_pathology(&mut self.pathology_log, tick, a.id, PathologyKind::Stuck);
                self.events.push(
                    tick,
                    Some(a.id),
                    a.pos,
                    EventKind::Pathology {
                        kind: PathologyKind::Stuck,
                    },
                );
            }
        } else if tick == a.pathology.last_progress_tick {
            a.pathology.stuck_flagged = false;
        }
        debug_assert!(a.hunger.is_finite() && a.energy.is_finite() && a.health.is_finite());
    }

    fn process_deaths(&mut self, tick: u64) {
        let mut i = 0;
        while i < self.agents.len() {
            let a = &self.agents[i];
            let ceiling = a.health_ceiling(&self.rates);
            let cause = if ceiling <= 0.0 {
                Some(DeathCause::OldAge)
            } else if a.health <= 0.0 {
                Some(DeathCause::Deprivation)
            } else {
                None
            };
            match cause {
                None => i += 1,
                Some(cause) => {
                    let dead = self.agents.remove(i);
                    self.bury(dead, cause, tick);
                }
            }
        }
    }

    fn bury(&mut self, mut a: Agent, cause: DeathCause, tick: u64) {
        let c = &self.config;
        // Transfer the whole inventory: home chests first, else a pile here.
        let goods = a.inventory;
        let mut into_home = None;
        let mut remaining = goods;
        if let Some(h) = a.home {
            let home = &mut self.world.homes[h as usize];
            for r in Resource::GOODS {
                for chest in home.chests.iter_mut() {
                    let amt = remaining.get(r);
                    if amt <= 0.0 {
                        break;
                    }
                    remaining.transfer_to(chest, r, amt, c.inventory.chest_capacity, &c.inventory);
                }
            }
            if remaining != goods {
                into_home = Some(h);
            }
        }
        if !remaining.is_empty() {
            match self.world.piles.iter_mut().find(|p| p.pos == a.pos) {
                Some(p) => p.goods.add_all(&remaining),
                None => self.world.piles.push(Pile {
                    pos: a.pos,
                    goods: remaining,
                }),
            }
        }
        for r in Resource::GOODS {
            self.ledger.transferred[r.index()] += f64::from(goods.get(r));
        }
        a.inventory = Inventory::EMPTY;
        if !goods.is_empty() {
            self.events.push(
                tick,
                Some(a.id),
                a.pos,
                EventKind::Dropped { goods, into_home },
            );
        }
        if let Some(h) = a.home {
            self.leave_home(a.id, h, tick);
        }

        let age_decline = a.age_decline(&self.rates);
        let wear = age_decline + a.frailty;
        let explanation = match cause {
            DeathCause::OldAge => format!(
                "health ceiling reached 0: {:.0}% age, {:.0}% frailty ({:.0} days deprived{})",
                100.0 * age_decline / wear.max(1e-6),
                100.0 * a.frailty / wear.max(1e-6),
                a.stats.ticks_deprived as f32 / TICKS_PER_DAY as f32,
                if a.frailty_crowding > 0.0 {
                    ", some from crowding"
                } else {
                    ""
                }
            ),
            DeathCause::Deprivation => format!(
                "health reached 0 from unmet needs (hunger {:.2}, energy {:.2}, ceiling {:.2})",
                a.hunger,
                a.energy,
                a.health_ceiling(&self.rates)
            ),
        };
        let starved_beside_food = cause == DeathCause::Deprivation && self.knew_reachable_food(&a);
        if starved_beside_food {
            self.pathologies[2] += 1;
            log_pathology(
                &mut self.pathology_log,
                tick,
                a.id,
                PathologyKind::StarvedBesideFood,
            );
            self.events.push(
                tick,
                Some(a.id),
                a.pos,
                EventKind::Pathology {
                    kind: PathologyKind::StarvedBesideFood,
                },
            );
        }
        let rec = DeathRecord {
            agent: a.id,
            tick,
            pos: a.pos,
            cause,
            age_days: a.age_days(),
            age_decline,
            frailty: a.frailty,
            frailty_from_crowding: a.frailty_crowding,
            days_deprived: a.stats.ticks_deprived as f32 / TICKS_PER_DAY as f32,
            home: a.home,
            children: a.stats.births,
            explanation,
            starved_beside_food,
        };
        self.events
            .push(tick, Some(a.id), a.pos, EventKind::Died { cause });
        match cause {
            DeathCause::Deprivation => {
                self.today.deaths_deprivation += 1;
                self.total_deaths[0] += 1;
            }
            DeathCause::OldAge => {
                self.today.deaths_old_age += 1;
                self.total_deaths[1] += 1;
            }
        }
        if a.parent.is_some() {
            // Lifespan statistics only for agents whose whole life we observed.
            self.lifespans.0 += f64::from(a.age_days());
            self.lifespans.1 += 1;
        }
        self.deaths.push(rec);
        self.traces.remove(&a.id);
        self.graveyard.push(a);
        if self.graveyard.len() > GRAVEYARD_CAP {
            self.graveyard.remove(0);
        }
    }

    /// Did the agent remember food it could have reached within the
    /// starved-beside-food travel budget?
    fn knew_reachable_food(&mut self, a: &Agent) -> bool {
        let mc = &self.config.movement;
        let budget = self.config.pathology.starved_reach_ticks * mc.mp_per_tick;
        let mut foods: Vec<Pos> = a
            .memory
            .resources
            .iter()
            .filter(|m| m.resource == Resource::Food && m.amount >= 1.0)
            .map(|m| m.pos)
            .collect();
        foods.sort_by_key(|p| (p.octile10(a.pos), *p));
        for p in foods.into_iter().take(5) {
            if p == a.pos {
                return true;
            }
            let found = if self.config.sim.agent_kind == AgentKind::Omniscient {
                self.scratch.find_path(
                    &self.world,
                    &Omniscient,
                    a.pos,
                    p,
                    mc,
                    mc.astar_max_expansions,
                )
            } else {
                self.scratch.find_path(
                    &self.world,
                    &AgentBelief(&a.memory),
                    a.pos,
                    p,
                    mc,
                    mc.astar_max_expansions,
                )
            };
            if let Ok(path) = found {
                if path.cost <= budget {
                    return true;
                }
            }
        }
        false
    }

    // -----------------------------------------------------------------------
    // Daily
    // -----------------------------------------------------------------------

    fn daily(&mut self, tick: u64) {
        let c = self.config.clone();
        // Weather: droughts start and end, then regrowth is scaled by them
        // (and by the season, if enabled).
        let day = (tick / TICKS_PER_DAY) as u32;
        let (started, ended) = self
            .weather
            .update(day, &self.world, &c, &mut self.rng_weather);
        for d in started {
            self.events.push(
                tick,
                None,
                d.center,
                EventKind::DroughtStarted {
                    radius: d.radius,
                    days: d.end_day - d.start_day,
                    severity: d.severity,
                    scripted: d.scripted,
                },
            );
        }
        for d in ended {
            self.events
                .push(tick, None, d.center, EventKind::DroughtEnded);
        }
        // Route disruptions start and end through ordinary movement costs.
        let (started, ended) =
            self.disruptions
                .update(day, &mut self.world, &c.disruption.scripted);
        for d in started {
            let pos = d
                .tiles
                .first()
                .map_or(Pos::default(), |&t| self.world.dims.pos(t));
            self.events.push(
                tick,
                None,
                pos,
                EventKind::RouteDisrupted {
                    tiles: d.tiles.len() as u32,
                    days: d.end_day - d.start_day,
                    effect: d.effect,
                },
            );
        }
        for d in ended {
            let pos = d
                .tiles
                .first()
                .map_or(Pos::default(), |&t| self.world.dims.pos(t));
            self.events.push(
                tick,
                None,
                pos,
                EventKind::RouteRestored {
                    tiles: d.tiles.len() as u32,
                },
            );
        }
        let factors = self.weather.regrowth_factors(&self.world, day, &c.weather);
        let (food, wood) = resources::regrow(&mut self.world, &c.resources, |i| factors[i]);
        self.ledger.regrown[0] += food;
        self.ledger.regrown[1] += wood;

        // Spoilage.
        let mut spoiled = 0.0f64;
        for a in self.agents.iter_mut() {
            let s = a.inventory.food * c.inventory.carried_spoil_per_day;
            spoiled += f64::from(a.inventory.take(Resource::Food, s));
        }
        for h in self.world.homes.iter_mut() {
            for ch in h.chests.iter_mut() {
                let s = ch.food * c.inventory.stored_spoil_per_day;
                spoiled += f64::from(ch.take(Resource::Food, s));
            }
        }
        for p in self.world.piles.iter_mut() {
            let s = p.goods.food * c.inventory.carried_spoil_per_day;
            spoiled += f64::from(p.goods.take(Resource::Food, s));
        }
        self.world.piles.retain(|p| !p.goods.is_empty());
        self.ledger.spoiled[0] += spoiled;

        // Skill practice decays when unused; idle detector.
        for a in self.agents.iter_mut() {
            for r in Resource::ALL {
                if a.practiced_today & (1 << r.index()) == 0 {
                    let s = &mut a.skills[r.index()];
                    s.practice = (s.practice - c.skills.practice_decay_per_day).max(0.0);
                }
            }
            if a.practiced_today & CRAFT_PRACTICE_BIT == 0 {
                a.craft.practice = (a.craft.practice - c.skills.practice_decay_per_day).max(0.0);
            }
            a.practiced_today = 0;
            if a.forage_today_ticks >= 2 {
                let rate = a.forage_today_food / a.forage_today_ticks as f32;
                a.forage_rate = 0.8 * a.forage_rate + 0.2 * rate;
            }
            a.forage_today_food = 0.0;
            a.forage_today_ticks = 0;
            a.memory
                .daily_decay(c.social.reciprocity_decay_per_day, 1.0 - 1.0 / 30.0);
            if a.pathology.idle_today > c.pathology.idle_ticks_per_day {
                self.pathologies[3] += 1;
                log_pathology(&mut self.pathology_log, tick, a.id, PathologyKind::Idle);
                self.events.push(
                    tick,
                    Some(a.id),
                    a.pos,
                    EventKind::Pathology {
                        kind: PathologyKind::Idle,
                    },
                );
            }
            a.pathology.idle_today = 0;
        }

        // Crowding clocks.
        for h in self.world.homes.iter_mut() {
            let over =
                h.residents.len() as u32 > h.comfortable_capacity(c.homes.comfortable_capacity);
            match (over, h.over_capacity_since) {
                (true, None) => h.over_capacity_since = Some(tick),
                (false, Some(_)) => h.over_capacity_since = None,
                _ => {}
            }
        }

        // Traffic decays daily; heavily crossed tiles wear into trails, which
        // fade back after a spell below the threshold. Roads are built, not
        // worn, and are left alone here.
        let tr = &c.trails;
        for t in self.world.tiles.iter_mut() {
            if t.traffic > 0.0 {
                t.traffic *= tr.decay;
                if t.traffic < 0.01 {
                    t.traffic = 0.0;
                }
            }
            if !tr.enabled || !t.is_land() {
                continue;
            }
            match t.path_level {
                crate::world::PathLevel::None if t.traffic >= tr.threshold => {
                    t.path_level = crate::world::PathLevel::Trail;
                    t.trail_fade_days = 0;
                    self.today.trails_formed += 1;
                }
                crate::world::PathLevel::Trail => {
                    if t.traffic >= tr.threshold {
                        t.trail_fade_days = 0;
                    } else {
                        t.trail_fade_days += 1;
                        if u32::from(t.trail_fade_days) >= tr.fade_days {
                            t.path_level = crate::world::PathLevel::None;
                            t.trail_fade_days = 0;
                            self.today.trails_faded += 1;
                        }
                    }
                }
                _ => {}
            }
        }

        self.daily_roads(tick);
        self.settlements.update(&self.world, day, &c.settlement);
        let m = self.collect_metrics(day);
        self.metrics.push(m);
        self.today = DayCounters::default();
        let keep_from = (tick + 1).saturating_sub(TRADE_WINDOW_TICKS);
        self.trade_log.retain(|r| r.tick >= keep_from);
    }

    fn collect_metrics(&self, day: u32) -> DailyMetrics {
        let r = &self.rates;
        let n = self.agents.len();
        let nf = n.max(1) as f32;
        let mut m = DailyMetrics {
            day: day + 1,
            population: n as u32,
            ..DailyMetrics::default()
        };
        let (mut h, mut e, mut hp, mut fr, mut age, mut known) =
            (0f64, 0f64, 0f64, 0f64, 0f64, 0f64);
        let mut homed = 0u32;
        let mut carried = 0f64;
        for a in &self.agents {
            if a.is_juvenile(r) {
                m.juveniles += 1;
            } else if a.age_ticks >= r.elder_ticks {
                m.elders += 1;
            } else {
                m.adults += 1;
            }
            h += f64::from(a.hunger);
            e += f64::from(a.energy);
            hp += f64::from(a.health);
            fr += f64::from(a.frailty);
            age += f64::from(a.age_days());
            known += f64::from(a.memory.known_count());
            carried += f64::from(a.inventory.food);
            if a.home.is_some() {
                homed += 1;
            }
        }
        let nf64 = f64::from(nf);
        m.mean_hunger = (h / nf64) as f32;
        m.mean_energy = (e / nf64) as f32;
        m.mean_health = (hp / nf64) as f32;
        m.mean_frailty = (fr / nf64) as f32;
        m.mean_age_days = (age / nf64) as f32;
        m.mean_known_tiles = (known / nf64) as f32;
        m.homed_fraction = homed as f32 / nf;
        m.food_carried = carried as f32;
        m.births = self.today.births;
        m.deaths_deprivation = self.today.deaths_deprivation;
        m.deaths_old_age = self.today.deaths_old_age;
        m.total_births = self.total_births;
        m.total_deaths_deprivation = self.total_deaths[0];
        m.total_deaths_old_age = self.total_deaths[1];
        m.mean_lifespan_days = if self.lifespans.1 > 0 {
            (self.lifespans.0 / f64::from(self.lifespans.1)) as f32
        } else {
            0.0
        };
        let mut residents = 0u32;
        for home in &self.world.homes {
            if home.is_vacant() {
                m.homes_vacant += 1;
            } else {
                m.homes_occupied += 1;
                residents += home.residents.len() as u32;
            }
            m.chests += home.chests.len() as u32;
            let s = home.stored();
            m.food_stored += s.food;
            m.wood_stored += s.wood;
        }
        m.mean_residents = residents as f32 / m.homes_occupied.max(1) as f32;
        m.food_world = self
            .world
            .tiles
            .iter()
            .map(|t| f64::from(t.food))
            .sum::<f64>() as f32;
        m.food_gathered = self.today.food_gathered as f32;
        m.food_eaten = self.today.food_eaten as f32;
        m.tiles_moved = self.today.tiles_moved;
        m.travel_per_food = if self.today.food_gathered > 0.0 {
            (self.today.tiles_moved as f64 / self.today.food_gathered) as f32
        } else {
            0.0
        };
        m.route_failures = self.today.route_failures;
        m.astar_expansions_per_search = if self.today.astar_searches > 0 {
            self.today.astar_expansions as f32 / self.today.astar_searches as f32
        } else {
            0.0
        };
        m.homes_founded = self.today.homes_founded;
        m.home_joins = self.today.home_joins;
        m.home_moves = self.today.home_moves;
        m.storage_built = self.today.storage_built;
        m.road_tiles = self
            .world
            .tiles
            .iter()
            .filter(|t| t.path_level == crate::world::PathLevel::Road)
            .count() as u32;
        m.shelters = self
            .world
            .homes
            .iter()
            .map(|h| u32::from(h.shelter_level))
            .sum();
        m.trades = self.today.trades;
        m.food_traded = self.today.food_traded as f32;
        m.tips_shared = self.today.tips;
        m.forage_radius = if self.today.forage_distance_n > 0 {
            (self.today.forage_distance_sum / self.today.forage_distance_n as f64) as f32
        } else {
            0.0
        };
        let adults: Vec<f32> = self
            .agents
            .iter()
            .filter(|a| !a.is_juvenile(r))
            .map(|a| a.forage_rate)
            .collect();
        m.mean_forage_rate = adults.iter().sum::<f32>() / adults.len().max(1) as f32;
        m.clark_evans = emergence::clark_evans(&self.world);
        m.depletion_ratio = emergence::depletion_ratio(&self.world);
        m.specialization = emergence::specialization(&self.agents, r);
        m.mean_home_age_days = emergence::mean_home_age_days(&self.world, self.tick);
        let land = self
            .world
            .tiles
            .iter()
            .filter(|t| t.is_land())
            .count()
            .max(1);
        let in_drought = (0..self.world.dims.len() as u32)
            .map(|i| self.world.dims.pos(i))
            .filter(|&p| self.world.tile(p).is_land() && self.weather.in_drought(p))
            .count();
        m.drought_area = in_drought as f32 / land as f32;
        self.observatory_metrics(&mut m);
        m.disrupted_tiles = DisruptionState::disrupted_tiles(&self.world);
        m.trail_tiles = self
            .world
            .tiles
            .iter()
            .filter(|t| t.path_level != crate::world::PathLevel::None)
            .count() as u32;
        m.settlement_candidates = self.settlements.count(SizeClass::Candidate);
        m.hamlets = self.settlements.count(SizeClass::Hamlet);
        m.villages = self.settlements.count(SizeClass::Village);
        m.towns = self.settlements.count(SizeClass::Town);
        m.largest_settlement_residents = self
            .settlements
            .confirmed()
            .map(|t| t.residents)
            .max()
            .unwrap_or(0);
        m.pathology_oscillating = self.pathologies[0];
        m.pathology_stuck = self.pathologies[1];
        m.pathology_starved_beside_food = self.pathologies[2];
        m.pathology_idle = self.pathologies[3];
        let total: u64 = self.today.goal_ticks.iter().sum();
        m.goal_share = self
            .today
            .goal_ticks
            .iter()
            .map(|&g| g as f32 / total.max(1) as f32)
            .collect();
        m.at_cap = self.hit_cap;
        m
    }

    // -----------------------------------------------------------------------
    // Hashing and invariants
    // -----------------------------------------------------------------------

    pub fn state_hash(&self) -> u64 {
        let mut h = StableHasher::new();
        h.u64(self.tick);
        self.config.stable_hash(&mut h);
        self.world.stable_hash_world(&mut h);
        h.len(self.agents.len());
        for a in &self.agents {
            a.stable_hash(&mut h);
        }
        h.u32(self.next_id);
        self.rng_agents.stable_hash(&mut h);
        self.rng_births.stable_hash(&mut h);
        self.rng_conflicts.stable_hash(&mut h);
        self.rng_weather.stable_hash(&mut h);
        self.weather.stable_hash(&mut h);
        self.disruptions.stable_hash(&mut h);
        h.finish()
    }

    /// Resource conservation and bounds checks. Returns the first violation.
    pub fn check_invariants(&self) -> Result<(), String> {
        let c = &self.config;
        let totals = self.world.resource_totals();
        let carried = self.carried_totals();
        for r in Resource::GOODS {
            let actual = f64::from(totals.get(r)) + carried[r.index()];
            let expected = self.ledger.expected_total(r);
            let tol = 1e-3 * expected.abs().max(1.0);
            if (actual - expected).abs() > tol {
                return Err(format!(
                    "{} not conserved: actual {actual:.3}, ledger {expected:.3}",
                    r.name()
                ));
            }
        }
        for a in &self.agents {
            let cap = a.carry_capacity(c, &self.rates);
            if !a.inventory.is_valid() || a.inventory.weight(&c.inventory) > cap + 1e-3 {
                return Err(format!("agent {} inventory invalid or over capacity", a.id));
            }
            for (name, v) in [
                ("hunger", a.hunger),
                ("energy", a.energy),
                ("health", a.health),
            ] {
                if !(0.0..=1.0).contains(&v) {
                    return Err(format!("agent {} {name} out of range: {v}", a.id));
                }
            }
            if a.health > a.health_ceiling(&self.rates) + 1e-6 {
                return Err(format!("agent {} health above ceiling", a.id));
            }
            if !self.world.tile(a.pos).is_land() {
                return Err(format!("agent {} in the water at {:?}", a.id, a.pos));
            }
        }
        for h in &self.world.homes {
            for ch in &h.chests {
                if !ch.is_valid() || ch.weight(&c.inventory) > c.inventory.chest_capacity + 1e-3 {
                    return Err(format!("home {} chest over capacity", h.id));
                }
            }
            if h.chests.len() as u32 > c.inventory.max_chests {
                return Err(format!("home {} has too many chests", h.id));
            }
            if !h.residents.windows(2).all(|w| w[0] < w[1]) {
                return Err(format!("home {} residents not sorted", h.id));
            }
            for &rid in &h.residents {
                match self.agent(rid) {
                    Some(a) if a.home == Some(h.id) => {}
                    _ => return Err(format!("home {} lists non-resident {rid}", h.id)),
                }
            }
        }
        for a in &self.agents {
            if let Some(hid) = a.home {
                if !self.world.homes[hid as usize].residents.contains(&a.id) {
                    return Err(format!("agent {} not in its home's residents", a.id));
                }
            }
        }
        if self.agents.len() as u32 > c.life.population_cap {
            return Err("population above cap".into());
        }
        Ok(())
    }
}

/// Observe the sight square and update memory.
pub fn observe(a: &mut Agent, world: &World, c: &Config, r: &TickRates, tick: u64) {
    if c.sim.agent_kind == AgentKind::Omniscient {
        observe_omniscient(a, world, c, tick);
        return;
    }
    let half_life = r.freshness_half_life_ticks;
    let mem_cfg = &c.memory;
    let home_slots = mem_cfg.home_slots as usize;
    let changed_slots = mem_cfg.changed_slots as usize;
    let disruption_slots = mem_cfg.disruption_slots as usize;
    let mut seen: Vec<Pos> = Vec::with_capacity(169);
    world.vis.for_each_visible(world, a.pos, |p| seen.push(p));
    let mut amounts: Vec<(Pos, [f32; 3])> = Vec::with_capacity(seen.len());
    for &p in &seen {
        let t = world.tile(p);
        let pile = if world.piles.is_empty() {
            None
        } else {
            world.pile_at(p).map(|pl| pl.goods)
        };
        let extra = |r: Resource| pile.map_or(0.0, |g| g.get(r));
        amounts.push((
            p,
            [
                t.food + extra(Resource::Food),
                t.wood + extra(Resource::Wood),
                t.stone + extra(Resource::Stone),
            ],
        ));
    }
    a.memory
        .observe_resources(a.pos, &amounts, tick, mem_cfg, half_life);
    for &p in &seen {
        a.memory.learn(p);
        let t = world.tile(p);
        for &hid in world.homes_on(p) {
            a.memory.note_home(
                hid,
                p,
                home_sighting(world, hid, c.homes.comfortable_capacity),
                tick,
                home_slots,
            );
        }
        a.memory.note_changed(p, t.path_level, tick, changed_slots);
        a.memory
            .note_disruption(p, world.disruption(p), tick, a.pos, disruption_slots);
    }
    if !mem_cfg.enabled {
        a.memory.forget_resources_not_in(|p| seen.contains(&p));
    }
    if let Some(hid) = a.home {
        let h = &world.homes[hid as usize];
        if h.pos == a.pos {
            a.memory.home_food_seen = h.stored().food;
            a.memory.home_room_seen = h
                .chests
                .iter()
                .map(|ch| (c.inventory.chest_capacity - ch.weight(&c.inventory)).max(0.0))
                .sum();
        }
    }
    a.last_observe_tick = tick;
    a.moved = false;
}

/// What an agent standing in sight of home `hid` can see of it.
fn home_sighting(world: &World, hid: HomeId, base_capacity: u32) -> HomeSighting {
    let h = &world.homes[hid as usize];
    HomeSighting {
        residents: h.residents.len() as u32,
        has_storage: !h.chests.is_empty(),
        comfortable: h.comfortable_capacity(base_capacity),
        // A home always counts itself, even if the tile index is not built yet.
        tile_homes: (world.homes_on(h.pos).len() as u32).max(1),
    }
}

/// Omniscient baseline (evaluation only): perfect knowledge within the same
/// memory limits as normal agents. The agent knows the terrain of the whole
/// map, and for each resource it remembers the *nearest* tiles that hold it
/// (true amounts, seen now), up to its pool size; likewise the nearest homes.
/// Choosing, scoring and routing use the true world (`decision::omniscient`).
/// Its decision logic is the same as a normal agent's, so the gap between the
/// two measures what limited sight and memory cost. `memory.enabled = false`
/// does not apply to it: it always knows.
///
/// Tiles are visited ring by ring (Chebyshev distance), each ring's perimeter
/// in a fixed order, and the scan stops once every pool is full or the world
/// has nothing more to offer, so it never walks the whole map needlessly.
/// A resource the agent recently failed to reach (its memory still has the
/// `unreachable_until` cooldown) is skipped so it cannot crowd out reachable
/// ones; it is offered again after the cooldown.
fn observe_omniscient(a: &mut Agent, world: &World, c: &Config, tick: u64) {
    let mem = &c.memory;
    let n = world.dims.len();
    if (a.memory.known_count() as usize) < n {
        for i in 0..n as u32 {
            a.memory.learn(world.dims.pos(i));
        }
    }
    let amount_at = |p: Pos, res: Resource| {
        let t = world.tile(p);
        let base = match res {
            Resource::Food => t.food,
            Resource::Wood => t.wood,
            Resource::Stone => t.stone,
            Resource::Tools => 0.0,
        };
        base + world.pile_at(p).map_or(0.0, |pl| pl.goods.get(res))
    };
    // Use failed_targets, not the resource pool: set_resources replaces the
    // pool, so a cooldown stored only on a resource entry would be wiped on
    // the next re-observe (every `reobserve_ticks`) instead of lasting the
    // full unreachable window. mark_unreachable always records failed_targets.
    let cooling = |p: Pos, _res: Resource| a.memory.is_failed_target(p, tick);
    // Blocked tiles are not offered as goals (except the tile the agent is
    // already on, so it can still plan a way out). True route costs only
    // hold if the pools never point at impassable targets.
    let usable = |p: Pos| p == a.pos || world.disruption(p) != crate::world::Disruption::Blocked;
    // How many entries each pool can actually get: its size, or fewer if the
    // world holds fewer eligible tiles.
    let mut available = [0usize; 3];
    for i in 0..n as u32 {
        let p = world.dims.pos(i);
        if !usable(p) {
            continue;
        }
        for res in Resource::ALL {
            if amount_at(p, res) >= mem.min_amount {
                available[res.index()] += 1;
            }
        }
    }
    let caps = [
        AgentMemory::slots(Resource::Food, mem).min(available[0]),
        AgentMemory::slots(Resource::Wood, mem).min(available[1]),
        AgentMemory::slots(Resource::Stone, mem).min(available[2]),
    ];
    let home_cap = (mem.home_slots as usize).min(world.homes.len());
    let mut found: [Vec<crate::agents::memory::ResourceMemory>; 3] = Default::default();
    let mut homes: Vec<(HomeId, Pos)> = Vec::new();
    type Found = [Vec<crate::agents::memory::ResourceMemory>; 3];
    let visit = |p: Pos, found: &mut Found, homes: &mut Vec<(HomeId, Pos)>| {
        if !world.contains(p) || !usable(p) {
            return;
        }
        for res in Resource::ALL {
            let k = res.index();
            if found[k].len() >= caps[k] {
                continue;
            }
            let amount = amount_at(p, res);
            if amount >= mem.min_amount && !cooling(p, res) {
                found[k].push(crate::agents::memory::ResourceMemory {
                    pos: p,
                    resource: res,
                    amount,
                    seen_tick: tick,
                    unreachable_until: 0,
                });
            }
        }
        for &hid in world.homes_on(p) {
            if homes.len() < home_cap {
                homes.push((hid, p));
            }
        }
    };
    let max_r = world.dims.width.max(world.dims.height) as i32;
    for r in 0..=max_r {
        let full = (0..3).all(|k| found[k].len() >= caps[k]) && homes.len() >= home_cap;
        if full {
            break;
        }
        if r == 0 {
            visit(a.pos, &mut found, &mut homes);
            continue;
        }
        // Perimeter of the ring at distance r: top and bottom rows, then the
        // left and right columns without their corners.
        for dx in -r..=r {
            visit(a.pos.offset(dx, -r), &mut found, &mut homes);
            visit(a.pos.offset(dx, r), &mut found, &mut homes);
        }
        for dy in (-r + 1)..r {
            visit(a.pos.offset(-r, dy), &mut found, &mut homes);
            visit(a.pos.offset(r, dy), &mut found, &mut homes);
        }
    }
    let [food, wood, stone] = found;
    let mut all = food;
    all.extend(wood);
    all.extend(stone);
    a.memory.set_resources(all);
    for (hid, p) in homes {
        a.memory.note_home(
            hid,
            p,
            home_sighting(world, hid, c.homes.comfortable_capacity),
            tick,
            mem.home_slots as usize,
        );
    }
    if let Some(hid) = a.home {
        let h = &world.homes[hid as usize];
        a.memory.home_food_seen = h.stored().food;
        a.memory.home_room_seen = h
            .chests
            .iter()
            .map(|ch| (c.inventory.chest_capacity - ch.weight(&c.inventory)).max(0.0))
            .sum();
    }
    a.last_observe_tick = tick;
    a.moved = false;
}

fn log_pathology(
    log: &mut Vec<(u64, u32, PathologyKind)>,
    tick: u64,
    id: u32,
    kind: PathologyKind,
) {
    if log.len() < 10_000 {
        log.push((tick, id, kind));
    }
}

fn insert_sorted(v: &mut Vec<u32>, id: u32) {
    if let Err(pos) = v.binary_search(&id) {
        v.insert(pos, id);
    }
}

/// Random baseline: the options in a uniformly random order, with Idle moved
/// last, so the agent takes the first other option whose target it can reach
/// and only idles when none is reachable.
fn random_choice(mut cands: Vec<Candidate>, rng: &mut Rng) -> Vec<Candidate> {
    if cands.len() > 1 {
        rng.shuffle(&mut cands);
    }
    // Stable: keeps the shuffled order of everything else.
    cands.sort_by_key(|c| c.kind == GoalKind::Idle);
    cands
}

/// Trades kept for network measurement (30 days).
pub const TRADE_WINDOW_TICKS: u64 = 30 * TICKS_PER_DAY;

impl Simulation {
    /// Observatory metrics (A.24). Measurement only.
    fn observatory_metrics(&self, m: &mut DailyMetrics) {
        use crate::simulation::emergence as em;
        let inv = &self.config.inventory;
        m.wealth_gini = em::gini(&em::household_wealth(&self.world, &self.agents, inv));
        let sizes = em::cluster_sizes(&self.world, self.config.settlement.radius as i32);
        let homed: u32 = sizes.iter().map(|s| s.1).sum();
        m.largest_cluster_share = sizes
            .first()
            .map_or(0.0, |s| s.1 as f32 / homed.max(1) as f32);
        m.clusters_2plus = sizes.iter().filter(|s| s.0 >= 2).count() as u32;
        m.mean_move_distance = if self.today.home_moves > 0 {
            (self.today.home_move_distance_sum / f64::from(self.today.home_moves)) as f32
        } else {
            0.0
        };
        let since = (self.tick + 1).saturating_sub(TRADE_WINDOW_TICKS);
        let net = em::trade_network(&self.trade_log, since);
        m.trade_pairs_30d = net.pairs;
        m.trade_mean_degree = net.mean_degree;
        m.trade_degree_gini = net.degree_gini;
        m.trade_cross_household_share = net.cross_household_share;
        m.trade_repeat_share = net.repeat_share;
        let moran =
            |f: fn(&Agent) -> f32| em::morans_i(&self.world, &self.agents, 8, f).unwrap_or(0.0);
        m.moran_curiosity = moran(|a| a.traits.curiosity);
        m.moran_caution = moran(|a| a.traits.caution);
        m.moran_sociability = moran(|a| a.traits.sociability);
        m.moran_diligence = moran(|a| a.traits.diligence);
        m.tools_crafted = self.today.tools_crafted as f32;
        m.trait_copies = self.today.trait_copies;
        {
            let adults: Vec<&Agent> = self
                .agents
                .iter()
                .filter(|a| !a.is_juvenile(&self.rates))
                .collect();
            let n = adults.len().max(1) as f64;
            let mean = |f: fn(&Agent) -> f32| {
                (adults.iter().map(|a| f64::from(f(a))).sum::<f64>() / n) as f32
            };
            m.mean_curiosity = mean(|a| a.traits.curiosity);
            m.mean_caution = mean(|a| a.traits.caution);
            m.mean_sociability = mean(|a| a.traits.sociability);
            m.mean_diligence = mean(|a| a.traits.diligence);
        }
        let adults: Vec<&Agent> = self
            .agents
            .iter()
            .filter(|a| !a.is_juvenile(&self.rates))
            .collect();
        let n = adults.len().max(1) as f32;
        m.tool_holders_share = adults.iter().filter(|a| a.inventory.tools > 0.0).count() as f32 / n;
        m.crafter_share = adults
            .iter()
            .filter(|a| {
                let top = a.skills.iter().map(|s| s.practice).fold(0.0f32, f32::max);
                a.craft.practice > 0.05 && a.craft.practice >= top
            })
            .count() as f32
            / n;
        let prices: Vec<f64> = adults
            .iter()
            .filter(|a| a.memory.tool_price > 0.0)
            .map(|a| f64::from(a.memory.tool_price))
            .collect();
        if prices.len() >= 2 {
            let mean = prices.iter().sum::<f64>() / prices.len() as f64;
            let var = prices.iter().map(|p| (p - mean) * (p - mean)).sum::<f64>()
                / (prices.len() as f64 - 1.0);
            m.tool_price_mean = mean as f32;
            m.tool_price_cv = if mean > 0.0 {
                (var.sqrt() / mean) as f32
            } else {
                0.0
            };
        }
    }
}

/// Bit in `Agent::practiced_today` for crafting (resources use bits 0..3).
pub const CRAFT_PRACTICE_BIT: u8 = 1 << 3;

/// A founder's crafting aptitude, uniform in the gathering-aptitude range,
/// from a stateless hash of (world seed, agent id). Drawing it from an RNG
/// stream would shift every later draw and change runs with tools disabled.
fn craft_aptitude(seed: u64, id: u32, c: &Config) -> f32 {
    let u =
        crate::rng::unit_f32_from_u64(crate::hash::mix_words(&[seed, u64::from(id), 0xC2AF_7001]));
    c.skills.aptitude_min + u * (c.skills.aptitude_max - c.skills.aptitude_min)
}

/// A newborn's crafting aptitude: the parent's, jittered like other traits,
/// again from a stateless hash.
fn inherited_craft_aptitude(parent: f32, seed: u64, id: u32, c: &Config) -> f32 {
    let u =
        crate::rng::unit_f32_from_u64(crate::hash::mix_words(&[seed, u64::from(id), 0xC2AF_7002]));
    let j = c.life.trait_inherit_jitter;
    (parent + (u * 2.0 - 1.0) * j).clamp(c.skills.aptitude_min, c.skills.aptitude_max)
}

impl Simulation {
    /// Craft tools from carried wood and stone (A.26).
    fn do_craft(&mut self, i: usize, tick: u64) {
        let c = self.config.clone();
        let tc = &c.tools;
        let a = &mut self.agents[i];
        if !tc.enabled || a.inventory.wood < tc.craft_wood || a.inventory.stone < tc.craft_stone {
            a.goal = Goal::idle(a.pos, tick);
            return;
        }
        a.goal.work += 1;
        a.pathology.last_progress_tick = tick;
        a.practiced_today |= CRAFT_PRACTICE_BIT;
        a.craft.practice = (a.craft.practice + tc.practice_per_tick).min(c.skills.practice_max);
        if a.goal.work < tc.craft_ticks.max(1) {
            return;
        }
        let w = a.inventory.take(Resource::Wood, tc.craft_wood);
        let st = a.inventory.take(Resource::Stone, tc.craft_stone);
        self.ledger.construction[Resource::Wood.index()] += f64::from(w);
        self.ledger.construction[Resource::Stone.index()] += f64::from(st);
        let cap = a.carry_capacity(&c, &self.rates);
        let made = (tc.output_per_batch * a.craft.level()).min(a.inventory.room_for(
            Resource::Tools,
            cap,
            &c.inventory,
        ));
        a.inventory.put(Resource::Tools, made);
        self.ledger.regrown[Resource::Tools.index()] += f64::from(made);
        a.stats.tools_crafted += made;
        self.today.tools_crafted += f64::from(made);
        a.goal = Goal::idle(a.pos, tick);
        let (id, pos) = (a.id, a.pos);
        self.events
            .push(tick, Some(id), pos, EventKind::Crafted { amount: made });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::Scenario;

    fn build(toml: &str) -> Simulation {
        let mut c = Config::default();
        c.sim.check_invariants = true;
        Scenario::from_toml_str(toml)
            .expect("scenario parses")
            .build(&c)
            .expect("scenario builds")
    }

    fn fell_back(sim: &Simulation) -> bool {
        sim.events
            .events
            .iter()
            .any(|e| matches!(e.kind, EventKind::JoinFellBackToFounding { .. }))
    }

    /// A homeless agent arrives to join a home that is now full: founding its
    /// own home on that tile beats having none, so it builds next door.
    #[test]
    fn failed_join_falls_back_to_founding_when_it_scores_well() {
        let toml = r#"
name = "join-fallback"
map = """
........
........
"""
[[home]]
at = [3, 1]
[[agent]]
at = [3, 1]
home = 0
[[agent]]
at = [3, 1]
home = 0
[[agent]]
at = [3, 1]
home = 0
[[agent]]
at = [3, 1]
home = 0
[[agent]]
at = [3, 1]
knows = "all"
"#;
        let mut sim = build(toml);
        let pos = Pos::new(3, 1);
        assert_eq!(sim.world.homes[0].residents.len(), 4, "home 0 is full");
        let i = 4;
        assert!(sim.agents[i].home.is_none());

        sim.do_establish(i, pos, Some(0), 0);

        assert!(fell_back(&sim), "fallback event logged");
        assert_eq!(sim.world.homes.len(), 2, "a new home was founded");
        assert_eq!(sim.world.homes[1].pos, pos, "on the same tile");
        assert_eq!(sim.agents[i].home, Some(1));
        assert_eq!(sim.world.homes[0].residents.len(), 4, "home 0 unchanged");
        sim.check_invariants().expect("state stays consistent");
    }

    /// An agent with a good home (storage, plains) arrives to join a home on a
    /// mountain that is now full: founding on the mountain scores worse than
    /// staying, so it keeps its home and re-plans.
    #[test]
    fn failed_join_replans_when_founding_scores_worse_than_staying() {
        let toml = r#"
name = "join-replan"
map = """
.......m
"""
[[home]]
at = [7, 0]
[[home]]
at = [0, 0]
chests = [10.0]
[[agent]]
at = [7, 0]
home = 0
[[agent]]
at = [7, 0]
home = 0
[[agent]]
at = [7, 0]
home = 0
[[agent]]
at = [7, 0]
home = 0
[[agent]]
at = [7, 0]
home = 1
knows = "all"
traits = [0.5, 0.5, 0.0, 0.5]
"#;
        let mut sim = build(toml);
        let pos = Pos::new(7, 0);
        let i = 4;
        assert_eq!(sim.agents[i].home, Some(1));

        sim.do_establish(i, pos, Some(0), 0);

        assert!(!fell_back(&sim), "no fallback");
        assert_eq!(sim.world.homes.len(), 2, "nothing founded");
        assert_eq!(sim.agents[i].home, Some(1), "kept its home");
        assert!(sim.agents[i].home_plan.is_none(), "plan dropped to re-plan");
        assert_eq!(sim.agents[i].goal.kind, GoalKind::Idle);
        sim.check_invariants().expect("state stays consistent");
    }

    // ---- Baselines (review fixes) -------------------------------------

    fn omniscient_config() -> Config {
        let mut c = Config::default();
        c.sim.agent_kind = AgentKind::Omniscient;
        c
    }

    /// A tile holding several homes must not overflow the home pool and evict
    /// the home the agent is standing on.
    #[test]
    fn omniscient_home_pool_keeps_the_nearest_homes() {
        let toml = r#"
name = "homes-on-shared-tiles"
map = """
......
......
"""
[[home]]
at = [1, 0]
[[home]]
at = [2, 0]
[[home]]
at = [2, 0]
[[home]]
at = [2, 0]
[[agent]]
at = [1, 0]
home = 0
"#;
        let mut c = omniscient_config();
        c.memory.home_slots = 2;
        let sc = crate::scenario::Scenario::from_toml_str(toml).expect("parses");
        let sim = sc.build(&c).expect("builds");
        let homes: Vec<HomeId> = sim.agents[0].memory.homes.iter().map(|h| h.home).collect();
        assert_eq!(homes.len(), 2, "pool size respected: {homes:?}");
        assert!(homes.contains(&0), "kept the home it stands on: {homes:?}");
    }

    #[test]
    fn omniscient_observation_is_deterministic_and_respects_pools() {
        let mut sim = Simulation::new(1, omniscient_config());
        sim.run(5 * 24);
        let c = sim.config.clone();
        for i in [0, sim.agents.len() / 2] {
            let mut a = sim.agents[i].clone();
            let mut b = sim.agents[i].clone();
            observe_omniscient(&mut a, &sim.world, &c, sim.tick);
            observe_omniscient(&mut b, &sim.world, &c, sim.tick);
            assert_eq!(a.memory, b.memory, "same state, same memory");
            for res in Resource::ALL {
                assert!(a.memory.resource_count(res) <= AgentMemory::slots(res, &c.memory));
            }
            assert!(a.memory.homes.len() <= c.memory.home_slots as usize);
            assert_eq!(a.memory.known_count() as usize, sim.world.dims.len());
        }
    }

    /// A resource on cooldown after a failed route is skipped, so the pool
    /// fills with other places instead of retrying the unreachable one.
    #[test]
    fn omniscient_skips_resources_on_unreachable_cooldown() {
        let mut c = omniscient_config();
        c.memory.food_slots = 1;
        let mut sim = Simulation::new(1, c.clone());
        sim.run(24);
        let tick = sim.tick;
        let mut a = sim.agents[0].clone();
        observe_omniscient(&mut a, &sim.world, &c, tick);
        let nearest = a
            .memory
            .resources
            .iter()
            .find(|m| m.resource == Resource::Food)
            .unwrap()
            .pos;
        a.memory.mark_unreachable(nearest, tick + 100);
        observe_omniscient(&mut a, &sim.world, &c, tick);
        let food: Vec<_> = a
            .memory
            .resources
            .iter()
            .filter(|m| m.resource == Resource::Food)
            .collect();
        assert_eq!(food.len(), 1, "the pool is refilled");
        assert_ne!(food[0].pos, nearest, "the unreachable place was skipped");
        // A later re-observe inside the window must still skip it (the cooldown
        // lives in failed_targets, not in the replaced resource pool).
        observe_omniscient(&mut a, &sim.world, &c, tick + 6);
        let food: Vec<_> = a
            .memory
            .resources
            .iter()
            .filter(|m| m.resource == Resource::Food)
            .collect();
        assert_ne!(
            food[0].pos, nearest,
            "cooldown must survive a re-observe inside the window"
        );
        // After the cooldown it is offered again.
        observe_omniscient(&mut a, &sim.world, &c, tick + 200);
        let food: Vec<_> = a
            .memory
            .resources
            .iter()
            .filter(|m| m.resource == Resource::Food)
            .collect();
        assert_eq!(food[0].pos, nearest);
    }

    /// Omniscient agents score travel with true path levels, so a road makes
    /// a trip look cheaper to them than to a normal agent who has not seen it.
    #[test]
    fn omniscient_travel_estimates_use_true_roads() {
        let mut sim = Simulation::new(1, Config::default());
        let a = sim.agents[0].clone();
        let from = a.pos;
        let to = Pos::new(from.x + 6, from.y);
        for dx in 1..=6 {
            let p = Pos::new(from.x + dx, from.y);
            if sim.world.contains(p) && sim.world.tile(p).is_land() {
                sim.world.tile_mut(p).path_level = crate::world::PathLevel::Road;
            }
        }
        let normal = decision::travel_ticks(&a, &sim.ctx(0), from, to);
        sim.config.sim.agent_kind = AgentKind::Omniscient;
        let omni = decision::travel_ticks(&a, &sim.ctx(0), from, to);
        assert!(
            omni < normal,
            "omniscient {omni} should see the road (normal {normal})"
        );
        assert_eq!(
            decision::believed_path(&a, &sim.ctx(0), Pos::new(from.x + 1, from.y)),
            sim.world.tile(Pos::new(from.x + 1, from.y)).path_level
        );
    }

    #[test]
    fn omniscient_runs_stay_on_land_and_snapshots_round_trip() {
        let mut c = omniscient_config();
        c.sim.check_invariants = true; // includes "agent in the water"
        let mut sim = Simulation::new(1, c);
        sim.run(10 * 24);
        let mut resumed = Simulation::from_snapshot_json(&sim.snapshot_json()).expect("loads");
        sim.run(48);
        resumed.run(48);
        assert_eq!(resumed.state_hash(), sim.state_hash());
    }

    #[test]
    fn random_baseline_is_deterministic_and_idles_only_last() {
        let mut c = Config::default();
        c.sim.agent_kind = AgentKind::Random;
        let mut a = Simulation::new(2, c.clone());
        let mut b = Simulation::new(2, c);
        a.run(5 * 24);
        b.run(5 * 24);
        assert_eq!(a.state_hash(), b.state_hash());

        let cx = a.ctx(a.tick);
        let cands = decision::evaluate(&a.agents[0], &cx);
        assert!(cands.iter().any(|c| c.kind == GoalKind::Idle));
        let mut rng = Rng::for_stream(9, Stream::Agents);
        for _ in 0..20 {
            let order = random_choice(cands.clone(), &mut rng);
            assert_eq!(order.last().unwrap().kind, GoalKind::Idle, "Idle is last");
            assert_eq!(order.len(), cands.len());
        }
    }
}
