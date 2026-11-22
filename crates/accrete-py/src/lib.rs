//! Python bindings for the Accrete RL environment (`accrete_rl._core`).
//!
//! A thin layer over `accrete_core::rl::AccreteEnv`: configuration comes in as
//! TOML strings, observations go out as NumPy arrays. All simulation logic,
//! determinism, and validation live in the Rust core; the Gymnasium wrapper
//! is in `python/accrete_rl/env.py`.

// PyO3 0.22's `#[pymethods]` expansion converts `PyErr` into itself.
#![allow(clippy::useless_conversion)]

use accrete_core::rl::observation::{GRID_CHANNEL_NAMES, SLOT_NAMES, VECTOR_NAMES};
use accrete_core::rl::{
    actions, AccreteEnv, EnvError, EpisodeSummary, Observation, RlConfig, StepInfo, GRID_CHANNELS,
    GRID_SIDE, N_ACTIONS, SLOT_FEATURES, VECTOR_SIZE,
};
use accrete_core::Config;
use numpy::ndarray::{Array2, Array3};
use numpy::{IntoPyArray, PyArray1};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;

fn env_err(e: EnvError) -> PyErr {
    match e {
        EnvError::InvalidAction(_) | EnvError::MaskedAction(_) => {
            PyValueError::new_err(e.to_string())
        }
        _ => PyRuntimeError::new_err(e.to_string()),
    }
}

fn obs_dict(py: Python<'_>, obs: Observation) -> PyResult<Bound<'_, PyDict>> {
    let d = PyDict::new_bound(py);
    let grid = Array3::from_shape_vec((GRID_CHANNELS, GRID_SIDE, GRID_SIDE), obs.grid)
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    let slots = Array2::from_shape_vec((N_ACTIONS, SLOT_FEATURES), obs.slots)
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    let mask: Vec<i8> = obs.mask.iter().map(|&m| i8::from(m)).collect();
    d.set_item("grid", grid.into_pyarray_bound(py))?;
    d.set_item("vector", PyArray1::from_vec_bound(py, obs.vector))?;
    d.set_item("slots", slots.into_pyarray_bound(py))?;
    d.set_item("action_mask", PyArray1::from_vec_bound(py, mask))?;
    Ok(d)
}

fn info_dict<'py>(py: Python<'py>, info: &StepInfo) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new_bound(py);
    d.set_item("ticks", info.ticks)?;
    d.set_item("day", info.day)?;
    d.set_item("agent", info.agent)?;
    d.set_item("goal", info.kind)?;
    d.set_item("route_failed", info.route_failed)?;
    d.set_item("births", info.births)?;
    d.set_item("deprived_ticks", info.deprived_ticks)?;
    d.set_item("death", info.death)?;
    d.set_item("ended_by", info.ended_by)?;
    Ok(d)
}

fn summary_dict<'py>(py: Python<'py>, s: &EpisodeSummary) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new_bound(py);
    d.set_item("seed", s.seed)?;
    d.set_item("agent", s.agent)?;
    d.set_item("episode_return", s.episode_return)?;
    d.set_item("steps", s.steps)?;
    d.set_item("days_alive", s.days_alive)?;
    d.set_item("births", s.births)?;
    d.set_item("deprived_days", s.deprived_days)?;
    d.set_item("death", s.death)?;
    Ok(d)
}

/// One learning agent in a scripted Accrete society.
///
/// `sim_config` and `rl_config` are TOML strings (see `default_sim_config()`
/// and `default_rl_config()`); omitted values take their defaults.
#[pyclass(module = "accrete_rl._core", name = "Env")]
struct PyEnv {
    inner: AccreteEnv,
}

#[pymethods]
impl PyEnv {
    #[new]
    #[pyo3(signature = (sim_config=None, rl_config=None))]
    fn new(sim_config: Option<&str>, rl_config: Option<&str>) -> PyResult<Self> {
        let sim = match sim_config {
            Some(s) => Config::from_toml_str(s).map_err(PyValueError::new_err)?,
            None => Config::default(),
        };
        let rl = match rl_config {
            Some(s) => RlConfig::from_toml_str(s).map_err(PyValueError::new_err)?,
            None => RlConfig::default(),
        };
        Ok(Self {
            inner: AccreteEnv::new(sim, rl),
        })
    }

    /// Start an episode on world `seed`; returns the first observation.
    fn reset<'py>(&mut self, py: Python<'py>, seed: u64) -> PyResult<Bound<'py, PyDict>> {
        // The simulation runs without the GIL, so Python threads can step
        // several environments in parallel.
        let inner = &mut self.inner;
        let obs = py
            .allow_threads(move || inner.reset(seed))
            .map_err(env_err)?;
        obs_dict(py, obs)
    }

    /// Run one macro step. Returns `(obs, reward, terminated, truncated, info)`.
    #[allow(clippy::type_complexity)]
    fn step<'py>(
        &mut self,
        py: Python<'py>,
        action: usize,
    ) -> PyResult<(Bound<'py, PyDict>, f32, bool, bool, Bound<'py, PyDict>)> {
        let inner = &mut self.inner;
        let r = py
            .allow_threads(move || inner.step(action))
            .map_err(env_err)?;
        let info = info_dict(py, &r.info)?;
        Ok((
            obs_dict(py, r.obs)?,
            r.reward,
            r.terminated,
            r.truncated,
            info,
        ))
    }

    fn action_mask<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray1<i8>> {
        let m: Vec<i8> = self
            .inner
            .action_mask()
            .iter()
            .map(|&b| i8::from(b))
            .collect();
        PyArray1::from_vec_bound(py, m)
    }

    /// The action the agent's own scripted logic would take now.
    fn heuristic_action(&self) -> usize {
        self.inner.heuristic_action()
    }

    /// The same episode with the agent left to its scripted logic (baseline).
    fn native_episode<'py>(&self, py: Python<'py>, seed: u64) -> PyResult<Bound<'py, PyDict>> {
        let inner = &self.inner;
        let s = py
            .allow_threads(move || inner.native_episode(seed))
            .map_err(env_err)?;
        summary_dict(py, &s)
    }

    /// Describe the option behind an action slot, e.g. `gather-food -> (12, 40), eta 6.0`.
    fn describe_action(&self, action: usize) -> Option<String> {
        self.inner.slot(action).map(|c| {
            format!(
                "{} -> ({}, {}), eta {:.1}, scripted utility {:+.2}",
                c.kind.name(),
                c.target.x,
                c.target.y,
                c.est_ticks,
                c.score
            )
        })
    }

    /// Current simulation status (tick, day, population, controlled agent).
    fn status<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new_bound(py);
        if let Some(sim) = self.inner.sim() {
            d.set_item("tick", sim.tick)?;
            d.set_item("day", sim.day())?;
            d.set_item("population", sim.population())?;
            d.set_item("agent", self.inner.agent_id())?;
            if let Some(a) = sim.agent(self.inner.agent_id()) {
                d.set_item("pos", (a.pos.x, a.pos.y))?;
                d.set_item("hunger", a.hunger)?;
                d.set_item("energy", a.energy)?;
                d.set_item("health", a.health)?;
                d.set_item("home", a.home)?;
                d.set_item("goal", a.goal.kind.name())?;
            }
        }
        Ok(d)
    }

    #[getter]
    fn agent_id(&self) -> u32 {
        self.inner.agent_id()
    }
}

#[pyfunction]
fn default_sim_config() -> String {
    Config::default().to_toml_string()
}

#[pyfunction]
fn default_rl_config() -> String {
    toml::to_string_pretty(&RlConfig::default()).expect("serializes")
}

#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyEnv>()?;
    m.add_function(wrap_pyfunction!(default_sim_config, m)?)?;
    m.add_function(wrap_pyfunction!(default_rl_config, m)?)?;
    m.add("N_ACTIONS", N_ACTIONS)?;
    m.add("GRID_CHANNELS", GRID_CHANNELS)?;
    m.add("GRID_SIDE", GRID_SIDE)?;
    m.add("VECTOR_SIZE", VECTOR_SIZE)?;
    m.add("SLOT_FEATURES", SLOT_FEATURES)?;
    m.add("ACTION_NAMES", actions::action_names())?;
    m.add("GRID_CHANNEL_NAMES", GRID_CHANNEL_NAMES.to_vec())?;
    m.add("VECTOR_NAMES", VECTOR_NAMES.to_vec())?;
    m.add("SLOT_NAMES", SLOT_NAMES.to_vec())?;
    Ok(())
}
