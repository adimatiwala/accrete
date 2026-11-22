//! Daily metrics, exported as CSV/JSON by the CLI and plotted by the viewer.

use serde::{Deserialize, Serialize};

use crate::agents::agent::GoalKind;

/// Counters accumulated during the current day and reset at its end.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DayCounters {
    pub births: u32,
    pub deaths_deprivation: u32,
    pub deaths_old_age: u32,
    pub food_gathered: f64,
    pub food_eaten: f64,
    pub tiles_moved: u64,
    pub route_failures: u32,
    pub astar_searches: u64,
    pub astar_expansions: u64,
    pub homes_founded: u32,
    pub home_joins: u32,
    pub home_moves: u32,
    pub storage_built: u32,
    pub blocked_births: u32,
    pub trails_formed: u32,
    pub trails_faded: u32,
    pub tips: u32,
    pub trades: u32,
    pub food_traded: f64,
    pub roads_built: u32,
    pub shelters_built: u32,
    /// Sum and count of the distance from home of each food gathering tick.
    pub forage_distance_sum: f64,
    pub forage_distance_n: u64,
    pub goal_ticks: [u64; GoalKind::COUNT],
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DailyMetrics {
    pub day: u32,
    pub population: u32,
    pub adults: u32,
    pub juveniles: u32,
    pub elders: u32,
    pub births: u32,
    pub deaths_deprivation: u32,
    pub deaths_old_age: u32,
    pub total_births: u32,
    pub total_deaths_deprivation: u32,
    pub total_deaths_old_age: u32,
    pub mean_hunger: f32,
    pub mean_energy: f32,
    pub mean_health: f32,
    pub mean_frailty: f32,
    pub mean_age_days: f32,
    pub mean_lifespan_days: f32,
    pub homed_fraction: f32,
    pub homes_occupied: u32,
    pub homes_vacant: u32,
    pub mean_residents: f32,
    pub chests: u32,
    pub food_world: f32,
    pub food_carried: f32,
    pub food_stored: f32,
    pub wood_stored: f32,
    pub mean_known_tiles: f32,
    pub food_gathered: f32,
    pub food_eaten: f32,
    pub tiles_moved: u64,
    /// Tiles walked per unit of food gathered today (lower = more efficient).
    pub travel_per_food: f32,
    pub route_failures: u32,
    pub astar_expansions_per_search: f32,
    pub homes_founded: u32,
    pub home_joins: u32,
    pub home_moves: u32,
    pub storage_built: u32,
    pub trail_tiles: u32,
    pub road_tiles: u32,
    pub shelters: u32,
    pub trades: u32,
    pub food_traded: f32,
    pub tips_shared: u32,
    /// Mean distance (tiles) from home at which food was gathered today.
    pub forage_radius: f32,
    /// Mean experienced foraging return (food per trip tick) of adults.
    pub mean_forage_rate: f32,
    /// Clark-Evans nearest-neighbour ratio of occupied homes: 1 = random,
    /// < 1 = clustered, > 1 = evenly spaced.
    pub clark_evans: f32,
    /// Standing food near homes (within 3 tiles) relative to elsewhere, as a
    /// fraction of capacity: < 1 means settlements deplete their surroundings.
    pub depletion_ratio: f32,
    /// Mean dominance of an adult's most-practised skill (0 = generalist, 1 = specialist).
    pub specialization: f32,
    /// Mean age in days of occupied homes.
    pub mean_home_age_days: f32,
    /// Share of land tiles under an active drought.
    pub drought_area: f32,
    /// Tiles under an active route disruption.
    pub disrupted_tiles: u32,
    pub settlement_candidates: u32,
    pub hamlets: u32,
    pub villages: u32,
    pub towns: u32,
    pub largest_settlement_residents: u32,
    pub pathology_oscillating: u64,
    pub pathology_stuck: u64,
    pub pathology_starved_beside_food: u64,
    pub pathology_idle: u64,
    /// Share of agent-ticks today spent on each goal kind (`GoalKind::ALL` order).
    pub goal_share: Vec<f32>,
    pub at_cap: bool,
}

impl DailyMetrics {
    pub fn csv_header() -> String {
        let mut cols: Vec<String> = [
            "day",
            "population",
            "adults",
            "juveniles",
            "elders",
            "births",
            "deaths_deprivation",
            "deaths_old_age",
            "total_births",
            "total_deaths_deprivation",
            "total_deaths_old_age",
            "mean_hunger",
            "mean_energy",
            "mean_health",
            "mean_frailty",
            "mean_age_days",
            "mean_lifespan_days",
            "homed_fraction",
            "homes_occupied",
            "homes_vacant",
            "mean_residents",
            "chests",
            "food_world",
            "food_carried",
            "food_stored",
            "wood_stored",
            "mean_known_tiles",
            "food_gathered",
            "food_eaten",
            "tiles_moved",
            "travel_per_food",
            "route_failures",
            "astar_expansions_per_search",
            "homes_founded",
            "home_joins",
            "home_moves",
            "storage_built",
            "trail_tiles",
            "road_tiles",
            "shelters",
            "trades",
            "food_traded",
            "tips_shared",
            "forage_radius",
            "mean_forage_rate",
            "clark_evans",
            "depletion_ratio",
            "specialization",
            "mean_home_age_days",
            "drought_area",
            "disrupted_tiles",
            "settlement_candidates",
            "hamlets",
            "villages",
            "towns",
            "largest_settlement_residents",
            "pathology_oscillating",
            "pathology_stuck",
            "pathology_starved_beside_food",
            "pathology_idle",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        for k in GoalKind::ALL {
            cols.push(format!("goal_{}", k.name().replace('-', "_")));
        }
        cols.push("at_cap".into());
        cols.join(",")
    }

    pub fn csv_row(&self) -> String {
        let mut v: Vec<String> = vec![
            self.day.to_string(),
            self.population.to_string(),
            self.adults.to_string(),
            self.juveniles.to_string(),
            self.elders.to_string(),
            self.births.to_string(),
            self.deaths_deprivation.to_string(),
            self.deaths_old_age.to_string(),
            self.total_births.to_string(),
            self.total_deaths_deprivation.to_string(),
            self.total_deaths_old_age.to_string(),
            format!("{:.4}", self.mean_hunger),
            format!("{:.4}", self.mean_energy),
            format!("{:.4}", self.mean_health),
            format!("{:.4}", self.mean_frailty),
            format!("{:.2}", self.mean_age_days),
            format!("{:.2}", self.mean_lifespan_days),
            format!("{:.4}", self.homed_fraction),
            self.homes_occupied.to_string(),
            self.homes_vacant.to_string(),
            format!("{:.3}", self.mean_residents),
            self.chests.to_string(),
            format!("{:.1}", self.food_world),
            format!("{:.1}", self.food_carried),
            format!("{:.1}", self.food_stored),
            format!("{:.1}", self.wood_stored),
            format!("{:.1}", self.mean_known_tiles),
            format!("{:.1}", self.food_gathered),
            format!("{:.1}", self.food_eaten),
            self.tiles_moved.to_string(),
            format!("{:.4}", self.travel_per_food),
            self.route_failures.to_string(),
            format!("{:.1}", self.astar_expansions_per_search),
            self.homes_founded.to_string(),
            self.home_joins.to_string(),
            self.home_moves.to_string(),
            self.storage_built.to_string(),
            self.trail_tiles.to_string(),
            self.road_tiles.to_string(),
            self.shelters.to_string(),
            self.trades.to_string(),
            format!("{:.1}", self.food_traded),
            self.tips_shared.to_string(),
            format!("{:.3}", self.forage_radius),
            format!("{:.3}", self.mean_forage_rate),
            format!("{:.3}", self.clark_evans),
            format!("{:.3}", self.depletion_ratio),
            format!("{:.3}", self.specialization),
            format!("{:.1}", self.mean_home_age_days),
            format!("{:.4}", self.drought_area),
            self.disrupted_tiles.to_string(),
            self.settlement_candidates.to_string(),
            self.hamlets.to_string(),
            self.villages.to_string(),
            self.towns.to_string(),
            self.largest_settlement_residents.to_string(),
            self.pathology_oscillating.to_string(),
            self.pathology_stuck.to_string(),
            self.pathology_starved_beside_food.to_string(),
            self.pathology_idle.to_string(),
        ];
        for s in &self.goal_share {
            v.push(format!("{s:.4}"));
        }
        v.push(self.at_cap.to_string());
        v.join(",")
    }
}
