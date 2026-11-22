//! End-to-end tests of the `accrete` binary: manifests, `reproduce`, and
//! determinism across `--jobs`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn accrete(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_accrete"))
        .args(args)
        .output()
        .expect("run accrete")
}

fn tmp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("accrete-cli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

fn ok(o: &Output) {
    assert!(
        o.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn reproduce_exits_zero_for_a_faithful_manifest_and_one_for_a_wrong_one() {
    let dir = tmp("repro");
    ok(&accrete(&[
        "run",
        "--seed",
        "3",
        "--days",
        "2",
        "--every",
        "0",
        "-o",
        s(&dir),
    ]));
    let m = dir.join("run_seed3_manifest.json");
    let good = accrete(&["reproduce", s(&m)]);
    ok(&good);
    assert!(String::from_utf8_lossy(&good.stdout).contains("reproduced"));
    let text = std::fs::read_to_string(&m).unwrap();
    std::fs::write(
        &m,
        text.replace("\"ticks\": 48", "\"ticks\": 47")
            .replace("\"final_tick\": 48", "\"final_tick\": 47"),
    )
    .unwrap();
    assert_eq!(accrete(&["reproduce", s(&m)]).status.code(), Some(1));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn every_output_is_identical_whatever_the_number_of_jobs() {
    let run = |jobs: &str| {
        let dir = tmp(&format!("jobs{jobs}"));
        ok(&accrete(&[
            "experiments",
            "--seed",
            "1-3",
            "--days",
            "3",
            "--every",
            "0",
            "--only",
            "trade",
            "--jobs",
            jobs,
            "-o",
            s(&dir),
        ]));
        let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter_map(|p| {
                let name = p.file_name()?.to_str()?.to_string();
                // runs.sqlite (and WAL/SHM) are not byte-stable across --jobs.
                if name == "runs.sqlite" || name.starts_with("runs.sqlite-") {
                    return None;
                }
                Some((name, std::fs::read(&p).unwrap()))
            })
            .collect();
        files.sort();
        assert!(dir.join("runs.sqlite").is_file(), "auto-index present");
        let _ = std::fs::remove_dir_all(&dir);
        files
    };
    let (a, b) = (run("1"), run("3"));
    assert_eq!(a.len(), b.len());
    assert!(a.iter().any(|(n, _)| n.ends_with("_manifest.json")));
    // Manifests record the output folder, which differs between the two
    // runs here, so compare them with that path removed.
    for ((na, ca), (nb, cb)) in a.iter().zip(&b) {
        assert_eq!(na, nb);
        let norm = |c: &[u8], tag: &str| {
            String::from_utf8_lossy(c).replace(&format!("jobs{tag}"), "jobsN")
        };
        assert_eq!(
            norm(ca, "1"),
            norm(cb, "3"),
            "{na} differs between --jobs 1 and 3"
        );
    }
}

#[test]
fn saving_over_the_loaded_snapshot_is_refused_before_running() {
    let dir = tmp("saveload");
    let snap = dir.join("a.snap.json");
    ok(&accrete(&[
        "run",
        "--seed",
        "1",
        "--days",
        "1",
        "--every",
        "0",
        "-o",
        s(&dir),
        "--save",
        s(&snap),
    ]));
    let before = std::fs::read(&snap).unwrap();
    let o = accrete(&[
        "run",
        "--load",
        s(&snap),
        "--days",
        "1",
        "-o",
        s(&dir),
        "--save",
        s(&snap),
    ]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("--save must differ from --load"));
    assert_eq!(std::fs::read(&snap).unwrap(), before, "snapshot untouched");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_moved_results_folder_and_both_branch_arms_reproduce() {
    let root = tmp("moved");
    let (snaps, out) = (root.join("snaps"), root.join("out"));
    let snap = snaps.join("a.snap.json");
    ok(&accrete(&[
        "run",
        "--seed",
        "2",
        "--days",
        "2",
        "--every",
        "0",
        "-o",
        s(&snaps),
        "--save",
        s(&snap),
    ]));
    ok(&accrete(&[
        "run",
        "--load",
        s(&snap),
        "--days",
        "1",
        "--every",
        "0",
        "--label",
        "r2",
        "-o",
        s(&out),
    ]));
    ok(&accrete(&[
        "branch",
        "--load",
        s(&snap),
        "--days",
        "1",
        "--every",
        "0",
        "--drought",
        "2:3:64:64:20:0.5",
        "--label-b",
        "dry",
        "-o",
        s(&out),
    ]));
    let moved = root.with_file_name(format!(
        "{}-moved",
        root.file_name().unwrap().to_string_lossy()
    ));
    let _ = std::fs::remove_dir_all(&moved);
    std::fs::rename(&root, &moved).unwrap();
    for m in [
        "r2_seed2_manifest.json",
        "control_seed2_manifest.json",
        "dry_seed2_manifest.json",
    ] {
        ok(&accrete(&["reproduce", s(&moved.join("out").join(m))]));
    }
    let _ = std::fs::remove_dir_all(&moved);
}

#[test]
fn jobs_zero_is_rejected() {
    let dir = tmp("jobs0");
    let o = accrete(&[
        "experiments",
        "--seed",
        "1",
        "--days",
        "1",
        "--jobs",
        "0",
        "-o",
        s(&dir),
    ]);
    assert_eq!(o.status.code(), Some(2));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sweeps_reject_jobs_zero_and_bad_points_before_running() {
    let dir = tmp("sweepbad");
    for args in [
        vec![
            "sweep",
            "--param",
            "resources.food_regrowth=0.1",
            "--jobs",
            "0",
        ],
        vec!["sweep", "--param", "resources.food_regrowth=x"],
        vec!["sweep", "--param", "nope.key=1"],
        // Semantically impossible, non-finite, and no-op points.
        vec!["sweep", "--param", "resources.stone_min=100,300"],
        vec!["sweep", "--param", "world.width=0,64"],
        vec!["sweep", "--param", "resources.food_regrowth=0.1,nan"],
        vec!["sweep", "--param", "sim.check_invariants=false,true"],
        vec![
            "sweep",
            "--lhs",
            "3",
            "--param",
            "resources.food_regrowth=-1..0.5",
        ],
        vec![
            "sweep",
            "--set",
            "resources.food_regrowth=inf",
            "--param",
            "spawn.agents=50",
        ],
    ] {
        let mut a = args.clone();
        a.extend(["--days", "1", "-o", s(&dir)]);
        let o = accrete(&a);
        assert_eq!(o.status.code(), Some(2), "{args:?}: {o:?}");
    }
    assert!(!dir.join("sweep_runs.csv").exists(), "nothing ran");
    assert!(!dir.join("sweep_design.json").exists(), "nothing ran");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_different_run_cannot_overwrite_an_existing_manifest() {
    let dir = tmp("manifestow");
    ok(&accrete(&[
        "run",
        "--seed",
        "1",
        "--days",
        "2",
        "--label",
        "run",
        "-o",
        s(&dir),
    ]));
    let m = dir.join("run_seed1_manifest.json");
    assert!(m.is_file());
    // Same label into the same folder for more days: a different run.
    let o = accrete(&[
        "run",
        "--seed",
        "1",
        "--days",
        "3",
        "--label",
        "run",
        "-o",
        s(&dir),
    ]);
    assert_ne!(o.status.code(), Some(0), "{o:?}");
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.contains("already records a different run") || err.contains("panic"),
        "{err}"
    );
    // The original manifest still reproduces.
    ok(&accrete(&["reproduce", s(&m)]));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn branch_rejects_a_silent_global_config() {
    let dir = tmp("branchcfg");
    let snap = dir.join("s.snap.json");
    ok(&accrete(&[
        "run",
        "--seed",
        "1",
        "--days",
        "2",
        "--save",
        s(&snap),
        "-o",
        s(&dir),
    ]));
    let cfg = dir.join("c.toml");
    std::fs::write(&cfg, "resources.food_regrowth = 0.01\n").unwrap();
    let o = accrete(&[
        "--config",
        s(&cfg),
        "branch",
        "--load",
        s(&snap),
        "--days",
        "1",
        "-o",
        s(&dir),
    ]);
    assert_eq!(o.status.code(), Some(2), "{o:?}");
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("--config-b"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_sweep_run_reproduces_from_its_manifest() {
    let dir = tmp("sweeprepro");
    ok(&accrete(&[
        "sweep",
        "--lhs",
        "2",
        "--param",
        "resources.food_regrowth=0.02..0.2",
        "--param",
        "sight.radius=4..9",
        "--seed",
        "3",
        "--days",
        "2",
        "-o",
        s(&dir),
    ]));
    for k in ["p000", "p001"] {
        let m = dir.join(format!("{k}_seed3_manifest.json"));
        ok(&accrete(&["reproduce", s(&m)]));
    }
    let design = std::fs::read_to_string(dir.join("sweep_design.json")).unwrap();
    assert!(design.contains("\"base_config\""), "{design}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn longer_days_with_the_same_label_is_refused_before_clobbering() {
    let dir = tmp("daysclash");
    ok(&accrete(&[
        "run",
        "--seed",
        "1",
        "--days",
        "2",
        "--every",
        "0",
        "--save-every",
        "1",
        "-o",
        s(&dir),
        "--label",
        "clash",
    ]));
    let snap = dir.join("clash_seed1_day001.snap.json");
    let before = std::fs::read(&snap).unwrap();
    let o = accrete(&[
        "run",
        "--seed",
        "1",
        "--days",
        "4",
        "--every",
        "0",
        "--save-every",
        "1",
        "-o",
        s(&dir),
        "--label",
        "clash",
    ]);
    assert_ne!(o.status.code(), Some(0), "{o:?}");
    let err = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stderr),
        String::from_utf8_lossy(&o.stdout)
    );
    assert!(err.contains("already records a different run"), "{err}");
    assert_eq!(std::fs::read(&snap).unwrap(), before, "snap not clobbered");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_corrupt_manifest_is_refused_instead_of_overwritten() {
    let dir = tmp("corruptm");
    ok(&accrete(&[
        "run",
        "--seed",
        "1",
        "--days",
        "1",
        "--every",
        "0",
        "-o",
        s(&dir),
    ]));
    let m = dir.join("run_seed1_manifest.json");
    std::fs::write(&m, "{not json").unwrap();
    let o = accrete(&[
        "run",
        "--seed",
        "1",
        "--days",
        "1",
        "--every",
        "0",
        "-o",
        s(&dir),
    ]);
    assert_ne!(o.status.code(), Some(0), "{o:?}");
    let err = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stderr),
        String::from_utf8_lossy(&o.stdout)
    );
    assert!(err.contains("cannot be read"), "{err}");
    assert_eq!(std::fs::read_to_string(&m).unwrap(), "{not json");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_indexes_itself_and_save_every_writes_periodic_snapshots() {
    let dir = tmp("index");
    ok(&accrete(&[
        "run",
        "--seed",
        "4",
        "--days",
        "4",
        "--every",
        "0",
        "--save-every",
        "2",
        "-o",
        s(&dir),
        "--label",
        "idx",
    ]));
    let db = dir.join("runs.sqlite");
    assert!(db.is_file(), "auto-index after run");
    assert!(dir.join("idx_seed4_day002.snap.json").is_file());
    assert!(dir.join("idx_seed4_day004.snap.json").is_file());
    let m = std::fs::read_to_string(dir.join("idx_seed4_manifest.json")).unwrap();
    assert!(m.contains("idx_seed4_day002.snap.json"), "{m}");
    ok(&accrete(&[
        "run",
        "--seed",
        "5",
        "--days",
        "2",
        "--every",
        "0",
        "-o",
        s(&dir),
        "--label",
        "idx",
    ]));
    let q = accrete(&["index", "query", "-d", s(&dir), "--label", "idx"]);
    ok(&q);
    let csv = String::from_utf8_lossy(&q.stdout);
    assert!(csv.contains("\n4,idx,"), "{csv}");
    assert!(csv.contains("\n5,idx,"), "{csv}");
    let sql = accrete(&[
        "index",
        "sql",
        "-d",
        s(&dir),
        "SELECT COUNT(*) AS n FROM runs WHERE label = 'idx'",
    ]);
    ok(&sql);
    assert!(
        String::from_utf8_lossy(&sql.stdout).contains('2'),
        "{:?}",
        sql
    );
    // Rebuild after deleting a manifest drops the stale row.
    std::fs::remove_file(dir.join("idx_seed5_manifest.json")).unwrap();
    ok(&accrete(&["index", "rebuild", "-d", s(&dir)]));
    let sql = accrete(&[
        "index",
        "sql",
        "-d",
        s(&dir),
        "SELECT COUNT(*) AS n FROM runs",
    ]);
    ok(&sql);
    assert!(
        String::from_utf8_lossy(&sql.stdout).contains('1'),
        "{:?}",
        sql
    );
    let bad = accrete(&["index", "sql", "-d", s(&dir), "DELETE FROM runs"]);
    assert_eq!(bad.status.code(), Some(2));
    let lim = accrete(&["index", "query", "-d", s(&dir), "--limit", "0"]);
    assert_eq!(lim.status.code(), Some(2));
    let _ = std::fs::remove_dir_all(&dir);
}
