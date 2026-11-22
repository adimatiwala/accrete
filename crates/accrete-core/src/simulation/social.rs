//! Meetings between agents: knowledge sharing and barter (Week 3).
//!
//! **Sharing.** Agents standing on the same tile tell each other about the
//! best food places they remember (with the original sighting time, so the
//! listener learns nothing fresher than the teller knew). Sociable agents say
//! more; agents keep quiet with partners they distrust (negative reciprocity).
//!
//! **Barter** (accrete.md, "Trade rule"). Agents on the same or neighbouring
//! tiles may trade. Each values goods by its own needs: units below what it
//! wants (food reserve, building materials) are worth `food_value` or
//! `material_value`, units beyond it only `surplus_value`. The proposer offers
//! a ratio from both sides' marginal values; each side accepts only if it ends
//! up better off after the time the trade takes, and only goods actually held
//! change hands. Reciprocity scores rise when a partner's share of the gain
//! was fair, fall when it took most, and drift back to neutral over time.
//! Agents prefer high-score partners and demand more from low-score ones.

use crate::agents::agent::Agent;
use crate::agents::decision::{material_wants, reserve_target};
use crate::geom::Pos;
use crate::inventory::{Inventory, Resource};
use crate::simulation::Simulation;
use crate::simulation::events::EventKind;

/// Per-agent view used for valuing trades.
#[derive(Clone, Copy, Debug)]
struct Wants {
    want: [f32; 4],
    weight: [f32; 4],
    capacity: f32,
}

fn utility(inv: &Inventory, w: &Wants, surplus: f32) -> f32 {
    let mut u = 0.0;
    for r in Resource::GOODS {
        let x = inv.get(r);
        let d = w.want[r.index()];
        u += w.weight[r.index()] * x.min(d) + surplus * (x - d).max(0.0);
    }
    u
}

/// Marginal value of one more unit of `r`.
fn marginal(inv: &Inventory, w: &Wants, r: Resource, surplus: f32) -> f32 {
    if inv.get(r) < w.want[r.index()] {
        w.weight[r.index()]
    } else {
        surplus
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TradeOffer {
    pub give: Resource,
    pub give_qty: f32,
    pub get: Resource,
    pub get_qty: f32,
    pub gain_proposer: f32,
    pub gain_partner: f32,
}

impl Simulation {
    fn wants_of(&self, i: usize, tick: u64) -> Wants {
        let cx = self.ctx(tick);
        let a = &self.agents[i];
        let (wood, stone) = material_wants(a, &cx);
        let sc = &self.config.social;
        Wants {
            want: [
                reserve_target(a, &cx),
                wood,
                stone,
                if self.config.tools.enabled && !a.is_juvenile(&self.rates) {
                    self.config.tools.wanted
                } else {
                    0.0
                },
            ],
            weight: [
                sc.food_value * (1.0 + a.hunger),
                sc.material_value,
                sc.material_value,
                sc.tool_value,
            ],
            capacity: a.carry_capacity(&self.config, &self.rates),
        }
    }

    /// Find the best mutually beneficial offer from agent `p` to agent `q`.
    pub fn best_offer(&self, p: usize, q: usize, tick: u64) -> Option<TradeOffer> {
        let sc = &self.config.social;
        let inv_cfg = &self.config.inventory;
        let (wp, wq) = (self.wants_of(p, tick), self.wants_of(q, tick));
        let (ap, aq) = (&self.agents[p], &self.agents[q]);
        let time = self.config.decision.time_cost * sc.trade_ticks;
        let s = sc.surplus_value;
        let (up0, uq0) = (
            utility(&ap.inventory, &wp, s),
            utility(&aq.inventory, &wq, s),
        );
        // The partner asks for more from agents it distrusts.
        let trust = |a: &Agent, other: u32| {
            if sc.reciprocity_enabled {
                a.memory.reciprocity(other)
            } else {
                0.0
            }
        };
        let need_p = sc.trade_min_gain * (1.0 - 0.5 * trust(ap, aq.id));
        let need_q = sc.trade_min_gain * (1.0 - 0.5 * trust(aq, ap.id));
        let mut best: Option<TradeOffer> = None;
        const QTY: [f32; 10] = [1.0, 2.0, 3.0, 4.0, 6.0, 8.0, 10.0, 12.0, 16.0, 20.0];
        for give in Resource::GOODS {
            for get in Resource::GOODS {
                if give == get || ap.inventory.get(give) < 1.0 || aq.inventory.get(get) < 1.0 {
                    continue;
                }
                // Ratio (units of `get` per unit of `give`) at the midpoint of
                // the two sides' marginal rates of substitution.
                let mp =
                    marginal(&ap.inventory, &wp, give, s) / marginal(&ap.inventory, &wp, get, s);
                let mq =
                    marginal(&aq.inventory, &wq, give, s) / marginal(&aq.inventory, &wq, get, s);
                // A partner who distrusts the proposer demands better terms
                // (the proposer receives proportionally less); trust is
                // rewarded symmetrically, up to +/-50 % (trust is in [-1, 1]).
                let terms = 1.0 + 0.5 * trust(aq, ap.id);
                let ratio = 0.5 * (mp + mq) * terms;
                if !ratio.is_finite() || ratio <= 0.0 {
                    continue;
                }
                for &g in &QTY {
                    // Neither side takes more than it can carry.
                    let room_q = aq.inventory.room_for(give, wq.capacity, inv_cfg);
                    let give_qty = g.min(ap.inventory.get(give)).min(room_q);
                    let mut after_give = ap.inventory;
                    *after_give.get_mut(give) -= give_qty;
                    let room_p = after_give.room_for(get, wp.capacity, inv_cfg);
                    let get_qty = (give_qty * ratio).min(aq.inventory.get(get)).min(room_p);
                    if give_qty < 0.5 || get_qty < 0.5 {
                        continue;
                    }
                    let mut ip = ap.inventory;
                    let mut iq = aq.inventory;
                    *ip.get_mut(give) -= give_qty;
                    *iq.get_mut(give) += give_qty;
                    *iq.get_mut(get) -= get_qty;
                    *ip.get_mut(get) += get_qty;
                    if ip.weight(inv_cfg) > wp.capacity + 1e-4
                        || iq.weight(inv_cfg) > wq.capacity + 1e-4
                    {
                        continue;
                    }
                    let gp = utility(&ip, &wp, s) - up0 - time;
                    let gq = utility(&iq, &wq, s) - uq0 - time;
                    if gp < need_p || gq < need_q {
                        continue;
                    }
                    let better = match &best {
                        None => true,
                        Some(b) => gp.min(gq) > b.gain_proposer.min(b.gain_partner),
                    };
                    if better {
                        best = Some(TradeOffer {
                            give,
                            give_qty,
                            get,
                            get_qty,
                            gain_proposer: gp,
                            gain_partner: gq,
                        });
                    }
                }
            }
        }
        best
    }

    /// Sharing and trading among agents who meet this tick.
    pub(crate) fn meetings(&mut self, tick: u64) {
        let sc = self.config.social.clone();
        if !sc.share_enabled && !sc.trade_enabled {
            return;
        }
        // Agents by tile (sorted by tile index, then agent index).
        let mut by_tile: Vec<(u32, usize)> = self
            .agents
            .iter()
            .enumerate()
            .map(|(i, a)| (self.world.dims.idx(a.pos), i))
            .collect();
        by_tile.sort_unstable();

        if sc.share_enabled {
            let mut k = 0;
            while k < by_tile.len() {
                let mut end = k + 1;
                while end < by_tile.len() && by_tile[end].0 == by_tile[k].0 {
                    end += 1;
                }
                for x in k..end {
                    for y in (x + 1)..end {
                        self.share(by_tile[x].1, by_tile[y].1, tick);
                    }
                }
                k = end;
            }
        }

        if sc.trade_enabled {
            let mut order: Vec<usize> = (0..self.agents.len()).collect();
            self.rng_conflicts.shuffle(&mut order);
            let mut traded = vec![false; self.agents.len()];
            for p in order {
                if traded[p] || self.agents[p].inventory.is_empty() {
                    continue;
                }
                // Neighbours on the same or adjacent tiles, best-trusted first.
                let pos = self.agents[p].pos;
                let mut partners: Vec<usize> = Vec::new();
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let t = pos.offset(dx, dy);
                        if !self.world.contains(t) {
                            continue;
                        }
                        let ti = self.world.dims.idx(t);
                        let start = by_tile.partition_point(|&(tt, _)| tt < ti);
                        for &(tt, q) in &by_tile[start..] {
                            if tt != ti {
                                break;
                            }
                            if q != p && !traded[q] {
                                partners.push(q);
                            }
                        }
                    }
                }
                if partners.is_empty() {
                    continue;
                }
                let me = &self.agents[p];
                partners.sort_by(|&a, &b| {
                    let sa = me.memory.reciprocity(self.agents[a].id);
                    let sb = me.memory.reciprocity(self.agents[b].id);
                    sb.total_cmp(&sa)
                        .then(self.agents[a].id.cmp(&self.agents[b].id))
                });
                for q in partners.into_iter().take(3) {
                    if let Some(offer) = self.best_offer(p, q, tick) {
                        self.execute_trade(p, q, offer, tick);
                        traded[p] = true;
                        traded[q] = true;
                        break;
                    }
                }
            }
        }
    }

    fn share(&mut self, x: usize, y: usize, tick: u64) {
        let sc = &self.config.social;
        let interval = u64::from(sc.share_interval_ticks);
        let slots = sc.social_slots as usize;
        let ready = |a: &Agent| tick >= a.last_share_tick + interval;
        if !ready(&self.agents[x]) || !ready(&self.agents[y]) {
            return;
        }
        let (idx, idy) = (self.agents[x].id, self.agents[y].id);
        for (from, to, to_id) in [(x, y, idy), (y, x, idx)] {
            let teller = &self.agents[from];
            let teller_price = teller.memory.tool_price;
            if teller.memory.reciprocity(to_id) < 0.0 {
                continue; // keep quiet with someone who treated me badly
            }
            let n = (sc.share_items_base as f32
                + teller.traits.sociability * sc.share_items_sociability as f32)
                .round() as usize;
            let half_life = self.rates.freshness_half_life_ticks;
            let mut tips: Vec<_> = teller
                .memory
                .resources
                .iter()
                .filter(|m| m.resource == Resource::Food)
                .cloned()
                .collect();
            tips.sort_by(|a, b| {
                let va = a.amount * crate::agents::memory::freshness(a.seen_tick, tick, half_life);
                let vb = b.amount * crate::agents::memory::freshness(b.seen_tick, tick, half_life);
                vb.total_cmp(&va).then(a.pos.cmp(&b.pos))
            });
            tips.truncate(n);
            let mem_cfg = self.config.memory.clone();
            let listener = &mut self.agents[to];
            let mut learned = 0;
            for t in tips {
                if listener.memory.hear_about(&t, tick, &mem_cfg, half_life) {
                    learned += 1;
                }
            }
            listener.stats.tips_received += learned;
            // Talk turns to prices: the listener moves toward the teller's
            // believed exchange rate (A.26).
            if teller_price > 0.0 && self.config.tools.enabled {
                let g = self.config.tools.price_gossip;
                let m = &mut listener.memory;
                m.tool_price = if m.tool_price > 0.0 {
                    m.tool_price + g * (teller_price - m.tool_price)
                } else {
                    teller_price
                };
            }
            self.today.tips += learned;
        }
        for (i, other) in [(x, idy), (y, idx)] {
            let a = &mut self.agents[i];
            a.memory.meet(other, tick, slots);
            a.last_share_tick = tick;
        }
    }

    fn execute_trade(&mut self, p: usize, q: usize, o: TradeOffer, tick: u64) {
        let sc = self.config.social.clone();
        let slots = sc.social_slots as usize;
        let (idp, idq) = (self.agents[p].id, self.agents[q].id);
        let (hp, hq) = (self.agents[p].home, self.agents[q].home);
        self.trade_log
            .push(crate::simulation::emergence::TradeRecord {
                tick,
                a: idp,
                b: idq,
                cross_household: hp.is_none() || hp != hq,
            });
        let given = self.agents[p].inventory.take(o.give, o.give_qty);
        self.agents[q].inventory.put(o.give, given);
        let got = self.agents[q].inventory.take(o.get, o.get_qty);
        self.agents[p].inventory.put(o.get, got);
        self.ledger.transferred[o.give.index()] += f64::from(given);
        self.ledger.transferred[o.get.index()] += f64::from(got);
        // Each side learns the exchange rate it just experienced (A.26).
        let rate = match (o.give, o.get) {
            (Resource::Tools, Resource::Food) if given > 0.0 => Some(got / given),
            (Resource::Food, Resource::Tools) if got > 0.0 => Some(given / got),
            _ => None,
        };
        if let Some(rate) = rate {
            let lr = self.config.tools.price_learning;
            for k in [p, q] {
                let m = &mut self.agents[k].memory;
                m.tool_price = if m.tool_price > 0.0 {
                    m.tool_price + lr * (rate - m.tool_price)
                } else {
                    rate
                };
            }
        }
        let total = (o.gain_proposer + o.gain_partner).max(1e-6);
        // Each side rates the other by the other's share of the total gain.
        for (me, other, other_share) in [
            (p, idq, o.gain_partner / total),
            (q, idp, o.gain_proposer / total),
        ] {
            let a = &mut self.agents[me];
            a.stats.trades += 1;
            if let Some(k) = a.memory.meet(other, tick, slots) {
                let s = &mut a.memory.social[k];
                s.trades += 1;
                if sc.reciprocity_enabled {
                    let delta = sc.reciprocity_step * (1.0 - 4.0 * (other_share - 0.5).max(0.0));
                    s.score = (s.score + delta).clamp(-1.0, 1.0);
                }
            }
        }
        self.today.trades += 1;
        if o.give == Resource::Food {
            self.today.food_traded += f64::from(given);
        }
        if o.get == Resource::Food {
            self.today.food_traded += f64::from(got);
        }
        let pos: Pos = self.agents[p].pos;
        self.events.push(
            tick,
            Some(idp),
            pos,
            EventKind::Traded {
                partner: idq,
                gave: o.give,
                gave_qty: given,
                got: o.get,
                got_qty: got,
            },
        );
    }
}
