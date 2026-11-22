//! Inventories of food, wood, and stone, measured in resource units.
//!
//! Capacity is measured in *weight units*: food 1, wood 2, stone 3 by default
//! (see `InventoryConfig`). All transfers go through methods that clamp to what
//! is actually held, so callers cannot create or destroy resources by accident.

use serde::{Deserialize, Serialize};

use crate::config::InventoryConfig;
use crate::hash::{StableHash, StableHasher};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Resource {
    Food,
    Wood,
    Stone,
    /// A crafted, durable good (A.26). Never on tiles; only carried, stored,
    /// traded, and worn out by use.
    Tools,
}

impl Resource {
    /// The natural resources: what tiles hold, regrow, and agents remember.
    pub const ALL: [Resource; 3] = [Resource::Food, Resource::Wood, Resource::Stone];
    /// Every good an inventory can hold (natural resources plus tools).
    pub const GOODS: [Resource; 4] = [
        Resource::Food,
        Resource::Wood,
        Resource::Stone,
        Resource::Tools,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn name(self) -> &'static str {
        match self {
            Resource::Food => "food",
            Resource::Wood => "wood",
            Resource::Stone => "stone",
            Resource::Tools => "tools",
        }
    }

    pub fn weight(self, c: &InventoryConfig) -> f32 {
        match self {
            Resource::Food => c.food_weight,
            Resource::Wood => c.wood_weight,
            Resource::Stone => c.stone_weight,
            Resource::Tools => c.tools_weight,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct Inventory {
    pub food: f32,
    pub wood: f32,
    pub stone: f32,
    #[serde(default)]
    pub tools: f32,
}

impl Inventory {
    pub const EMPTY: Inventory = Inventory {
        food: 0.0,
        wood: 0.0,
        stone: 0.0,
        tools: 0.0,
    };

    #[inline]
    pub fn get(&self, r: Resource) -> f32 {
        match r {
            Resource::Food => self.food,
            Resource::Wood => self.wood,
            Resource::Stone => self.stone,
            Resource::Tools => self.tools,
        }
    }

    #[inline]
    pub fn get_mut(&mut self, r: Resource) -> &mut f32 {
        match r {
            Resource::Food => &mut self.food,
            Resource::Wood => &mut self.wood,
            Resource::Stone => &mut self.stone,
            Resource::Tools => &mut self.tools,
        }
    }

    pub fn weight(&self, c: &InventoryConfig) -> f32 {
        self.food * c.food_weight
            + self.wood * c.wood_weight
            + self.stone * c.stone_weight
            + self.tools * c.tools_weight
    }

    /// Units of `r` that fit in the remaining capacity.
    pub fn room_for(&self, r: Resource, capacity: f32, c: &InventoryConfig) -> f32 {
        ((capacity - self.weight(c)) / r.weight(c)).max(0.0)
    }

    pub fn is_empty(&self) -> bool {
        self.food <= 0.0 && self.wood <= 0.0 && self.stone <= 0.0 && self.tools <= 0.0
    }

    /// Remove up to `amount` of `r`; returns the amount actually removed.
    pub fn take(&mut self, r: Resource, amount: f32) -> f32 {
        let slot = self.get_mut(r);
        let got = amount.max(0.0).min(*slot);
        *slot -= got;
        if *slot < 1e-6 {
            // Snap dust to zero so tiny residues never accumulate; the residue is
            // returned with the take so nothing is lost.
            let dust = *slot;
            *slot = 0.0;
            return got + dust;
        }
        got
    }

    /// Add `amount` of `r` (caller is responsible for capacity checks).
    pub fn put(&mut self, r: Resource, amount: f32) {
        debug_assert!(amount >= 0.0 && amount.is_finite());
        *self.get_mut(r) += amount;
    }

    /// Move up to `amount` of `r` from `self` into `to`, limited by `to`'s room.
    pub fn transfer_to(
        &mut self,
        to: &mut Inventory,
        r: Resource,
        amount: f32,
        to_capacity: f32,
        c: &InventoryConfig,
    ) -> f32 {
        let room = to.room_for(r, to_capacity, c);
        let moved = self.take(r, amount.min(room));
        to.put(r, moved);
        moved
    }

    pub fn add_all(&mut self, o: &Inventory) {
        self.food += o.food;
        self.wood += o.wood;
        self.stone += o.stone;
        self.tools += o.tools;
    }

    pub fn is_valid(&self) -> bool {
        Resource::GOODS
            .iter()
            .all(|&r| self.get(r).is_finite() && self.get(r) >= 0.0)
    }
}

impl StableHash for Inventory {
    fn stable_hash(&self, h: &mut StableHasher) {
        h.f32(self.food);
        h.f32(self.wood);
        h.f32(self.stone);
        h.f32(self.tools);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_never_exceeds_holdings() {
        let mut inv = Inventory {
            food: 3.0,
            ..Inventory::EMPTY
        };
        assert_eq!(inv.take(Resource::Food, 5.0), 3.0);
        assert_eq!(inv.food, 0.0);
        assert_eq!(inv.take(Resource::Food, 1.0), 0.0);
    }

    #[test]
    fn transfer_respects_capacity() {
        let c = InventoryConfig::default();
        let mut a = Inventory {
            wood: 20.0,
            ..Inventory::EMPTY
        };
        let mut b = Inventory::EMPTY;
        let moved = a.transfer_to(&mut b, Resource::Wood, 20.0, 30.0, &c);
        assert_eq!(moved, 15.0); // 15 wood = 30 weight
        assert_eq!(a.wood + b.wood, 20.0);
        assert!(b.weight(&c) <= 30.0);
    }
}
