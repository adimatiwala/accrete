//! Simulation configuration.
//!
//! Every tunable lives here, with the defaults from `accrete.md`. Rates are
//! written **per day** (or per hour where the spec says so) and converted to
//! per-tick values in exactly one place: [`TickRates::new`].
//!
//! `configs/default.toml` mirrors `Config::default()`; a test keeps them equal.

use serde::{Deserialize, Serialize};

use crate::hash::{StableHash, StableHasher};

/// 1 tick = 1 hour.
pub const TICKS_PER_DAY: u64 = 24;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub world: WorldConfig,
    pub resources: ResourceConfig,
    pub sight: SightConfig,
    pub movement: MovementConfig,
    pub spawn: SpawnConfig,
    pub needs: NeedsConfig,
    pub life: LifeConfig,
    pub inventory: InventoryConfig,
    pub memory: MemoryConfig,
    pub skills: SkillConfig,
    pub homes: HomeConfig,
    pub decision: DecisionConfig,
    pub pathology: PathologyConfig,
    pub settlement: SettlementConfig,
    pub trails: TrailConfig,
    pub weather: WeatherConfig,
    pub social: SocialConfig,
    pub infrastructure: InfrastructureConfig,
    pub disruption: DisruptionConfig,
    pub sim: SimConfig,
}

impl Config {
    pub fn from_toml_str(s: &str) -> Result<Self, String> {
        let c: Self = toml::from_str(s).map_err(|e| e.to_string())?;
        c.validate()?;
        Ok(c)
    }

    /// Rejects settings no run can use, so a bad value fails up front
    /// instead of panicking or running silently with nonsense:
    /// - every float is finite (NaN or infinity cannot be reproduced);
    /// - quantities, rates and capacities are non-negative (utility weights
    ///   in `[decision]` and `infrastructure.household_weight` are
    ///   preferences and may be any finite value);
    /// - fractions and probabilities lie in `[0, 1]`;
    /// - every `*_min` is at most its `*_max`;
    /// - the world, movement points and memory pools are large enough to run.
    pub fn validate(&self) -> Result<(), String> {
        let doc = toml::Value::try_from(self).map_err(|e| e.to_string())?;
        let sections = doc.as_table().expect("config is a table");
        for (section, table) in sections {
            let Some(table) = table.as_table() else {
                continue;
            };
            for (key, v) in table {
                let Some(x) = v.as_float() else { continue };
                let name = format!("{section}.{key}");
                if !x.is_finite() {
                    return Err(format!("{name} must be a finite number, got {x}"));
                }
                let preference = section == "decision" || name == "infrastructure.household_weight";
                if x < 0.0 && !preference {
                    return Err(format!("{name} must not be negative, got {x}"));
                }
            }
        }
        let unit = [
            ("world.ocean_fraction", self.world.ocean_fraction),
            ("world.mountain_fraction", self.world.mountain_fraction),
            ("world.hill_fraction", self.world.hill_fraction),
            (
                "resources.initial_food_fraction",
                self.resources.initial_food_fraction,
            ),
            ("life.reproduce_chance", self.life.reproduce_chance),
            (
                "inventory.carried_spoil_per_day",
                self.inventory.carried_spoil_per_day,
            ),
            (
                "inventory.stored_spoil_per_day",
                self.inventory.stored_spoil_per_day,
            ),
            (
                "memory.newborn_resource_share",
                self.memory.newborn_resource_share,
            ),
            ("trails.decay", self.trails.decay),
            ("weather.daily_chance", self.weather.daily_chance),
            ("weather.severity_min", self.weather.severity_min),
            ("weather.severity_max", self.weather.severity_max),
        ];
        for (name, x) in unit {
            if !(0.0..=1.0).contains(&x) {
                return Err(format!("{name} must be between 0 and 1, got {x}"));
            }
        }
        let w = &self.world;
        if w.ocean_fraction + w.mountain_fraction + w.hill_fraction > 1.0 {
            return Err(format!(
                "world.ocean_fraction + mountain_fraction + hill_fraction must be at most 1, got {}",
                w.ocean_fraction + w.mountain_fraction + w.hill_fraction
            ));
        }
        let pairs_f = [
            (
                "spawn.initial_hunger",
                self.spawn.initial_hunger_min,
                self.spawn.initial_hunger_max,
            ),
            (
                "skills.aptitude",
                self.skills.aptitude_min,
                self.skills.aptitude_max,
            ),
            (
                "weather.severity",
                self.weather.severity_min,
                self.weather.severity_max,
            ),
        ];
        for (name, lo, hi) in pairs_f {
            if lo > hi {
                return Err(format!(
                    "{name}_min ({lo}) must not exceed {name}_max ({hi})"
                ));
            }
        }
        if self.life.maturity_days > self.life.elder_days {
            return Err(format!(
                "life.maturity_days ({}) must not exceed life.elder_days ({})",
                self.life.maturity_days, self.life.elder_days
            ));
        }
        let pairs_u = [
            (
                "resources.stone",
                self.resources.stone_min,
                self.resources.stone_max,
            ),
            (
                "weather.radius",
                self.weather.radius_min,
                self.weather.radius_max,
            ),
            (
                "weather.length",
                self.weather.length_min_days,
                self.weather.length_max_days,
            ),
        ];
        for (name, lo, hi) in pairs_u {
            if lo > hi {
                return Err(format!(
                    "{name}_min ({lo}) must not exceed {name}_max ({hi})"
                ));
            }
        }
        let at_least_one = [
            ("world.width", self.world.width),
            ("world.height", self.world.height),
            ("movement.mp_per_tick", self.movement.mp_per_tick),
            ("movement.min_mp", self.movement.min_mp),
            // Agents must be able to remember at least one blocked tile, or
            // they walk into floods forever and the stuck detector never fires.
            ("memory.disruption_slots", self.memory.disruption_slots),
        ];
        for (name, x) in at_least_one {
            if x == 0 {
                return Err(format!("{name} must be at least 1"));
            }
        }
        self.disruption.validate()
    }

    pub fn to_toml_string(&self) -> String {
        toml::to_string_pretty(self).expect("config serializes")
    }

    /// Hash of the configuration, via its canonical JSON form. Floats
    /// serialize with round-trip precision, and field order is the struct's.
    /// Debugging-only settings that never change results
    /// (`sim.check_invariants`, `sim.event_log_capacity`) are excluded, so
    /// turning on `--check` does not change the state hash.
    pub fn digest(&self) -> u64 {
        let json =
            serde_json::to_string(&self.without_debug_settings()).expect("config serializes");
        crate::hash::fnv1a64(json.as_bytes())
    }

    /// This config with the debugging-only settings reset to their defaults.
    pub fn without_debug_settings(&self) -> Config {
        let mut c = self.clone();
        let d = SimConfig::default();
        c.sim.check_invariants = d.check_invariants;
        c.sim.event_log_capacity = d.event_log_capacity;
        c
    }

    /// Whether `other` differs from this config in anything that affects
    /// results (debugging-only settings are ignored).
    pub fn differs_in_results(&self, other: &Config) -> bool {
        self.without_debug_settings() != other.without_debug_settings()
    }
}

impl StableHash for Config {
    fn stable_hash(&self, h: &mut StableHasher) {
        h.u64(self.digest());
    }
}

// ---------------------------------------------------------------------------
// World generation
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorldConfig {
    pub width: u32,
    pub height: u32,
    /// Fraction of tiles below sea level (chosen by elevation quantile).
    pub ocean_fraction: f32,
    /// Fraction of *land* tiles classified as mountains (highest first).
    pub mountain_fraction: f32,
    /// Fraction of *land* tiles classified as hills (next highest).
    pub hill_fraction: f32,
    /// Normalized moisture at or above which low land is forest.
    pub forest_moisture: f32,
    /// Base feature size of continents, in tiles.
    pub elevation_scale: f32,
    pub elevation_octaves: u32,
    /// Domain-warp strength, in tiles.
    pub warp_strength: f32,
    /// Height of ridged mountain chains added on top of the base terrain.
    pub ridge_strength: f32,
    /// How strongly elevation is pushed down toward the map edge (0 = none).
    pub edge_falloff: f32,
    pub moisture_scale: f32,
    pub moisture_octaves: u32,
    /// Added moisture next to the coast, fading over `coast_moisture_range` tiles.
    pub coast_moisture: f32,
    pub coast_moisture_range: u32,
    /// Water bodies smaller than this (not touching the border) are filled to land.
    pub min_lake_tiles: u32,
    /// Land masses smaller than this are submerged.
    pub min_island_tiles: u32,
    /// Majority-filter passes that remove isolated biome specks.
    pub smoothing_passes: u32,
    /// Flow accumulation (in rain units, about one per tile) needed to form a river.
    pub river_threshold: f32,
    /// Minimum filled-elevation drop per downstream step, used to drain flats and pits.
    pub river_epsilon: f32,
}

impl Default for WorldConfig {
    fn default() -> Self {
        Self {
            width: 128,
            height: 128,
            ocean_fraction: 0.30,
            mountain_fraction: 0.07,
            hill_fraction: 0.16,
            forest_moisture: 0.56,
            elevation_scale: 56.0,
            elevation_octaves: 6,
            warp_strength: 9.0,
            ridge_strength: 0.35,
            edge_falloff: 0.9,
            moisture_scale: 40.0,
            moisture_octaves: 5,
            coast_moisture: 0.12,
            coast_moisture_range: 6,
            min_lake_tiles: 10,
            min_island_tiles: 24,
            smoothing_passes: 2,
            river_threshold: 110.0,
            river_epsilon: 1.0e-5,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ResourceConfig {
    // Fertility = clamp(biome_base + moisture_weight * moisture + river_bonus(d), 0, 1)
    pub fertility_plains: f32,
    pub fertility_forest: f32,
    pub fertility_hills: f32,
    pub fertility_mountains: f32,
    pub moisture_weight: f32,
    /// River bonus for distance 1, 2, 3, 4 tiles; zero beyond.
    pub river_bonus: [f32; 4],
    /// Tiles with lower fertility grow no food.
    pub min_food_fertility: f32,
    pub food_capacity_scale: f32,
    pub patch_scale: f32,
    pub patch_rich_above: f32,
    pub patch_poor_below: f32,
    pub patch_rich_factor: f32,
    pub patch_poor_factor: f32,
    pub initial_food_fraction: f32,
    /// food += food_regrowth * fertility * (capacity - food), once per day.
    pub food_regrowth: f32,
    pub forest_wood: f32,
    pub plains_wood: f32,
    pub plains_wood_min_moisture: f32,
    /// wood += wood_regrowth * (capacity - wood), once per day.
    pub wood_regrowth: f32,
    pub stone_scale: f32,
    pub stone_threshold: f32,
    pub stone_min: u32,
    pub stone_max: u32,
}

impl Default for ResourceConfig {
    fn default() -> Self {
        Self {
            fertility_plains: 0.3,
            fertility_forest: 0.2,
            fertility_hills: 0.1,
            fertility_mountains: 0.0,
            moisture_weight: 0.3,
            river_bonus: [0.35, 0.25, 0.15, 0.05],
            min_food_fertility: 0.15,
            food_capacity_scale: 20.0,
            patch_scale: 7.0,
            patch_rich_above: 0.65,
            patch_poor_below: 0.35,
            patch_rich_factor: 1.5,
            patch_poor_factor: 0.4,
            initial_food_fraction: 0.8,
            food_regrowth: 0.15,
            forest_wood: 40.0,
            plains_wood: 5.0,
            plains_wood_min_moisture: 0.5,
            wood_regrowth: 0.02,
            stone_scale: 9.0,
            stone_threshold: 0.6,
            stone_min: 50,
            stone_max: 200,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SightConfig {
    /// Square sight radius in tiles (6 -> 13x13). Fixed at 6 by the mask layout.
    pub radius: u32,
    /// Vertical relief of land, in metres, from sea level to the highest peak.
    pub relief_m: f32,
    pub eye_height_m: f32,
    /// Forest canopy height added to a forest tile when it is *between* the
    /// viewer and the target (forests hide what lies behind them).
    pub canopy_m: f32,
    /// Re-observe every N ticks while staying put.
    pub reobserve_ticks: u32,
}

impl Default for SightConfig {
    fn default() -> Self {
        Self {
            radius: 6,
            relief_m: 1500.0,
            eye_height_m: 15.0,
            canopy_m: 15.0,
            reobserve_ticks: 6,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MovementConfig {
    pub mp_per_tick: u32,
    pub plains_mp: u32,
    pub forest_mp: u32,
    pub hills_mp: u32,
    pub mountains_mp: u32,
    pub river_extra_mp: u32,
    /// Diagonal steps cost `diagonal_num / 10` times the tile cost.
    pub diagonal_num: u32,
    pub min_mp: u32,
    /// A* expansion limit per search.
    pub astar_max_expansions: u32,
    /// A route is re-planned at most this often unless blocked.
    pub replan_ticks: u32,
}

impl Default for MovementConfig {
    fn default() -> Self {
        Self {
            mp_per_tick: 10,
            plains_mp: 10,
            forest_mp: 20,
            hills_mp: 30,
            mountains_mp: 60,
            river_extra_mp: 20,
            diagonal_num: 14,
            min_mp: 5,
            astar_max_expansions: 4000,
            replan_ticks: 12,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpawnConfig {
    pub agents: u32,
    /// Minimum Chebyshev spacing between spawn tiles; relaxed if too few fit.
    pub min_spacing: u32,
    /// Radius in which a spawn tile must have enough food.
    pub food_radius: u32,
    /// Minimum food within `food_radius` (units).
    pub min_nearby_food: f32,
    /// Initial agents know terrain (not resources) within this radius of spawn.
    pub initial_known_radius: u32,
    pub initial_food: f32,
    pub initial_hunger_min: f32,
    pub initial_hunger_max: f32,
}

impl Default for SpawnConfig {
    fn default() -> Self {
        Self {
            agents: 100,
            min_spacing: 4,
            food_radius: 4,
            min_nearby_food: 60.0,
            initial_known_radius: 8,
            initial_food: 4.0,
            initial_hunger_min: 0.05,
            initial_hunger_max: 0.3,
        }
    }
}

// ---------------------------------------------------------------------------
// Agents (Week 2)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NeedsConfig {
    pub hunger_per_day: f32,
    pub juvenile_hunger_factor: f32,
    pub hunger_per_food: f32,
    /// Max food units eaten in one Eat action (one meal).
    pub max_meal: f32,
    pub energy_per_active_tick: f32,
    pub rest_per_tick: f32,
    pub rest_per_tick_home: f32,
    pub very_hungry: f32,
    pub health_loss_deprived_per_day: f32,
    pub health_loss_starving_per_day: f32,
    pub health_recovery_per_day: f32,
    pub low_health: f32,
}

impl Default for NeedsConfig {
    fn default() -> Self {
        Self {
            hunger_per_day: 0.33,
            juvenile_hunger_factor: 0.5,
            hunger_per_food: 0.165,
            max_meal: 4.0,
            energy_per_active_tick: 0.05,
            rest_per_tick: 0.1,
            rest_per_tick_home: 0.125,
            very_hungry: 0.7,
            health_loss_deprived_per_day: 0.05,
            health_loss_starving_per_day: 0.15,
            health_recovery_per_day: 0.1,
            low_health: 0.3,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LifeConfig {
    pub maturity_days: f32,
    pub elder_days: f32,
    pub age_decline_per_day: f32,
    /// Frailty gained per day spent very hungry, exhausted, or at low health.
    pub frailty_per_deprived_day: f32,
    /// Extra frailty per day for each resident of an overcrowded home.
    pub frailty_per_crowded_day: f32,
    pub reproduce_cooldown_days: f32,
    pub reproduce_health_margin: f32,
    pub reproduce_food_carried: f32,
    pub reproduce_food_stored: f32,
    pub reproduce_chance: f32,
    /// After a failed birth roll the agent waits this long before trying again.
    pub reproduce_retry_days: f32,
    pub newborn_food: f32,
    pub birth_energy_cost: f32,
    pub juvenile_gather_factor: f32,
    /// Juveniles keep within this many tiles of home.
    pub juvenile_leash: u32,
    pub trait_inherit_jitter: f32,
    pub population_cap: u32,
}

impl Default for LifeConfig {
    fn default() -> Self {
        Self {
            maturity_days: 40.0,
            elder_days: 150.0,
            age_decline_per_day: 0.012,
            frailty_per_deprived_day: 0.006,
            frailty_per_crowded_day: 0.001,
            reproduce_cooldown_days: 30.0,
            reproduce_health_margin: 0.1,
            reproduce_food_carried: 10.0,
            reproduce_food_stored: 20.0,
            reproduce_chance: 0.35,
            reproduce_retry_days: 1.0,
            newborn_food: 6.0,
            birth_energy_cost: 0.2,
            juvenile_gather_factor: 0.5,
            juvenile_leash: 10,
            trait_inherit_jitter: 0.1,
            population_cap: 1000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InventoryConfig {
    pub carry_adult: f32,
    pub carry_juvenile: f32,
    pub food_weight: f32,
    pub wood_weight: f32,
    pub stone_weight: f32,
    pub carried_spoil_per_day: f32,
    pub stored_spoil_per_day: f32,
    pub chest_capacity: f32,
    pub max_chests: u32,
    pub storage_wood: f32,
    pub storage_ticks: u32,
}

impl Default for InventoryConfig {
    fn default() -> Self {
        Self {
            carry_adult: 30.0,
            carry_juvenile: 15.0,
            food_weight: 1.0,
            wood_weight: 2.0,
            stone_weight: 3.0,
            carried_spoil_per_day: 0.02,
            stored_spoil_per_day: 0.005,
            chest_capacity: 200.0,
            max_chests: 3,
            storage_wood: 10.0,
            storage_ticks: 6,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MemoryConfig {
    /// Master switch for the "Knowledge" experiment: when false, agents keep
    /// no resource memories beyond what they currently see.
    pub enabled: bool,
    /// The 64 resource-memory slots are partitioned by resource type, so that
    /// plentiful wood never crowds food out of memory.
    pub food_slots: u32,
    pub wood_slots: u32,
    pub stone_slots: u32,
    pub changed_slots: u32,
    /// Route disruptions (floods, landslides) the agent has seen. Kept apart
    /// from `changed_slots` so a large flood never crowds out trails and roads.
    pub disruption_slots: u32,
    pub home_slots: u32,
    /// Freshness = half_life / (half_life + age), with half_life in days.
    pub freshness_half_life_days: f32,
    /// Ignore resource amounts below this when observing.
    pub min_amount: f32,
    pub newborn_resource_share: f32,
    pub newborn_known_radius: u32,
    /// A target whose route failed is skipped for this many ticks.
    pub unreachable_cooldown_ticks: u32,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            food_slots: 40,
            wood_slots: 16,
            stone_slots: 8,
            changed_slots: 32,
            disruption_slots: 1024,
            home_slots: 16,
            freshness_half_life_days: 5.0,
            min_amount: 1.0,
            newborn_resource_share: 0.5,
            newborn_known_radius: 16,
            unreachable_cooldown_ticks: 48,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SkillConfig {
    pub aptitude_min: f32,
    pub aptitude_max: f32,
    pub practice_per_tick: f32,
    pub practice_max: f32,
    pub practice_decay_per_day: f32,
    pub base_food: f32,
    pub base_wood: f32,
    pub base_stone: f32,
}

impl Default for SkillConfig {
    fn default() -> Self {
        Self {
            aptitude_min: 0.7,
            aptitude_max: 1.3,
            practice_per_tick: 0.005,
            practice_max: 0.3,
            practice_decay_per_day: 0.001,
            base_food: 2.0,
            base_wood: 1.5,
            base_stone: 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HomeConfig {
    pub max_homes_per_tile: u32,
    pub comfortable_capacity: u32,
    pub crowding_grace_days: f32,
    pub crowding_rest_penalty: f32,
    pub crowding_rest_penalty_max: f32,
    /// Homeless adults consider founding a home after this many days alive.
    pub settle_after_days: f32,
    /// Radius around a candidate site that an agent considers when judging it.
    pub site_radius: u32,
    /// Daily food per resident that a site's known land should provide.
    pub site_food_per_person: f32,
    /// How often (ticks) an agent re-evaluates where it lives.
    pub reconsider_ticks: u32,
}

impl Default for HomeConfig {
    fn default() -> Self {
        Self {
            max_homes_per_tile: 8,
            comfortable_capacity: 4,
            crowding_grace_days: 7.0,
            crowding_rest_penalty: 0.15,
            crowding_rest_penalty_max: 0.6,
            settle_after_days: 2.0,
            site_radius: 5,
            site_food_per_person: 5.0,
            reconsider_ticks: 24,
        }
    }
}

/// Utility weights. Each option's score is `sum_i w_i * V_i - C`, and every
/// term is recorded for the inspector. See `agents::decision`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DecisionConfig {
    /// Value of one tick of the agent's time, at rest and well fed.
    pub time_cost: f32,
    pub eat_weight: f32,
    pub eat_threshold_base: f32,
    pub eat_threshold_caution: f32,
    pub rest_weight: f32,
    pub rest_threshold_base: f32,
    pub rest_threshold_diligence: f32,
    pub rest_threshold_caution: f32,
    pub rest_until: f32,
    pub food_weight: f32,
    /// Days of food an agent wants to carry: base + caution * span.
    pub reserve_days_base: f32,
    pub reserve_days_caution: f32,
    /// Days of food per resident a household wants in its chests.
    pub household_store_days: f32,
    pub wood_weight: f32,
    pub explore_weight: f32,
    pub explore_rays: u32,
    pub explore_max_dist: u32,
    pub explore_desperation: f32,
    /// Homed agents pay this fraction of the trip home in the score of far targets.
    pub home_pull: f32,
    pub home_weight: f32,
    pub social_weight: f32,
    /// Value of living within 3 tiles of the home an agent grew up in.
    pub kin_weight: f32,
    pub crowding_weight: f32,
    pub move_home_margin: f32,
    pub storage_weight: f32,
    pub deposit_weight: f32,
    pub withdraw_weight: f32,
    pub reproduce_weight: f32,
    /// Days of household food per person an agent wants before having a child.
    pub reproduce_security_days: f32,
    /// Weight of a household-food shortfall against having a child (0 = ignore).
    pub reproduce_security_weight: f32,
    /// Weight of perceived land shortage against having a child (0 = ignore;
    /// the "no preventive check" experiment).
    pub reproduce_land_weight: f32,
    /// Weight of poor *experienced* foraging returns against having a child
    /// (0 = ignore). Returns are food gathered per tick spent on food trips.
    pub reproduce_return_weight: f32,
    /// Foraging return (food per trip-tick) below which a child feels unaffordable.
    pub reproduce_min_return: f32,
    /// Value of gathering a unit-fraction of wanted building materials.
    pub materials_weight: f32,
    pub shelter_weight: f32,
    /// Bonus for continuing the current goal (hysteresis).
    pub commitment: f32,
    /// Amplitude of deterministic per-agent tie-breaking jitter.
    pub jitter: f32,
}

impl Default for DecisionConfig {
    fn default() -> Self {
        Self {
            time_cost: 0.02,
            eat_weight: 3.0,
            eat_threshold_base: 0.4,
            eat_threshold_caution: 0.15,
            rest_weight: 2.5,
            rest_threshold_base: 0.3,
            rest_threshold_diligence: 0.15,
            rest_threshold_caution: 0.1,
            rest_until: 0.95,
            food_weight: 1.6,
            reserve_days_base: 2.0,
            reserve_days_caution: 4.0,
            household_store_days: 6.0,
            wood_weight: 0.8,
            explore_weight: 0.6,
            explore_rays: 16,
            explore_max_dist: 48,
            explore_desperation: 1.2,
            home_pull: 0.5,
            home_weight: 1.0,
            social_weight: 0.35,
            kin_weight: 0.25,
            crowding_weight: 0.6,
            move_home_margin: 0.35,
            storage_weight: 0.9,
            deposit_weight: 0.9,
            withdraw_weight: 1.2,
            reproduce_weight: 1.5,
            reproduce_security_days: 6.0,
            reproduce_security_weight: 2.0,
            reproduce_land_weight: 3.0,
            reproduce_return_weight: 3.0,
            reproduce_min_return: 0.85,
            materials_weight: 0.8,
            shelter_weight: 0.9,
            commitment: 0.15,
            jitter: 0.02,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PathologyConfig {
    /// Oscillating = more than this many goal abandonments within
    /// `oscillation_window_ticks`.
    pub oscillation_switches: u32,
    pub oscillation_window_ticks: u32,
    pub stuck_ticks: u32,
    pub starved_reach_ticks: u32,
    pub idle_ticks_per_day: u32,
}

impl Default for PathologyConfig {
    fn default() -> Self {
        Self {
            oscillation_switches: 6,
            oscillation_window_ticks: 24,
            stuck_ticks: 12,
            starved_reach_ticks: 48,
            idle_ticks_per_day: 8,
        }
    }
}

/// Route disruptions (accrete.md, Week 4 Day 2): tiles made impassable or
/// slow for a while by an experiment, applied through ordinary movement
/// costs. Agents are never told; they find out by seeing the tiles.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DisruptionConfig {
    pub scripted: Vec<ScriptedDisruption>,
}

impl DisruptionConfig {
    pub fn validate(&self) -> Result<(), String> {
        self.scripted
            .iter()
            .try_for_each(ScriptedDisruption::validate)
    }
}

impl ScriptedDisruption {
    /// Reject entries that would silently do nothing or something else than
    /// written. A slow factor of 255 is reserved: it would tie with `Blocked`
    /// when overlapping disruptions keep the worst effect.
    pub fn validate(&self) -> Result<(), String> {
        let at = format!("disruption starting on day {}", self.start_day);
        if self.days == 0 {
            return Err(format!("{at}: days must be at least 1"));
        }
        if self.kind == DisruptionKind::Slow && !(2..=254).contains(&self.slow_factor) {
            return Err(format!(
                "{at}: slow_factor must be in 2..=254, got {}",
                self.slow_factor
            ));
        }
        if self.busiest_tiles > 0 && self.radius > 0 {
            return Err(format!(
                "{at}: set either busiest_tiles or center/radius, not both"
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DisruptionKind {
    /// Tiles become impassable.
    #[default]
    Block,
    /// Entering tiles costs `slow_factor` times the normal movement points.
    Slow,
}

/// One scheduled route disruption. It affects either the land tiles within
/// `radius` of `center`, or, when `busiest_tiles > 0`, that many of the most
/// travelled land tiles without homes on them (highest decayed traffic, so
/// trails and roads first) on `start_day`. Choosing tiles by traffic is the
/// experiment's world-level choice, not any agent's. A `center`/`radius`
/// area covers every land tile in it, homes included (a local flood).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptedDisruption {
    pub start_day: u32,
    pub days: u32,
    #[serde(default)]
    pub kind: DisruptionKind,
    #[serde(default = "default_slow_factor")]
    pub slow_factor: u8,
    #[serde(default)]
    pub center: [i32; 2],
    #[serde(default)]
    pub radius: u32,
    #[serde(default)]
    pub busiest_tiles: u32,
}

fn default_slow_factor() -> u8 {
    3
}

/// A drought scheduled by an experiment (applied through ordinary regrowth).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptedDrought {
    pub start_day: u32,
    pub days: u32,
    pub center: [i32; 2],
    pub radius: u32,
    pub severity: f32,
}

/// Random droughts (accrete.md, "Random droughts") and optional seasons.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WeatherConfig {
    pub random_droughts: bool,
    /// Chance per day that a drought starts (when none is active and the
    /// cooldown has passed).
    pub daily_chance: f32,
    pub radius_min: u32,
    pub radius_max: u32,
    pub length_min_days: u32,
    pub length_max_days: u32,
    /// Fraction by which food regrowth falls inside the drought.
    pub severity_min: f32,
    pub severity_max: f32,
    pub cooldown_days: u32,
    pub scripted: Vec<ScriptedDrought>,
    /// Seasonal swing of food regrowth (0 = no seasons): regrowth is
    /// multiplied by `1 ± amplitude` along a triangle wave.
    pub seasons_amplitude: f32,
    pub season_length_days: u32,
}

impl Default for WeatherConfig {
    fn default() -> Self {
        Self {
            random_droughts: true,
            daily_chance: 1.0 / 150.0,
            radius_min: 6,
            radius_max: 16,
            length_min_days: 10,
            length_max_days: 40,
            severity_min: 0.3,
            severity_max: 0.7,
            cooldown_days: 60,
            scripted: Vec::new(),
            seasons_amplitude: 0.0,
            season_length_days: 60,
        }
    }
}

/// Social knowledge exchange, barter, and reciprocity (Week 3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SocialConfig {
    /// Agents on the same tile tell each other about food they know.
    pub share_enabled: bool,
    pub share_interval_ticks: u32,
    /// Food memories passed on per meeting: base + sociability × extra.
    pub share_items_base: u32,
    pub share_items_sociability: u32,
    pub trade_enabled: bool,
    pub reciprocity_enabled: bool,
    /// Minimum utility gain (after the time cost) to accept a trade.
    pub trade_min_gain: f32,
    /// Ticks a trade takes, priced at the agent's value of time.
    pub trade_ticks: f32,
    pub reciprocity_step: f32,
    /// Fraction of each reciprocity score that drifts back to neutral per day.
    pub reciprocity_decay_per_day: f32,
    pub social_slots: u32,
    /// Utility per unit of a good held below the agent's need for it.
    pub food_value: f32,
    pub material_value: f32,
    /// Utility per unit held beyond need.
    pub surplus_value: f32,
}

impl Default for SocialConfig {
    fn default() -> Self {
        Self {
            share_enabled: true,
            share_interval_ticks: 6,
            share_items_base: 1,
            share_items_sociability: 2,
            trade_enabled: true,
            reciprocity_enabled: true,
            trade_min_gain: 0.05,
            trade_ticks: 1.0,
            reciprocity_step: 0.15,
            reciprocity_decay_per_day: 0.02,
            social_slots: 32,
            food_value: 1.0,
            material_value: 0.6,
            surplus_value: 0.05,
        }
    }
}

/// Roads (`ImproveTrail`) and shelters (`ImproveShelter`) (Week 3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InfrastructureConfig {
    pub roads_enabled: bool,
    /// Weight on the household's crossings in the road-benefit rule
    /// (0 = strictly selfish agents).
    pub household_weight: f32,
    pub road_wood: f32,
    pub road_stone: f32,
    pub road_ticks: u32,
    pub road_condition: u32,
    /// Horizon over which a road's time savings are counted.
    pub road_horizon_days: f32,
    /// Ticks of gathering the materials represent (opportunity cost).
    pub road_material_ticks: f32,
    pub frequent_slots: u32,
    pub shelters_enabled: bool,
    pub shelter_wood: f32,
    pub shelter_stone: f32,
    pub shelter_ticks: u32,
    pub max_shelter: u32,
}

impl Default for InfrastructureConfig {
    fn default() -> Self {
        Self {
            roads_enabled: true,
            household_weight: 0.5,
            road_wood: 2.0,
            road_stone: 2.0,
            road_ticks: 4,
            road_condition: 180,
            road_horizon_days: 30.0,
            road_material_ticks: 6.0,
            frequent_slots: 16,
            shelters_enabled: true,
            shelter_wood: 20.0,
            shelter_stone: 10.0,
            shelter_ticks: 12,
            max_shelter: 2,
        }
    }
}

/// Worn trails (Week 3): emergent from traffic, never planned.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TrailConfig {
    /// Master switch for the "trails" experiment (traffic is still counted).
    pub enabled: bool,
    /// Daily traffic decay λ in `traffic_next = λ·traffic + crossings`.
    pub decay: f32,
    /// Traffic at or above which a tile wears into a trail.
    pub threshold: f32,
    /// Days below the threshold after which a trail fades back.
    pub fade_days: u32,
}

impl Default for TrailConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            decay: 0.95,
            threshold: 20.0,
            fade_days: 30,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SettlementConfig {
    pub min_homes: u32,
    pub min_residents: u32,
    pub radius: u32,
    pub persist_days: u32,
    pub village_homes: u32,
    pub village_residents: u32,
    pub town_homes: u32,
    pub town_residents: u32,
}

impl Default for SettlementConfig {
    fn default() -> Self {
        Self {
            min_homes: 5,
            min_residents: 15,
            radius: 3,
            persist_days: 60,
            village_homes: 15,
            village_residents: 50,
            town_homes: 50,
            town_residents: 150,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SimConfig {
    /// Event log ring-buffer capacity.
    pub event_log_capacity: u32,
    /// Run resource-conservation and bounds invariants every tick (slower).
    pub check_invariants: bool,
    /// Baseline agents for evaluation: "normal", "random", or "omniscient".
    pub agent_kind: AgentKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AgentKind {
    #[default]
    Normal,
    Random,
    Omniscient,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            event_log_capacity: 20_000,
            check_invariants: false,
            agent_kind: AgentKind::Normal,
        }
    }
}

// ---------------------------------------------------------------------------
// Unit conversion: the single place where per-day rates become per-tick.
// ---------------------------------------------------------------------------

/// Per-tick rates derived from the per-day configuration.
#[derive(Clone, Debug, PartialEq)]
pub struct TickRates {
    pub hunger_per_tick: f32,
    pub health_loss_deprived_per_tick: f32,
    pub health_loss_starving_per_tick: f32,
    pub health_recovery_per_tick: f32,
    pub frailty_per_deprived_tick: f32,
    pub frailty_per_crowded_tick: f32,
    pub age_decline_per_tick: f32,
    pub maturity_ticks: u64,
    pub elder_ticks: u64,
    pub reproduce_cooldown_ticks: u64,
    pub reproduce_retry_ticks: u64,
    pub crowding_grace_ticks: u64,
    pub settle_after_ticks: u64,
    pub freshness_half_life_ticks: f32,
}

pub fn days_to_ticks(days: f32) -> u64 {
    (days * TICKS_PER_DAY as f32).round().max(0.0) as u64
}

impl TickRates {
    pub fn new(c: &Config) -> Self {
        let per_tick = 1.0 / TICKS_PER_DAY as f32;
        Self {
            hunger_per_tick: c.needs.hunger_per_day * per_tick,
            health_loss_deprived_per_tick: c.needs.health_loss_deprived_per_day * per_tick,
            health_loss_starving_per_tick: c.needs.health_loss_starving_per_day * per_tick,
            health_recovery_per_tick: c.needs.health_recovery_per_day * per_tick,
            frailty_per_deprived_tick: c.life.frailty_per_deprived_day * per_tick,
            frailty_per_crowded_tick: c.life.frailty_per_crowded_day * per_tick,
            age_decline_per_tick: c.life.age_decline_per_day * per_tick,
            maturity_ticks: days_to_ticks(c.life.maturity_days),
            elder_ticks: days_to_ticks(c.life.elder_days),
            reproduce_cooldown_ticks: days_to_ticks(c.life.reproduce_cooldown_days),
            reproduce_retry_ticks: days_to_ticks(c.life.reproduce_retry_days),
            crowding_grace_ticks: days_to_ticks(c.homes.crowding_grace_days),
            settle_after_ticks: days_to_ticks(c.homes.settle_after_days),
            freshness_half_life_ticks: c.memory.freshness_half_life_days * TICKS_PER_DAY as f32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_toml_matches_defaults() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("configs")
            .join("default.toml");
        let text = std::fs::read_to_string(&path).expect("configs/default.toml exists");
        let parsed = Config::from_toml_str(&text).expect("default.toml parses");
        assert_eq!(
            parsed,
            Config::default(),
            "configs/default.toml drifted from Config::default()"
        );
    }

    #[test]
    fn validation_rejects_settings_no_run_can_use() {
        let ok = Config::default();
        assert_eq!(ok.validate(), Ok(()));
        type Breaks = fn(&mut Config);
        let bad: [(&str, Breaks); 13] = [
            ("food_regrowth", |c| c.resources.food_regrowth = f32::NAN),
            ("food_regrowth", |c| {
                c.resources.food_regrowth = f32::INFINITY
            }),
            ("food_regrowth", |c| c.resources.food_regrowth = -0.1),
            ("reproduce_chance", |c| c.life.reproduce_chance = 1.5),
            ("hill_fraction", |c| {
                c.world.ocean_fraction = 0.6;
                c.world.hill_fraction = 0.6;
            }),
            ("stone_min", |c| c.resources.stone_min = 300),
            ("aptitude_min", |c| c.skills.aptitude_min = 2.0),
            ("radius_min", |c| c.weather.radius_min = 99),
            ("maturity_days", |c| c.life.elder_days = 0.0),
            ("world.width", |c| c.world.width = 0),
            ("min_mp", |c| c.movement.min_mp = 0),
            ("disruption_slots", |c| c.memory.disruption_slots = 0),
            ("time_cost", |c| c.decision.time_cost = f32::NAN),
        ];
        for (what, f) in bad {
            let mut c = ok.clone();
            f(&mut c);
            let e = c.validate().unwrap_err();
            assert!(e.contains(what), "{what}: {e}");
            assert!(
                Config::from_toml_str(&c.to_toml_string()).is_err(),
                "{what}"
            );
        }
        // Utility weights are preferences: negative is allowed, NaN is not.
        let mut c = ok.clone();
        c.decision.social_weight = -1.0;
        c.infrastructure.household_weight = -0.5;
        assert_eq!(c.validate(), Ok(()));
    }

    #[test]
    fn toml_roundtrip() {
        let c = Config::default();
        let back = Config::from_toml_str(&c.to_toml_string()).unwrap();
        assert_eq!(back, c);
        assert_eq!(back.digest(), c.digest());
    }

    #[test]
    fn unknown_fields_rejected() {
        assert!(Config::from_toml_str("[world]\nwidht = 3\n").is_err());
    }

    #[test]
    fn rates_convert_days() {
        let r = TickRates::new(&Config::default());
        assert_eq!(r.maturity_ticks, 960);
        assert!((r.hunger_per_tick * 24.0 - 0.33).abs() < 1e-6);
    }
}
