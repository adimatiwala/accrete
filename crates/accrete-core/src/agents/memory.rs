//! Fixed-size agent memory.
//!
//! | Memory | Size | Contents |
//! |---|---|---|
//! | Known tiles | 1 bit per tile | Tiles ever seen; terrain is static, so seeing a tile once reveals its terrain. |
//! | Resource memories | 64 entries | Position, resource, amount seen, tick seen. Partitioned by resource type (food 40, wood 16, stone 8) so plentiful wood never crowds out food. When a pool is full, the entry with the lowest `amount × freshness` is replaced (ties: lowest position). |
//! | Changed tiles | 32 entries | Trails and roads seen on a tile. Oldest replaced. |
//! | Disruptions | 1,024 entries | Route disruptions seen (floods, landslides). Sorted by
//! |  |  | tile for fast lookup; when full, the entry farthest from the agent is replaced.
//! |  |  | Kept apart from trails/roads so a large flood never crowds them out. |
//! | Known homes | 16 entries | Homes seen, their resident count, and whether they had storage. Oldest replaced. |
//!
//! Decisions read only this memory plus what the agent currently sees.

use serde::{Deserialize, Serialize};

use crate::config::MemoryConfig;
use crate::geom::{Dims, Pos};
use crate::hash::{StableHash, StableHasher};
use crate::inventory::Resource;
use crate::world::{Disruption, HomeId, PathLevel};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResourceMemory {
    pub pos: Pos,
    pub resource: Resource,
    pub amount: f32,
    pub seen_tick: u64,
    /// The last route to this place failed; ignore it until this tick.
    pub unreachable_until: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HomeMemory {
    pub home: HomeId,
    pub pos: Pos,
    pub residents: u32,
    pub has_storage: bool,
    /// Comfortable capacity as last seen (the shelter level is visible).
    pub comfortable: u32,
    /// Homes standing on the same tile as last seen.
    pub tile_homes: u32,
    pub seen_tick: u64,
}

/// What an agent sees of a home when the home's tile is in sight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HomeSighting {
    pub residents: u32,
    pub has_storage: bool,
    pub comfortable: u32,
    pub tile_homes: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChangedTile {
    pub pos: Pos,
    pub path_level: PathLevel,
    pub seen_tick: u64,
    #[serde(default)]
    pub traffic: f32,
}

/// A route disruption the agent last saw on a tile (flood, landslide).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeenDisruption {
    pub pos: Pos,
    pub disruption: Disruption,
    pub seen_tick: u64,
}

/// Someone this agent has met, and how fairly they have traded with it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SocialMemory {
    pub agent: u32,
    /// Reciprocity in [-1, 1]; 0 is neutral.
    pub score: f32,
    pub trades: u32,
    pub last_met: u64,
}

/// A tile the agent crosses often, with a count that decays over ~30 days.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FrequentTile {
    pub pos: Pos,
    pub count: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentMemory {
    width: u32,
    known: Vec<u64>,
    known_count: u32,
    pub resources: Vec<ResourceMemory>,
    pub homes: Vec<HomeMemory>,
    pub changed: Vec<ChangedTile>,
    /// Route disruptions, kept sorted by `pos` so lookups are a binary search.
    pub disruptions: Vec<SeenDisruption>,
    /// Food in the agent's own home chests when it last looked.
    pub home_food_seen: f32,
    /// Free chest weight at home when it last looked.
    pub home_room_seen: f32,
    /// Explore targets whose route failed, with the tick they expire.
    pub failed_targets: Vec<(Pos, u64)>,
    /// Agents met (32 slots by default).
    pub social: Vec<SocialMemory>,
    /// Most-crossed tiles (16 slots by default).
    pub frequent: Vec<FrequentTile>,
    /// Believed exchange rate, food per tool (A.26); 0 until the agent has
    /// traded tools or heard someone's rate.
    #[serde(default)]
    pub tool_price: f32,
    /// Places where trading went well (A.33), best first is not guaranteed.
    #[serde(default)]
    pub trade_spots: Vec<TradeSpot>,
}

/// A remembered trading place: a score that grows with each trade there
/// (or with what others say about it) and fades daily.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TradeSpot {
    pub pos: Pos,
    pub score: f32,
    pub seen_tick: u64,
}

/// Freshness in `(0, 1]`: `half_life / (half_life + age)`. Pure division, so
/// it is exactly reproducible.
#[inline]
pub fn freshness(seen_tick: u64, now: u64, half_life_ticks: f32) -> f32 {
    let age = now.saturating_sub(seen_tick) as f32;
    half_life_ticks / (half_life_ticks + age)
}

impl AgentMemory {
    pub fn new(dims: Dims) -> Self {
        Self {
            width: dims.width,
            known: vec![0; dims.len().div_ceil(64)],
            known_count: 0,
            resources: Vec::new(),
            homes: Vec::new(),
            changed: Vec::new(),
            disruptions: Vec::new(),
            home_food_seen: 0.0,
            home_room_seen: 0.0,
            failed_targets: Vec::new(),
            social: Vec::new(),
            frequent: Vec::new(),
            tool_price: 0.0,
            trade_spots: Vec::new(),
        }
    }

    /// Note meeting `other`; returns its index in `social`. When full, the
    /// least recently met (ties: lowest id) is forgotten.
    pub fn meet(&mut self, other: u32, tick: u64, cap: usize) -> Option<usize> {
        if cap == 0 {
            return None;
        }
        if let Some(i) = self.social.iter().position(|s| s.agent == other) {
            self.social[i].last_met = tick;
            return Some(i);
        }
        let e = SocialMemory {
            agent: other,
            score: 0.0,
            trades: 0,
            last_met: tick,
        };
        if self.social.len() < cap {
            self.social.push(e);
            return Some(self.social.len() - 1);
        }
        let (wi, _) = self
            .social
            .iter()
            .enumerate()
            .min_by(|a, b| {
                a.1.last_met
                    .cmp(&b.1.last_met)
                    .then(a.1.agent.cmp(&b.1.agent))
            })
            .expect("non-empty");
        self.social[wi] = e;
        Some(wi)
    }

    pub fn reciprocity(&self, other: u32) -> f32 {
        self.social
            .iter()
            .find(|s| s.agent == other)
            .map_or(0.0, |s| s.score)
    }

    /// Count a crossing of `p` in the frequent-tile list (weakest replaced).
    pub fn cross(&mut self, p: Pos, cap: usize) {
        if cap == 0 {
            return;
        }
        if let Some(f) = self.frequent.iter_mut().find(|f| f.pos == p) {
            f.count += 1.0;
            return;
        }
        let e = FrequentTile { pos: p, count: 1.0 };
        if self.frequent.len() < cap {
            self.frequent.push(e);
            return;
        }
        let (wi, wv) = self
            .frequent
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.count.total_cmp(&b.1.count).then(a.1.pos.cmp(&b.1.pos)))
            .map(|(i, f)| (i, f.count))
            .expect("non-empty");
        if wv <= 1.0 {
            self.frequent[wi] = e;
        }
    }

    pub fn crossings_at(&self, p: Pos) -> f32 {
        self.frequent
            .iter()
            .find(|f| f.pos == p)
            .map_or(0.0, |f| f.count)
    }

    /// Daily decay of social scores toward neutral and of crossing counts.
    pub fn daily_decay(&mut self, reciprocity_decay: f32, crossing_keep: f32) {
        for s in self.social.iter_mut() {
            s.score *= 1.0 - reciprocity_decay;
        }
        for f in self.frequent.iter_mut() {
            f.count *= crossing_keep;
        }
        self.frequent.retain(|f| f.count >= 0.05);
    }

    #[inline]
    fn bit(&self, p: Pos) -> usize {
        (p.y as u32 * self.width + p.x as u32) as usize
    }

    #[inline]
    pub fn knows(&self, p: Pos) -> bool {
        let b = self.bit(p);
        (self.known[b >> 6] >> (b & 63)) & 1 == 1
    }

    #[inline]
    pub fn learn(&mut self, p: Pos) {
        let b = self.bit(p);
        let w = &mut self.known[b >> 6];
        let m = 1u64 << (b & 63);
        if *w & m == 0 {
            *w |= m;
            self.known_count += 1;
        }
    }

    pub fn known_count(&self) -> u32 {
        self.known_count
    }

    pub fn slots(r: Resource, c: &MemoryConfig) -> usize {
        match r {
            Resource::Food => c.food_slots as usize,
            Resource::Wood => c.wood_slots as usize,
            Resource::Stone => c.stone_slots as usize,
            Resource::Tools => 0,
        }
    }

    /// Replace all resource memories at once (omniscient baseline only).
    /// `entries` must already respect the per-type pool sizes; an entry for a
    /// place the agent recently failed to reach keeps that cooldown.
    pub fn set_resources(&mut self, mut entries: Vec<ResourceMemory>) {
        for e in entries.iter_mut() {
            if let Some(old) = self
                .resources
                .iter()
                .find(|m| m.pos == e.pos && m.resource == e.resource)
            {
                e.unreachable_until = old.unreachable_until;
            }
        }
        self.resources = entries;
    }

    /// Record an observation of `amount` of `r` at `p`.
    pub fn note_resource(
        &mut self,
        p: Pos,
        r: Resource,
        amount: f32,
        tick: u64,
        c: &MemoryConfig,
        half_life: f32,
    ) {
        if let Some(i) = self
            .resources
            .iter()
            .position(|m| m.pos == p && m.resource == r)
        {
            if amount < c.min_amount {
                self.resources.remove(i);
            } else {
                let m = &mut self.resources[i];
                m.amount = amount;
                m.seen_tick = tick;
            }
            return;
        }
        if amount < c.min_amount {
            return;
        }
        let cap = Self::slots(r, c);
        if cap == 0 {
            return;
        }
        let entry = ResourceMemory {
            pos: p,
            resource: r,
            amount,
            seen_tick: tick,
            unreachable_until: 0,
        };
        let count = self.resources.iter().filter(|m| m.resource == r).count();
        if count < cap {
            self.resources.push(entry);
            return;
        }
        // Replace the weakest memory of this type if the new one is stronger.
        let mut worst: Option<(usize, f32)> = None;
        for (i, m) in self.resources.iter().enumerate() {
            if m.resource != r {
                continue;
            }
            let v = m.amount * freshness(m.seen_tick, tick, half_life);
            let better = match worst {
                None => true,
                Some((wi, wv)) => v < wv || (v == wv && m.pos < self.resources[wi].pos),
            };
            if better {
                worst = Some((i, v));
            }
        }
        if let Some((wi, wv)) = worst {
            if amount > wv {
                self.resources[wi] = entry;
            }
        }
    }

    /// Record a whole observation at once: `seen` lists each visible tile and
    /// the food, wood and stone amounts on it, in a fixed order. Equivalent to
    /// calling [`Self::note_resource`] for every tile and resource in order, but
    /// O(visible + memory) instead of O(visible × memory).
    pub fn observe_resources(
        &mut self,
        center: Pos,
        seen: &[(Pos, [f32; 3])],
        tick: u64,
        c: &MemoryConfig,
        half_life: f32,
    ) {
        const SIDE: i32 = 2 * crate::world::visibility::SIGHT_RADIUS + 1;
        const R: i32 = crate::world::visibility::SIGHT_RADIUS;
        let local = |p: Pos| -> Option<usize> {
            let dx = p.x - center.x + R;
            let dy = p.y - center.y + R;
            ((0..SIDE).contains(&dx) && (0..SIDE).contains(&dy)).then(|| (dy * SIDE + dx) as usize)
        };
        // Index of existing entries inside the sight square, per resource.
        let mut index = [[u8::MAX; (SIDE * SIDE) as usize]; 3];
        let mut count = [0usize; 3];
        for (i, m) in self.resources.iter().enumerate() {
            count[m.resource.index()] += 1;
            if let Some(l) = local(m.pos) {
                index[m.resource.index()][l] = i as u8;
            }
        }
        let caps = [
            Self::slots(Resource::Food, c),
            Self::slots(Resource::Wood, c),
            Self::slots(Resource::Stone, c),
        ];
        let mut removed = false;
        // Cached weakest entry per resource type (index, value); None = stale.
        let mut weakest: [Option<(usize, f32)>; 3] = [None; 3];
        for &(p, amounts) in seen {
            let Some(l) = local(p) else { continue };
            for res in Resource::ALL {
                let k = res.index();
                let amount = amounts[k];
                let existing = index[k][l];
                if existing != u8::MAX {
                    let m = &mut self.resources[existing as usize];
                    if amount < c.min_amount {
                        m.amount = -1.0; // tombstone, compacted below
                        index[k][l] = u8::MAX;
                        count[k] -= 1;
                        removed = true;
                    } else {
                        m.amount = amount;
                        m.seen_tick = tick;
                    }
                    weakest[k] = None;
                    continue;
                }
                if amount < c.min_amount || caps[k] == 0 {
                    continue;
                }
                let entry = ResourceMemory {
                    pos: p,
                    resource: res,
                    amount,
                    seen_tick: tick,
                    unreachable_until: 0,
                };
                if count[k] < caps[k] {
                    index[k][l] = self.resources.len() as u8;
                    self.resources.push(entry);
                    count[k] += 1;
                    weakest[k] = None;
                    continue;
                }
                if weakest[k].is_none() {
                    let mut w: Option<(usize, f32)> = None;
                    for (i, m) in self.resources.iter().enumerate() {
                        if m.resource != res || m.amount < 0.0 {
                            continue;
                        }
                        let v = m.amount * freshness(m.seen_tick, tick, half_life);
                        let better = match w {
                            None => true,
                            Some((wi, wv)) => v < wv || (v == wv && m.pos < self.resources[wi].pos),
                        };
                        if better {
                            w = Some((i, v));
                        }
                    }
                    weakest[k] = w;
                }
                if let Some((wi, wv)) = weakest[k] {
                    if amount > wv {
                        if let Some(ol) = local(self.resources[wi].pos) {
                            index[k][ol] = u8::MAX;
                        }
                        self.resources[wi] = entry;
                        index[k][l] = wi as u8;
                        weakest[k] = None;
                    }
                }
            }
        }
        if removed {
            self.resources.retain(|m| m.amount >= 0.0);
        }
    }

    /// Learn about a resource from someone else: keeps the teller's sighting
    /// time, so it never knows anything fresher than the teller did. Returns
    /// true if the memory changed.
    pub fn hear_about(
        &mut self,
        m: &ResourceMemory,
        tick: u64,
        c: &MemoryConfig,
        half_life: f32,
    ) -> bool {
        if let Some(e) = self
            .resources
            .iter_mut()
            .find(|e| e.pos == m.pos && e.resource == m.resource)
        {
            if e.seen_tick >= m.seen_tick {
                return false;
            }
            e.amount = m.amount;
            e.seen_tick = m.seen_tick;
            return true;
        }
        if m.amount < c.min_amount {
            return false;
        }
        let cap = Self::slots(m.resource, c);
        if cap == 0 {
            return false;
        }
        let entry = ResourceMemory {
            unreachable_until: 0,
            ..m.clone()
        };
        let value = m.amount * freshness(m.seen_tick, tick, half_life);
        if self.resource_count(m.resource) < cap {
            self.resources.push(entry);
        } else {
            let mut worst: Option<(usize, f32)> = None;
            for (i, e) in self.resources.iter().enumerate() {
                if e.resource != m.resource {
                    continue;
                }
                let v = e.amount * freshness(e.seen_tick, tick, half_life);
                if worst
                    .is_none_or(|(wi, wv)| v < wv || (v == wv && e.pos < self.resources[wi].pos))
                {
                    worst = Some((i, v));
                }
            }
            match worst {
                Some((wi, wv)) if value > wv => self.resources[wi] = entry,
                _ => return false,
            }
        }
        self.learn(m.pos);
        true
    }

    /// Forget all resource memories except those at `keep` (used when memory is
    /// disabled: the agent only "remembers" what it currently sees).
    pub fn forget_resources_not_in(&mut self, visible: impl Fn(Pos) -> bool) {
        self.resources.retain(|m| visible(m.pos));
    }

    pub fn note_home(&mut self, home: HomeId, pos: Pos, seen: HomeSighting, tick: u64, cap: usize) {
        if let Some(h) = self.homes.iter_mut().find(|h| h.home == home) {
            h.residents = seen.residents;
            h.has_storage = seen.has_storage;
            h.comfortable = seen.comfortable;
            h.tile_homes = seen.tile_homes;
            h.seen_tick = tick;
            h.pos = pos;
            return;
        }
        let e = HomeMemory {
            home,
            pos,
            residents: seen.residents,
            has_storage: seen.has_storage,
            comfortable: seen.comfortable,
            tile_homes: seen.tile_homes,
            seen_tick: tick,
        };
        if self.homes.len() < cap {
            self.homes.push(e);
        } else if cap > 0 {
            // Replace the oldest (ties: lowest home id).
            let (wi, _) = self
                .homes
                .iter()
                .enumerate()
                .min_by(|a, b| {
                    a.1.seen_tick
                        .cmp(&b.1.seen_tick)
                        .then(a.1.home.cmp(&b.1.home))
                })
                .expect("non-empty");
            self.homes[wi] = e;
        }
    }

    /// How many homes the agent believes stand on `p`: the latest count it
    /// saw there, or 0 if it remembers no home on that tile.
    pub fn believed_homes_on(&self, p: Pos) -> u32 {
        self.homes
            .iter()
            .filter(|h| h.pos == p)
            .max_by_key(|h| (h.seen_tick, h.home))
            .map_or(0, |h| h.tile_homes)
    }

    pub fn forget_home(&mut self, home: HomeId) {
        self.homes.retain(|h| h.home != home);
    }

    /// Record a trail or road the agent sees. A tile that looks bare again is
    /// forgotten. Disruptions use [`note_disruption`], so a flood never
    /// crowds roads out of this pool.
    pub fn note_changed(
        &mut self,
        pos: Pos,
        level: PathLevel,
        traffic: f32,
        tick: u64,
        cap: usize,
    ) {
        if let Some(i) = self.changed.iter().position(|c| c.pos == pos) {
            if level == PathLevel::None {
                self.changed.remove(i);
            } else {
                self.changed[i].path_level = level;
                self.changed[i].seen_tick = tick;
                self.changed[i].traffic = traffic;
            }
            return;
        }
        if level == PathLevel::None || cap == 0 {
            return;
        }
        let e = ChangedTile {
            pos,
            path_level: level,
            seen_tick: tick,
            traffic,
        };
        if self.changed.len() < cap {
            self.changed.push(e);
        } else {
            let (wi, _) = self
                .changed
                .iter()
                .enumerate()
                .min_by(|a, b| {
                    a.1.seen_tick
                        .cmp(&b.1.seen_tick)
                        .then(a.1.pos.cmp(&b.1.pos))
                })
                .expect("non-empty");
            self.changed[wi] = e;
        }
    }

    /// Record a route disruption the agent sees. Cleared when the tile looks
    /// normal again. Kept sorted by position. When full, the entry farthest
    /// from `at` (the agent's position) is replaced, ties broken by position.
    pub fn note_disruption(
        &mut self,
        pos: Pos,
        disruption: Disruption,
        tick: u64,
        at: Pos,
        cap: usize,
    ) {
        match self.disruptions.binary_search_by_key(&pos, |d| d.pos) {
            Ok(i) => {
                if disruption == Disruption::None {
                    self.disruptions.remove(i);
                } else {
                    self.disruptions[i].disruption = disruption;
                    self.disruptions[i].seen_tick = tick;
                }
            }
            Err(i) => {
                if disruption == Disruption::None || cap == 0 {
                    return;
                }
                let e = SeenDisruption {
                    pos,
                    disruption,
                    seen_tick: tick,
                };
                if self.disruptions.len() < cap {
                    self.disruptions.insert(i, e);
                } else {
                    let (wi, far) = self
                        .disruptions
                        .iter()
                        .enumerate()
                        .max_by(|a, b| {
                            a.1.pos
                                .dist2(at)
                                .cmp(&b.1.pos.dist2(at))
                                .then(a.1.pos.cmp(&b.1.pos))
                        })
                        .map(|(i, d)| (i, d.pos))
                        .expect("non-empty");
                    // Keep the nearest set: a farther (or equal, tied by pos)
                    // sighting is ignored.
                    let new_farther = pos
                        .dist2(at)
                        .cmp(&far.dist2(at))
                        .then(pos.cmp(&far))
                        .is_ge();
                    if new_farther {
                        return;
                    }
                    self.disruptions.remove(wi);
                    let i = self
                        .disruptions
                        .binary_search_by_key(&pos, |d| d.pos)
                        .unwrap_err();
                    self.disruptions.insert(i, e);
                }
            }
        }
    }

    pub fn believed_path(&self, p: Pos) -> PathLevel {
        self.changed
            .iter()
            .find(|c| c.pos == p)
            .map_or(PathLevel::None, |c| c.path_level)
    }

    /// Wear on a trail as last observed.
    pub fn believed_traffic(&self, p: Pos) -> f32 {
        self.changed
            .iter()
            .find(|c| c.pos == p)
            .map_or(0.0, |c| c.traffic)
    }

    /// The route disruption the agent last saw on `p`.
    pub fn believed_disruption(&self, p: Pos) -> Disruption {
        self.disruptions
            .binary_search_by_key(&p, |d| d.pos)
            .ok()
            .map_or(Disruption::None, |i| self.disruptions[i].disruption)
    }

    /// Drop the farthest entries until the pool fits `cap` (e.g. after
    /// `set_config` lowers `memory.disruption_slots`).
    pub fn trim_disruptions(&mut self, at: Pos, cap: usize) {
        while self.disruptions.len() > cap {
            let wi = self
                .disruptions
                .iter()
                .enumerate()
                .max_by(|a, b| {
                    a.1.pos
                        .dist2(at)
                        .cmp(&b.1.pos.dist2(at))
                        .then(a.1.pos.cmp(&b.1.pos))
                })
                .map(|(i, _)| i)
                .expect("non-empty");
            self.disruptions.remove(wi);
        }
    }

    pub fn mark_unreachable(&mut self, p: Pos, until: u64) {
        for m in self.resources.iter_mut().filter(|m| m.pos == p) {
            m.unreachable_until = until;
        }
        if !self.failed_targets.iter().any(|(q, _)| *q == p) {
            if self.failed_targets.len() >= 8 {
                self.failed_targets.remove(0);
            }
            self.failed_targets.push((p, until));
        }
    }

    pub fn is_failed_target(&self, p: Pos, now: u64) -> bool {
        self.failed_targets
            .iter()
            .any(|&(q, until)| q == p && now < until)
    }

    pub fn expire(&mut self, now: u64) {
        self.failed_targets.retain(|&(_, until)| now < until);
    }

    /// Inherit memories for a newborn: the parent's best `share` of resource
    /// memories (by `amount × freshness`, keeping their timestamps) and the
    /// known tiles within `radius` of `home`. Never more than the parent knew.
    pub fn inherit_from(
        parent: &AgentMemory,
        dims: Dims,
        home: Pos,
        radius: i32,
        share: f32,
        now: u64,
        half_life: f32,
    ) -> Self {
        let mut m = AgentMemory::new(dims);
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let p = home.offset(dx, dy);
                if dims.contains(p) && parent.knows(p) {
                    m.learn(p);
                }
            }
        }
        let mut ranked: Vec<&ResourceMemory> = parent.resources.iter().collect();
        ranked.sort_by(|a, b| {
            let va = a.amount * freshness(a.seen_tick, now, half_life);
            let vb = b.amount * freshness(b.seen_tick, now, half_life);
            vb.total_cmp(&va)
                .then(a.pos.cmp(&b.pos))
                .then(a.resource.cmp(&b.resource))
        });
        let keep = ((ranked.len() as f32) * share) as usize;
        for r in ranked.into_iter().take(keep) {
            let mut e = r.clone();
            e.unreachable_until = 0;
            m.resources.push(e);
            // Knowing a resource location implies knowing that tile.
            m.learn(r.pos);
        }
        m.homes = parent.homes.clone();
        m.changed = parent.changed.clone();
        m.disruptions = parent.disruptions.clone();
        m.home_food_seen = parent.home_food_seen;
        m.home_room_seen = parent.home_room_seen;
        m.tool_price = parent.tool_price;
        m.trade_spots = parent.trade_spots.clone();
        m
    }

    pub fn resource_count(&self, r: Resource) -> usize {
        self.resources.iter().filter(|m| m.resource == r).count()
    }
}

impl StableHash for AgentMemory {
    fn stable_hash(&self, h: &mut StableHasher) {
        h.u32(self.known_count);
        for &w in &self.known {
            h.u64(w);
        }
        h.len(self.resources.len());
        for m in &self.resources {
            m.pos.stable_hash(h);
            h.u8(m.resource as u8);
            h.f32(m.amount);
            h.u64(m.seen_tick);
            h.u64(m.unreachable_until);
        }
        h.len(self.homes.len());
        for m in &self.homes {
            h.u32(m.home);
            m.pos.stable_hash(h);
            h.u32(m.residents);
            h.bool(m.has_storage);
            h.u32(m.comfortable);
            h.u32(m.tile_homes);
            h.u64(m.seen_tick);
        }
        h.len(self.changed.len());
        for c in &self.changed {
            c.pos.stable_hash(h);
            h.u8(c.path_level as u8);
            h.u64(c.seen_tick);
            h.f32(c.traffic);
        }
        h.len(self.disruptions.len());
        for d in &self.disruptions {
            d.pos.stable_hash(h);
            h.u8(d.disruption.code());
            h.u64(d.seen_tick);
        }
        h.f32(self.home_food_seen);
        h.f32(self.home_room_seen);
        h.len(self.failed_targets.len());
        for (p, t) in &self.failed_targets {
            p.stable_hash(h);
            h.u64(*t);
        }
        h.len(self.social.len());
        for s in &self.social {
            h.u32(s.agent);
            h.f32(s.score);
            h.u32(s.trades);
            h.u64(s.last_met);
        }
        h.len(self.frequent.len());
        for f in &self.frequent {
            f.pos.stable_hash(h);
            h.f32(f.count);
        }
        h.f32(self.tool_price);
        h.len(self.trade_spots.len());
        for s in &self.trade_spots {
            s.pos.stable_hash(h);
            h.f32(s.score);
            h.u64(s.seen_tick);
        }
    }
}

impl AgentMemory {
    /// A trade happened at `pos`: strengthen (or remember) that place.
    pub fn note_trade_spot(&mut self, pos: Pos, add: f32, tick: u64, slots: usize) {
        if let Some(s) = self.trade_spots.iter_mut().find(|s| s.pos == pos) {
            s.score += add;
            s.seen_tick = tick;
            return;
        }
        self.trade_spots.push(TradeSpot {
            pos,
            score: add,
            seen_tick: tick,
        });
        self.trim_trade_spots(slots);
    }

    /// Someone spoke well of `pos`: remember it at least that well.
    pub fn hear_trade_spot(&mut self, pos: Pos, score: f32, tick: u64, slots: usize) {
        if let Some(s) = self.trade_spots.iter_mut().find(|s| s.pos == pos) {
            s.score = s.score.max(score);
            return;
        }
        self.trade_spots.push(TradeSpot {
            pos,
            score,
            seen_tick: tick,
        });
        self.trim_trade_spots(slots);
    }

    fn trim_trade_spots(&mut self, slots: usize) {
        while self.trade_spots.len() > slots {
            let worst = self
                .trade_spots
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| a.score.total_cmp(&b.score).then(b.pos.cmp(&a.pos)))
                .map(|(i, _)| i)
                .expect("non-empty");
            self.trade_spots.remove(worst);
        }
    }

    /// Daily fading of trading places; forgotten below a small score.
    pub fn decay_trade_spots(&mut self, keep: f32) {
        for s in &mut self.trade_spots {
            s.score *= keep;
        }
        self.trade_spots.retain(|s| s.score >= 0.05);
    }

    /// The best remembered trading place.
    pub fn best_trade_spot(&self) -> Option<&TradeSpot> {
        self.trade_spots
            .iter()
            .max_by(|a, b| a.score.total_cmp(&b.score).then(b.pos.cmp(&a.pos)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> MemoryConfig {
        MemoryConfig {
            food_slots: 3,
            ..MemoryConfig::default()
        }
    }

    #[test]
    fn bounded_and_replaces_weakest() {
        let c = cfg();
        let mut m = AgentMemory::new(Dims::new(16, 16));
        for (i, amt) in [5.0, 2.0, 8.0, 1.5, 9.0].iter().enumerate() {
            m.note_resource(Pos::new(i as i32, 0), Resource::Food, *amt, 0, &c, 120.0);
            assert!(m.resource_count(Resource::Food) <= 3);
        }
        let mut amounts: Vec<f32> = m.resources.iter().map(|r| r.amount).collect();
        amounts.sort_by(f32::total_cmp);
        assert_eq!(amounts, vec![5.0, 8.0, 9.0]);
    }

    #[test]
    fn depleted_observation_forgets() {
        let c = cfg();
        let mut m = AgentMemory::new(Dims::new(8, 8));
        m.note_resource(Pos::new(1, 1), Resource::Food, 4.0, 0, &c, 120.0);
        m.note_resource(Pos::new(1, 1), Resource::Food, 0.2, 5, &c, 120.0);
        assert_eq!(m.resource_count(Resource::Food), 0);
    }

    #[test]
    fn known_bits() {
        let mut m = AgentMemory::new(Dims::new(70, 3));
        let p = Pos::new(69, 2);
        assert!(!m.knows(p));
        m.learn(p);
        m.learn(p);
        assert!(m.knows(p));
        assert_eq!(m.known_count(), 1);
    }

    #[test]
    fn inheritance_is_subset() {
        let c = MemoryConfig::default();
        let dims = Dims::new(40, 40);
        let mut parent = AgentMemory::new(dims);
        for i in 0..10 {
            let p = Pos::new(i * 3, i);
            parent.learn(p);
            parent.note_resource(p, Resource::Food, 1.0 + i as f32, 0, &c, 120.0);
        }
        // Disruptions outside the newborn known radius are still inherited.
        parent.note_disruption(Pos::new(39, 39), Disruption::Blocked, 5, Pos::new(0, 0), 16);
        parent.note_disruption(
            Pos::new(1, 2),
            Disruption::Slowed { factor: 3 },
            6,
            Pos::new(0, 0),
            16,
        );
        let child = AgentMemory::inherit_from(&parent, dims, Pos::new(10, 5), 16, 0.5, 10, 120.0);
        assert_eq!(child.resources.len(), 5);
        for r in &child.resources {
            assert!(
                parent
                    .resources
                    .iter()
                    .any(|p| p.pos == r.pos && p.seen_tick == r.seen_tick)
            );
            assert!(parent.knows(r.pos));
        }
        for i in 0..dims.len() as u32 {
            let p = dims.pos(i);
            assert!(!child.knows(p) || parent.knows(p));
        }
        assert_eq!(child.disruptions, parent.disruptions);
        assert_eq!(
            child.believed_disruption(Pos::new(39, 39)),
            Disruption::Blocked
        );
    }

    #[test]
    fn floods_do_not_crowd_out_roads() {
        let mut m = AgentMemory::new(Dims::new(32, 32));
        let road = Pos::new(1, 1);
        m.note_changed(road, PathLevel::Road, 0.0, 0, 32);
        assert_eq!(m.believed_path(road), PathLevel::Road);
        // More disruptions than the old shared pool of 32.
        for i in 0..80 {
            m.note_disruption(
                Pos::new(i % 16, i / 16 + 2),
                Disruption::Blocked,
                i as u64,
                Pos::new(8, 8),
                1024,
            );
        }
        assert_eq!(m.disruptions.len(), 80);
        assert_eq!(m.believed_path(road), PathLevel::Road);
        assert_eq!(m.changed.len(), 1);
    }

    #[test]
    fn disruption_pool_drops_the_farthest_when_full() {
        let mut m = AgentMemory::new(Dims::new(64, 64));
        let at = Pos::new(0, 0);
        for i in 1..=4 {
            m.note_disruption(Pos::new(i, 0), Disruption::Blocked, i as u64, at, 3);
        }
        assert_eq!(m.disruptions.len(), 3);
        assert_eq!(m.believed_disruption(Pos::new(4, 0)), Disruption::None);
        assert_eq!(m.believed_disruption(Pos::new(1, 0)), Disruption::Blocked);
        assert_eq!(m.believed_disruption(Pos::new(2, 0)), Disruption::Blocked);
        assert_eq!(m.believed_disruption(Pos::new(3, 0)), Disruption::Blocked);
        // Restoring a tile frees its slot.
        m.note_disruption(Pos::new(1, 0), Disruption::None, 10, at, 3);
        assert_eq!(m.believed_disruption(Pos::new(1, 0)), Disruption::None);
        assert_eq!(m.disruptions.len(), 2);
    }

    #[test]
    fn disruption_memory_stays_sorted() {
        let mut m = AgentMemory::new(Dims::new(16, 16));
        for &(x, y) in &[(5, 5), (1, 1), (3, 0), (2, 8), (1, 0)] {
            m.note_disruption(Pos::new(x, y), Disruption::Blocked, 0, Pos::new(0, 0), 16);
        }
        assert!(m.disruptions.windows(2).all(|w| w[0].pos < w[1].pos));
        assert_eq!(m.believed_disruption(Pos::new(3, 0)), Disruption::Blocked);
    }

    #[test]
    fn reseeing_a_restored_tile_clears_the_belief() {
        let mut m = AgentMemory::new(Dims::new(8, 8));
        let p = Pos::new(2, 2);
        m.note_disruption(p, Disruption::Blocked, 0, Pos::new(0, 0), 8);
        assert_eq!(m.believed_disruption(p), Disruption::Blocked);
        m.note_disruption(p, Disruption::None, 10, Pos::new(0, 0), 8);
        assert_eq!(m.believed_disruption(p), Disruption::None);
        assert!(m.disruptions.is_empty());
    }

    #[test]
    fn equal_distance_prefers_the_lower_pos() {
        let mut m = AgentMemory::new(Dims::new(16, 16));
        let at = Pos::new(0, 0);
        m.note_disruption(Pos::new(3, 4), Disruption::Blocked, 0, at, 1); // dist2 = 25
        // Same distance, higher pos: ignored when the pool is full.
        m.note_disruption(Pos::new(4, 3), Disruption::Blocked, 1, at, 1);
        assert_eq!(m.believed_disruption(Pos::new(3, 4)), Disruption::Blocked);
        assert_eq!(m.believed_disruption(Pos::new(4, 3)), Disruption::None);
        // Same distance, lower pos: replaces the higher one.
        m.note_disruption(Pos::new(0, 5), Disruption::Blocked, 2, at, 1);
        assert_eq!(m.believed_disruption(Pos::new(0, 5)), Disruption::Blocked);
        assert_eq!(m.believed_disruption(Pos::new(3, 4)), Disruption::None);
    }
}
