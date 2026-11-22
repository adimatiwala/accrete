//! `accrete rl-eval`: baseline policies in the RL environment.

use accrete_core::Config;
use accrete_core::rl::{AccreteEnv, EpisodeSummary, RlConfig};
use accrete_core::rng::{Rng, Stream};

pub fn rl_eval(config: &Config, rl: &RlConfig, policies: &[String], seeds: &[u64]) {
    println!(
        "{:<10} {:>8} {:>9} {:>9} {:>7} {:>9} {:>7}",
        "policy", "return", "days", "deprived", "births", "deaths", "steps"
    );
    for policy in policies {
        let mut env = AccreteEnv::new(config.clone(), rl.clone());
        let mut runs: Vec<EpisodeSummary> = Vec::new();
        for &seed in seeds {
            let s = match policy.as_str() {
                "heuristic" => env.run_episode(seed, |e, _| e.heuristic_action()),
                "random" => {
                    let mut rng = Rng::for_stream(seed, Stream::Baseline);
                    rng.next_u64();
                    env.run_episode(seed, move |_, obs| {
                        let valid: Vec<usize> =
                            (0..obs.mask.len()).filter(|&a| obs.mask[a]).collect();
                        valid[rng.below(valid.len() as u32) as usize]
                    })
                }
                "native" => env.native_episode(seed),
                other => panic!("unknown policy {other} (heuristic, random, native)"),
            }
            .unwrap_or_else(|e| panic!("seed {seed}: {e}"));
            runs.push(s);
        }
        let n = runs.len() as f32;
        let mean = |f: &dyn Fn(&EpisodeSummary) -> f32| runs.iter().map(f).sum::<f32>() / n;
        println!(
            "{:<10} {:>8.2} {:>9.1} {:>9.2} {:>7.2} {:>8}/{:<1} {:>7.0}",
            policy,
            mean(&|r| r.episode_return),
            mean(&|r| r.days_alive),
            mean(&|r| r.deprived_days),
            mean(&|r| r.births as f32),
            runs.iter().filter(|r| r.death.is_some()).count(),
            runs.len(),
            mean(&|r| r.steps as f32),
        );
    }
}
