//! Fixed-size agent memory.
//!
//! | Memory | Size | Contents |
//! |---|---|---|
//! | Known tiles | 1 bit per tile | Tiles ever seen; terrain is static, so seeing a tile once reveals its terrain. |
//! | Resource memories | 64 entries | Position, resource, amount seen, tick seen. Partitioned by resource type (food 40, wood 16, stone 8) so plentiful wood never crowds out food. When a pool is full, the entry with the lowest `amount × freshness` is replaced (ties: lowest position). |
//! | Changed tiles | 32 entries | Tiles seen to differ from bare terrain (trails, roads). |
//! | Known homes | 16 entries | Homes seen, their resident count, and whether they had storage. Oldest replaced. |
//!
//! Decisions read only this memory plus what the agent currently sees.

use serde::{Deserialize, Serialize};

use crate::config::MemoryConfig;
use crate::geom::{Dims, Pos};
use crate::hash::{StableHash, StableHasher};
use crate::inventory::Resource;
use crate::world::{HomeId, PathLevel};

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
    pub seen_tick: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChangedTile {
    pub pos: Pos,
    pub path_level: PathLevel,
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
            home_food_seen: 0.0,
            home_room_seen: 0.0,
            failed_targets: Vec::new(),
            social: Vec::new(),
            frequent: Vec::new(),
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

    fn slots(r: Resource, c: &MemoryConfig) -> usize {
        match r {
            Resource::Food => c.food_slots as usize,
            Resource::Wood => c.wood_slots as usize,
            Resource::Stone => c.stone_slots as usize,
        }
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

    pub fn note_home(
        &mut self,
        home: HomeId,
        pos: Pos,
        residents: u32,
        has_storage: bool,
        tick: u64,
        cap: usize,
    ) {
        if let Some(h) = self.homes.iter_mut().find(|h| h.home == home) {
            h.residents = residents;
            h.has_storage = has_storage;
            h.seen_tick = tick;
            h.pos = pos;
            return;
        }
        let e = HomeMemory {
            home,
            pos,
            residents,
            has_storage,
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

    pub fn forget_home(&mut self, home: HomeId) {
        self.homes.retain(|h| h.home != home);
    }

    pub fn note_changed(&mut self, pos: Pos, level: PathLevel, tick: u64, cap: usize) {
        if let Some(i) = self.changed.iter().position(|c| c.pos == pos) {
            if level == PathLevel::None {
                self.changed.remove(i);
            } else {
                self.changed[i].path_level = level;
                self.changed[i].seen_tick = tick;
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

    pub fn believed_path(&self, p: Pos) -> PathLevel {
        self.changed
            .iter()
            .find(|c| c.pos == p)
            .map_or(PathLevel::None, |c| c.path_level)
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
        m.home_food_seen = parent.home_food_seen;
        m.home_room_seen = parent.home_room_seen;
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
            h.u64(m.seen_tick);
        }
        h.len(self.changed.len());
        for c in &self.changed {
            c.pos.stable_hash(h);
            h.u8(c.path_level as u8);
            h.u64(c.seen_tick);
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
    }
}
