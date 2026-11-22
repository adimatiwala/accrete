//! The action space: goal-level options in fixed slots.
//!
//! Action `kind * SLOTS_PER_KIND + j` selects the `j`-th best option of goal
//! kind `kind` (in [`GoalKind::ALL`] order) as the agent's own scoring ranks
//! them, e.g. "gather food at the 2nd-best remembered place". The learner
//! decides *what* to do and which of a few candidate targets; route planning,
//! movement, and the mechanics of each action stay with the simulation.
//! Slots with no option behind them are masked out. `Idle` (wait one tick in
//! place) is always available, so the mask is never empty.

use crate::agents::agent::GoalKind;
use crate::agents::decision::{self, Candidate};
use crate::simulation::Simulation;

pub const SLOTS_PER_KIND: usize = 3;
pub const N_ACTIONS: usize = GoalKind::COUNT * SLOTS_PER_KIND;

/// Decode an action index into `(kind, rank)`.
pub fn decode(action: usize) -> Option<(GoalKind, usize)> {
    (action < N_ACTIONS).then(|| {
        (
            GoalKind::ALL[action / SLOTS_PER_KIND],
            action % SLOTS_PER_KIND,
        )
    })
}

pub fn encode(kind: GoalKind, rank: usize) -> usize {
    kind as usize * SLOTS_PER_KIND + rank
}

/// Human-readable action names, e.g. `gather-food#1`.
pub fn action_names() -> Vec<String> {
    (0..N_ACTIONS)
        .map(|a| {
            let (k, j) = decode(a).expect("in range");
            format!("{}#{}", k.name(), j)
        })
        .collect()
}

/// The option behind every slot for agent `idx` at the current tick, plus the
/// slot the agent's own (scripted) decision logic would pick.
pub fn slot_candidates(sim: &Simulation, idx: usize) -> (Vec<Option<Candidate>>, usize) {
    let cx = sim.ctx(sim.tick);
    let agent = &sim.agents[idx];
    let ranked = decision::evaluate(agent, &cx);
    let mut slots: Vec<Option<Candidate>> = vec![None; N_ACTIONS];
    let mut filled = [0usize; GoalKind::COUNT];
    let mut heuristic = None;
    for cand in ranked {
        let k = cand.kind as usize;
        if filled[k] >= SLOTS_PER_KIND {
            continue;
        }
        let a = encode(cand.kind, filled[k]);
        filled[k] += 1;
        if heuristic.is_none() {
            heuristic = Some(a);
        }
        slots[a] = Some(cand);
    }
    let idle = encode(GoalKind::Idle, 0);
    if slots[idle].is_none() {
        slots[idle] = Some(Candidate {
            kind: GoalKind::Idle,
            target: agent.pos,
            join: None,
            score: 0.0,
            est_ticks: 0.0,
            terms: Vec::new(),
        });
    }
    (slots, heuristic.unwrap_or(idle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_roundtrip() {
        for a in 0..N_ACTIONS {
            let (k, j) = decode(a).unwrap();
            assert_eq!(encode(k, j), a);
        }
        assert!(decode(N_ACTIONS).is_none());
        assert_eq!(action_names().len(), N_ACTIONS);
    }
}
