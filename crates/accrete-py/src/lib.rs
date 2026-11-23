// PyO3 0.22 generates a PyErr-to-PyErr conversion in the method wrapper.
#![allow(clippy::useless_conversion)]

use accrete_core::rl::{
    Household, LearnedWorld, Observation, Weather, ACTIONS, MAX_DAYS, STATE_FIELDS, TARGET_FIELDS,
    TASK_VERSION,
};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

type PyObservation = (Vec<f32>, Vec<Vec<f32>>, Vec<bool>);
type TileView = (u8, u8, f32, f32);
type HomeView = (u32, i32, i32, f32);
type ResidentView = (u32, i32, i32, f32, f32, f32, f32);
type WorldView = (
    u64,
    u32,
    u32,
    Vec<TileView>,
    Vec<HomeView>,
    Vec<ResidentView>,
    u64,
);

fn py_observation(obs: &Observation) -> PyObservation {
    (
        obs.state.to_vec(),
        obs.targets.iter().map(|row| row.to_vec()).collect(),
        obs.mask.to_vec(),
    )
}

#[pyclass]
struct NativeHousehold {
    env: Household,
}

#[pymethods]
impl NativeHousehold {
    #[new]
    fn new(py: Python<'_>, seed: u64, days: u32, weather: &str, check: bool) -> PyResult<Self> {
        let weather = Weather::parse(weather).map_err(PyValueError::new_err)?;
        let env = py
            .allow_threads(|| Household::new(seed, weather, days, check))
            .map_err(PyValueError::new_err)?;
        Ok(Self { env })
    }

    fn observation(&self) -> PyObservation {
        py_observation(self.env.observation())
    }

    fn step(&mut self, py: Python<'_>, action: usize) -> PyResult<(f32, bool, bool, bool)> {
        let t = py
            .allow_threads(|| self.env.step(action))
            .map_err(PyValueError::new_err)?;
        Ok((t.reward, t.terminated, t.truncated, t.masked))
    }

    fn reserve_action(&self) -> usize {
        self.env.reserve_action()
    }

    fn score(&self) -> (u64, usize, bool, u64, u64, u32, f32) {
        let score = self.env.score();
        (
            score.ticks,
            score.alive,
            score.focal_alive,
            score.deprived_hours,
            score.focal_trades,
            score.tiles_moved,
            score.home_food,
        )
    }
}

#[pyclass]
struct NativeLearnedWorld {
    world: LearnedWorld,
}

#[pymethods]
impl NativeLearnedWorld {
    #[new]
    fn new(seed: u64, weather: &str, check: bool) -> PyResult<Self> {
        let weather = Weather::parse(weather).map_err(PyValueError::new_err)?;
        Ok(Self {
            world: LearnedWorld::new(seed, weather, check).map_err(PyValueError::new_err)?,
        })
    }

    fn observations(&self) -> Vec<(u32, PyObservation)> {
        self.world
            .observations()
            .map(|(id, obs)| (id, py_observation(obs)))
            .collect()
    }

    fn step(&mut self, py: Python<'_>, actions: Vec<usize>) -> PyResult<()> {
        py.allow_threads(|| self.world.step(&actions))
            .map_err(PyValueError::new_err)
    }

    fn display(&self) -> WorldView {
        let sim = self.world.simulation();
        let tiles = sim
            .world
            .tiles
            .iter()
            .map(|tile| {
                (
                    tile.biome as u8,
                    tile.path_level as u8,
                    tile.food,
                    tile.wood,
                )
            })
            .collect();
        let homes = sim
            .world
            .homes
            .iter()
            .map(|home| (home.id, home.pos.x, home.pos.y, home.stored().food))
            .collect();
        let agents = sim
            .agents
            .iter()
            .map(|agent| {
                (
                    agent.id,
                    agent.pos.x,
                    agent.pos.y,
                    agent.hunger,
                    agent.energy,
                    agent.health,
                    agent.inventory.food,
                )
            })
            .collect();
        let trades = sim
            .metrics
            .iter()
            .map(|row| u64::from(row.trades))
            .sum::<u64>()
            + u64::from(sim.today.trades);
        (
            sim.tick,
            sim.world.dims.width,
            sim.world.dims.height,
            tiles,
            homes,
            agents,
            trades,
        )
    }
}

#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<NativeHousehold>()?;
    m.add_class::<NativeLearnedWorld>()?;
    m.add("ACTIONS", ACTIONS.to_vec())?;
    m.add("STATE_FIELDS", STATE_FIELDS.to_vec())?;
    m.add("TARGET_FIELDS", TARGET_FIELDS.to_vec())?;
    m.add("MAX_DAYS", MAX_DAYS)?;
    m.add("TASK_VERSION", TASK_VERSION)?;
    Ok(())
}
