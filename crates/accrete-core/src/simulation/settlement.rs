//! Settlement detection — **measurement only**. Nothing here is read by agents.
//!
//! Once per day, occupied homes are grouped by single linkage: two homes are
//! in the same group when they are within `radius` tiles (Chebyshev) of each
//! other, directly or through a chain. A group with at least `min_homes`
//! occupied homes and `min_residents` residents is a *candidate*; a candidate
//! that persists for `persist_days` consecutive days is a *settlement*. A
//! tracked group keeps its identity while at least half of its homes continue
//! into one of today's groups.

use serde::{Deserialize, Serialize};

use crate::config::SettlementConfig;
use crate::geom::Pos;
use crate::world::{HomeId, World};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SizeClass {
    Candidate,
    Hamlet,
    Village,
    Town,
}

impl SizeClass {
    pub fn name(self) -> &'static str {
        match self {
            SizeClass::Candidate => "candidate",
            SizeClass::Hamlet => "hamlet",
            SizeClass::Village => "village",
            SizeClass::Town => "town",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tracked {
    pub id: u32,
    pub homes: Vec<HomeId>,
    pub residents: u32,
    pub centre: (f32, f32),
    pub since_day: u32,
    pub class: SizeClass,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SettlementTracker {
    pub next_id: u32,
    pub tracked: Vec<Tracked>,
    /// Settlements ever confirmed (ids), for persistence statistics.
    pub ever_confirmed: Vec<u32>,
}

/// Group occupied homes by single linkage within `radius`. Pure function.
pub fn clusters(homes: &[(HomeId, Pos, u32)], radius: i32) -> Vec<Vec<usize>> {
    let n = homes.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    for i in 0..n {
        for j in (i + 1)..n {
            if homes[i].1.chebyshev(homes[j].1) <= radius {
                let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                if a != b {
                    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
                    parent[hi] = lo;
                }
            }
        }
    }
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut root_of: Vec<usize> = vec![usize::MAX; n];
    for i in 0..n {
        let r = find(&mut parent, i);
        if root_of[r] == usize::MAX {
            root_of[r] = groups.len();
            groups.push(Vec::new());
        }
        groups[root_of[r]].push(i);
    }
    groups
}

impl SettlementTracker {
    pub fn update(&mut self, world: &World, day: u32, c: &SettlementConfig) {
        let occupied: Vec<(HomeId, Pos, u32)> = world
            .homes
            .iter()
            .filter(|h| !h.is_vacant())
            .map(|h| (h.id, h.pos, h.residents.len() as u32))
            .collect();
        let mut today: Vec<(Vec<HomeId>, u32, (f32, f32))> = Vec::new();
        for g in clusters(&occupied, c.radius as i32) {
            let residents: u32 = g.iter().map(|&i| occupied[i].2).sum();
            if (g.len() as u32) < c.min_homes || residents < c.min_residents {
                continue;
            }
            let mut ids: Vec<HomeId> = g.iter().map(|&i| occupied[i].0).collect();
            ids.sort_unstable();
            let k = g.len() as f32;
            let cx = g.iter().map(|&i| occupied[i].1.x as f32).sum::<f32>() / k;
            let cy = g.iter().map(|&i| occupied[i].1.y as f32).sum::<f32>() / k;
            today.push((ids, residents, (cx, cy)));
        }

        let mut used = vec![false; today.len()];
        let mut next: Vec<Tracked> = Vec::new();
        for t in &self.tracked {
            let mut best: Option<(usize, usize)> = None;
            for (gi, (ids, _, _)) in today.iter().enumerate() {
                if used[gi] {
                    continue;
                }
                let overlap = t
                    .homes
                    .iter()
                    .filter(|h| ids.binary_search(h).is_ok())
                    .count();
                if best.is_none_or(|(_, o)| overlap > o) {
                    best = Some((gi, overlap));
                }
            }
            if let Some((gi, overlap)) = best {
                if overlap * 2 >= t.homes.len() && overlap > 0 {
                    used[gi] = true;
                    let (ids, residents, centre) = today[gi].clone();
                    next.push(Tracked {
                        id: t.id,
                        homes: ids,
                        residents,
                        centre,
                        since_day: t.since_day,
                        class: SizeClass::Candidate,
                    });
                }
            }
        }
        for (gi, (ids, residents, centre)) in today.into_iter().enumerate() {
            if !used[gi] {
                next.push(Tracked {
                    id: self.next_id,
                    homes: ids,
                    residents,
                    centre,
                    since_day: day,
                    class: SizeClass::Candidate,
                });
                self.next_id += 1;
            }
        }
        for t in next.iter_mut() {
            let n = t.homes.len() as u32;
            t.class = if day.saturating_sub(t.since_day) < c.persist_days {
                SizeClass::Candidate
            } else if n >= c.town_homes && t.residents >= c.town_residents {
                SizeClass::Town
            } else if n >= c.village_homes && t.residents >= c.village_residents {
                SizeClass::Village
            } else {
                SizeClass::Hamlet
            };
            if t.class != SizeClass::Candidate && !self.ever_confirmed.contains(&t.id) {
                self.ever_confirmed.push(t.id);
            }
        }
        next.sort_by_key(|t| t.id);
        self.tracked = next;
    }

    pub fn count(&self, class: SizeClass) -> u32 {
        self.tracked.iter().filter(|t| t.class == class).count() as u32
    }

    pub fn confirmed(&self) -> impl Iterator<Item = &Tracked> {
        self.tracked
            .iter()
            .filter(|t| t.class != SizeClass::Candidate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_linkage_chains() {
        let homes = vec![
            (0, Pos::new(0, 0), 3),
            (1, Pos::new(3, 0), 3),
            (2, Pos::new(6, 0), 3),
            (3, Pos::new(20, 20), 3),
        ];
        let g = clusters(&homes, 3);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0], vec![0, 1, 2]);
        assert_eq!(g[1], vec![3]);
    }
}
