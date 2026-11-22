//! SQLite run index (accrete.md A.10 "run storage").
//!
//! Every finished run already writes a `*_manifest.json`. This module keeps a
//! queryable index of those manifests so Phase 2 campaigns can be listed and
//! filtered without grepping directories.
//!
//! - **`accrete index rebuild -d DIR`** walks `DIR` (recursively) for manifests
//!   and upserts them into `DIR/runs.sqlite` (or `--db`).
//! - **`accrete index query`** prints matching rows as CSV.
//! - **`accrete index sql`** runs a read-only SQL statement.
//! - Writing a manifest also upserts into the output folder's `runs.sqlite`,
//!   and refreshes a deterministic `runs_index.csv` beside it.
//!
//! In-process writers (`--jobs` threads) share a mutex so upserts cannot race.
//! SQLite's on-disk pages are still not byte-stable across runs; compare
//! `runs_index.csv` (or `index query`) when checking identity.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use accrete_core::config::AgentKind;
use accrete_core::hash;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params, params_from_iter};

use crate::manifest::{Manifest, Start};

pub const DB_NAME: &str = "runs.sqlite";
pub const INDEX_CSV: &str = "runs_index.csv";
pub const SCHEMA_VERSION: i64 = 1;
/// How deep `index rebuild` descends below `-d` for `*_manifest.json`.
pub const MAX_MANIFEST_DEPTH: u32 = 32;

/// Serialize all in-process index writers (`--jobs` shares one process).
static INDEX_LOCK: Mutex<()> = Mutex::new(());

/// True for the SQLite DB and its WAL/SHM sidecars. Binary pages are not
/// compared across `--jobs`; use `runs_index.csv` for a stable dump.
#[cfg(test)]
pub fn is_index_artifact(name: &str) -> bool {
    name == DB_NAME || name.starts_with("runs.sqlite-")
}

fn with_index_lock<R>(f: impl FnOnce() -> R) -> R {
    let _guard = INDEX_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f()
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS runs (
    id INTEGER PRIMARY KEY NOT NULL,
    manifest_path TEXT NOT NULL UNIQUE,
    label TEXT NOT NULL,
    seed INTEGER NOT NULL,
    start_kind TEXT NOT NULL,
    start_tick INTEGER NOT NULL,
    ticks INTEGER NOT NULL,
    final_tick INTEGER NOT NULL,
    final_state_hash TEXT NOT NULL,
    config_digest TEXT NOT NULL,
    food_regrowth REAL NOT NULL,
    agent_kind TEXT NOT NULL,
    trade_enabled INTEGER NOT NULL,
    trails_enabled INTEGER NOT NULL,
    git_commit TEXT NOT NULL,
    git_dirty INTEGER NOT NULL,
    accrete_version TEXT NOT NULL,
    cargo_lock_hash TEXT NOT NULL,
    population INTEGER,
    total_births INTEGER,
    total_deaths_deprivation INTEGER,
    hamlets INTEGER,
    trail_tiles INTEGER,
    indexed_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS runs_seed ON runs(seed);
CREATE INDEX IF NOT EXISTS runs_label ON runs(label);
CREATE INDEX IF NOT EXISTS runs_commit ON runs(git_commit);
CREATE INDEX IF NOT EXISTS runs_digest ON runs(config_digest);
";

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(2)
}

/// Default DB path next to the manifests: `dir/runs.sqlite`.
pub fn default_db(dir: &Path) -> PathBuf {
    dir.join(DB_NAME)
}

fn configure_writer(conn: &Connection) -> Result<(), String> {
    // Parallel --jobs writers share one DB; wait instead of failing immediately.
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|e| format!("busy_timeout: {e}"))?;
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;",
    )
    .map_err(|e| format!("pragma: {e}"))?;
    Ok(())
}

fn ensure_schema(conn: &Connection, db: &Path) -> Result<(), String> {
    conn.execute_batch(SCHEMA)
        .map_err(|e| format!("creating schema: {e}"))?;
    let ver: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    match ver.as_deref() {
        None => {
            // INSERT OR IGNORE: two parallel first-opens must not race on meta.
            conn.execute(
                "INSERT OR IGNORE INTO meta(key, value) VALUES ('schema_version', ?1)",
                params![SCHEMA_VERSION.to_string()],
            )
            .map_err(|e| e.to_string())?;
            let ver: String = conn
                .query_row(
                    "SELECT value FROM meta WHERE key = 'schema_version'",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if ver != SCHEMA_VERSION.to_string() {
                return Err(format!(
                    "{} has schema version {ver}, this build expects {SCHEMA_VERSION}",
                    db.display()
                ));
            }
        }
        Some(v) if v == SCHEMA_VERSION.to_string() => {}
        Some(v) => {
            return Err(format!(
                "{} has schema version {v}, this build expects {SCHEMA_VERSION}",
                db.display()
            ));
        }
    }
    Ok(())
}

fn open_rw(db: &Path) -> Result<Connection, String> {
    if let Some(parent) = db.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let conn = Connection::open(db).map_err(|e| format!("opening {}: {e}", db.display()))?;
    configure_writer(&conn)?;
    ensure_schema(&conn, db)?;
    Ok(conn)
}

fn open_ro(db: &Path) -> Result<Connection, String> {
    if !db.is_file() {
        return Err(format!(
            "no run index at {}; run `accrete index rebuild` first",
            db.display()
        ));
    }
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("opening {}: {e}", db.display()))?;
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|e| format!("busy_timeout: {e}"))?;
    let ver: String = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |r| r.get(0),
        )
        .map_err(|e| format!("{}: {e} (is this a runs.sqlite?)", db.display()))?;
    if ver != SCHEMA_VERSION.to_string() {
        return Err(format!(
            "{} has schema version {ver}, this build expects {SCHEMA_VERSION}",
            db.display()
        ));
    }
    Ok(conn)
}

fn path_is_absolute(s: &str) -> bool {
    Path::new(s).is_absolute()
        || (s.len() >= 3
            && s.as_bytes()[0].is_ascii_alphabetic()
            && s.as_bytes()[1] == b':'
            && (s.as_bytes()[2] == b'/' || s.as_bytes()[2] == b'\\'))
}

/// Resolve a path stored in the DB (relative to the DB directory, or absolute).
fn resolve_stored_path(db: &Path, stored: &str) -> PathBuf {
    if path_is_absolute(stored) {
        return PathBuf::from(stored);
    }
    let base = db.parent().unwrap_or_else(|| Path::new("."));
    let mut p = base.to_path_buf();
    for part in stored.split(['/', '\\']) {
        if !part.is_empty() {
            p.push(part);
        }
    }
    p
}

/// Path stored in the DB: prefer path relative to the DB's directory, with `/`.
fn store_path(db: &Path, manifest: &Path) -> String {
    let base = db.parent().unwrap_or_else(|| Path::new("."));
    crate::manifest::relative_to(base, manifest)
        .unwrap_or_else(|| manifest.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/")
}

fn start_kind(s: &Start) -> &'static str {
    match s {
        Start::Fresh { .. } => "fresh",
        Start::Snapshot { .. } => "snapshot",
    }
}

fn agent_kind_str(c: &accrete_core::Config) -> &'static str {
    match c.sim.agent_kind {
        AgentKind::Normal => "normal",
        AgentKind::Random => "random",
        AgentKind::Omniscient => "omniscient",
    }
}

struct MetricTotals {
    population: Option<i64>,
    total_births: Option<i64>,
    total_deaths_deprivation: Option<i64>,
    hamlets: Option<i64>,
    trail_tiles: Option<i64>,
}

fn metrics_totals(manifest_dir: &Path, m: &Manifest) -> MetricTotals {
    let empty = MetricTotals {
        population: None,
        total_births: None,
        total_deaths_deprivation: None,
        hamlets: None,
        trail_tiles: None,
    };
    let name = format!("{}_seed{}_metrics.csv", m.label, m.seed);
    let path = manifest_dir.join(&name);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return empty;
    };
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let Some(header) = lines.next() else {
        return empty;
    };
    let cols: Vec<&str> = header.split(',').collect();
    let idx = |name: &str| cols.iter().position(|c| *c == name);
    let (Some(i_pop), Some(i_births), Some(i_dep), Some(i_ham), Some(i_trail)) = (
        idx("population"),
        idx("total_births"),
        idx("total_deaths_deprivation"),
        idx("hamlets"),
        idx("trail_tiles"),
    ) else {
        return empty;
    };
    let Some(last) = lines.next_back() else {
        return empty;
    };
    let cells: Vec<&str> = last.split(',').collect();
    let get = |i: usize| {
        cells
            .get(i)
            .and_then(|s| s.parse::<f64>().ok())
            .map(|x| x as i64)
    };
    MetricTotals {
        population: get(i_pop),
        total_births: get(i_births),
        total_deaths_deprivation: get(i_dep),
        hamlets: get(i_ham),
        trail_tiles: get(i_trail),
    }
}

/// Upsert one manifest into `db`. `manifest_path` is the file on disk.
pub fn upsert(db: &Path, manifest_path: &Path) -> Result<(), String> {
    with_index_lock(|| {
        upsert_inner(db, manifest_path)?;
        write_index_csv_beside(db)
    })
}

fn upsert_inner(db: &Path, manifest_path: &Path) -> Result<(), String> {
    let m = Manifest::read(manifest_path)?;
    let conn = open_rw(db)?;
    let stored = store_path(db, manifest_path);
    let dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let totals = metrics_totals(dir, &m);
    let digest = hash::hex(m.config.digest());
    // Deterministic: same as final_state_hash (no wall-clock).
    let indexed_at = m.final_state_hash.clone();
    conn.execute(
        "INSERT INTO runs (
            manifest_path, label, seed, start_kind, start_tick, ticks, final_tick,
            final_state_hash, config_digest, food_regrowth, agent_kind, trade_enabled,
            trails_enabled, git_commit, git_dirty, accrete_version, cargo_lock_hash,
            population, total_births, total_deaths_deprivation, hamlets, trail_tiles, indexed_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23)
         ON CONFLICT(manifest_path) DO UPDATE SET
            label=excluded.label, seed=excluded.seed, start_kind=excluded.start_kind,
            start_tick=excluded.start_tick, ticks=excluded.ticks, final_tick=excluded.final_tick,
            final_state_hash=excluded.final_state_hash, config_digest=excluded.config_digest,
            food_regrowth=excluded.food_regrowth, agent_kind=excluded.agent_kind,
            trade_enabled=excluded.trade_enabled, trails_enabled=excluded.trails_enabled,
            git_commit=excluded.git_commit, git_dirty=excluded.git_dirty,
            accrete_version=excluded.accrete_version, cargo_lock_hash=excluded.cargo_lock_hash,
            population=excluded.population, total_births=excluded.total_births,
            total_deaths_deprivation=excluded.total_deaths_deprivation,
            hamlets=excluded.hamlets, trail_tiles=excluded.trail_tiles,
            indexed_at=excluded.indexed_at",
        params![
            stored,
            m.label,
            m.seed as i64,
            start_kind(&m.start),
            m.start_tick as i64,
            m.ticks as i64,
            m.final_tick as i64,
            m.final_state_hash,
            digest,
            f64::from(m.config.resources.food_regrowth),
            agent_kind_str(&m.config),
            m.config.social.trade_enabled as i64,
            m.config.trails.enabled as i64,
            m.build.git_commit,
            m.build.git_dirty as i64,
            m.build.accrete_version,
            m.build.cargo_lock_hash,
            totals.population,
            totals.total_births,
            totals.total_deaths_deprivation,
            totals.hamlets,
            totals.trail_tiles,
            indexed_at,
        ],
    )
    .map_err(|e| format!("upserting {}: {e}", manifest_path.display()))?;
    Ok(())
}

/// Index a manifest into `out_dir/runs.sqlite`. Failures are reported but do
/// not fail the run (the manifest file is the source of truth).
pub fn upsert_after_run(out_dir: &Path, manifest_path: &Path) {
    let db = default_db(out_dir);
    if let Err(e) = upsert(&db, manifest_path) {
        eprintln!("warning: could not update run index {}: {e}", db.display());
    }
}

/// Deterministic CSV dump next to the DB (`ORDER BY seed, label, manifest_path`).
fn write_index_csv_beside(db: &Path) -> Result<(), String> {
    if !db.is_file() {
        return Ok(());
    }
    let dir = db.parent().unwrap_or_else(|| Path::new("."));
    let csv = query_unlocked(&QueryArgs {
        db: db.to_path_buf(),
        seed: None,
        label: None,
        commit: None,
        digest: None,
        limit: 1_000_000,
    })?;
    std::fs::write(dir.join(INDEX_CSV), csv)
        .map_err(|e| format!("writing {}/{}: {e}", dir.display(), INDEX_CSV))?;
    Ok(())
}

fn remove_db_files(db: &Path) {
    let _ = std::fs::remove_file(db);
    let _ = std::fs::remove_file(PathBuf::from(format!("{}-wal", db.display())));
    let _ = std::fs::remove_file(PathBuf::from(format!("{}-shm", db.display())));
}

/// Rebuild the index from every manifest under `dir` (fresh DB + CSV). Used at
/// the end of multi-run commands so `--jobs` leaves a complete, ordered index.
pub fn finalize_dir(dir: &Path) -> Result<(), String> {
    let db = default_db(dir);
    with_index_lock(|| {
        remove_db_files(&db);
        let (ok, err) = rebuild_inner(dir, &db)?;
        write_index_csv_beside(&db)?;
        if err > 0 {
            return Err(format!(
                "indexed {ok} manifest(s) into {} ({err} failed)",
                db.display()
            ));
        }
        Ok(())
    })
}

/// Find `*_manifest.json` under `dir`, recursively up to [`MAX_MANIFEST_DEPTH`].
pub fn find_manifests(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    find_manifests_rec(dir, 0, &mut out)?;
    out.sort();
    out.dedup();
    Ok(out)
}

fn find_manifests_rec(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("reading {}: {e}", dir.display()))?;
    for e in entries {
        let e = e.map_err(|e| e.to_string())?;
        let p = e.path();
        if p.is_file() {
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if name.ends_with("_manifest.json") {
                out.push(p);
            }
        } else if p.is_dir() {
            if depth >= MAX_MANIFEST_DEPTH {
                eprintln!(
                    "warning: skipping {}: deeper than {MAX_MANIFEST_DEPTH} levels below the index root",
                    p.display()
                );
            } else {
                find_manifests_rec(&p, depth + 1, out)?;
            }
        }
    }
    Ok(())
}

pub fn rebuild(dir: &Path, db: &Path) -> Result<(usize, usize), String> {
    with_index_lock(|| {
        let counts = rebuild_inner(dir, db)?;
        write_index_csv_beside(db)?;
        Ok(counts)
    })
}

fn rebuild_inner(dir: &Path, db: &Path) -> Result<(usize, usize), String> {
    let manifests = find_manifests(dir)?;
    let mut ok = 0usize;
    let mut err = 0usize;
    for m in &manifests {
        match upsert_inner(db, m) {
            Ok(()) => ok += 1,
            Err(e) => {
                eprintln!("warning: {}: {e}", m.display());
                err += 1;
            }
        }
    }
    // Drop rows whose manifest file is gone.
    let conn = open_rw(db)?;
    let paths: Vec<String> = conn
        .prepare("SELECT manifest_path FROM runs")
        .map_err(|e| e.to_string())?
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    for stored in paths {
        let p = resolve_stored_path(db, &stored);
        if !p.is_file() {
            conn.execute("DELETE FROM runs WHERE manifest_path = ?1", params![stored])
                .map_err(|e| e.to_string())?;
        }
    }
    Ok((ok, err))
}

pub struct QueryArgs {
    pub db: PathBuf,
    pub seed: Option<u64>,
    pub label: Option<String>,
    pub commit: Option<String>,
    pub digest: Option<String>,
    pub limit: usize,
}

pub fn query(a: &QueryArgs) -> Result<String, String> {
    query_unlocked(a)
}

fn query_unlocked(a: &QueryArgs) -> Result<String, String> {
    let conn = open_ro(&a.db)?;
    let mut sql = String::from(
        "SELECT seed, label, ticks, final_tick, population, food_regrowth, agent_kind, \
         final_state_hash, config_digest, git_commit, manifest_path FROM runs WHERE 1=1",
    );
    let mut vals: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    if a.limit == 0 {
        return Err("--limit must be at least 1".into());
    }
    let limit = i64::try_from(a.limit).unwrap_or(i64::MAX).max(1);
    if let Some(s) = a.seed {
        sql.push_str(" AND seed = ?");
        vals.push(Box::new(s as i64));
    }
    if let Some(ref l) = a.label {
        sql.push_str(" AND label = ?");
        vals.push(Box::new(l.clone()));
    }
    if let Some(ref c) = a.commit {
        sql.push_str(" AND git_commit LIKE ?");
        vals.push(Box::new(format!("{c}%")));
    }
    if let Some(ref d) = a.digest {
        sql.push_str(" AND config_digest = ?");
        vals.push(Box::new(d.clone()));
    }
    sql.push_str(" ORDER BY seed, label LIMIT ?");
    vals.push(Box::new(limit));
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params_from_iter(vals.iter().map(|b| b.as_ref())), |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, f64>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, String>(9)?,
                r.get::<_, String>(10)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut out = String::from(
        "seed,label,ticks,final_tick,population,food_regrowth,agent_kind,final_state_hash,config_digest,git_commit,manifest_path\n",
    );
    for row in rows {
        let (seed, label, ticks, final_tick, pop, fr, ak, hash, dig, commit, path) =
            row.map_err(|e| e.to_string())?;
        let pop_s = pop.map(|p| p.to_string()).unwrap_or_default();
        // Format food_regrowth via f32 so CSV matches the config knob, not f64 noise.
        let fr_s = format!("{}", fr as f32);
        out.push_str(&format!(
            "{seed},{},{ticks},{final_tick},{pop_s},{fr_s},{},{},{},{},{}\n",
            csv_text(&label),
            csv_text(&ak),
            csv_text(&hash),
            csv_text(&dig),
            csv_text(&commit),
            csv_text(&path),
        ));
    }
    Ok(out)
}

fn looks_like_readonly_sql(sql: &str) -> bool {
    let trimmed = sql.trim();
    let lower = trimmed.to_ascii_lowercase();
    let head = lower.split_whitespace().next().unwrap_or("");
    match head {
        "select" => true,
        // WITH … must end in SELECT (not DELETE/INSERT/UPDATE). Strip strings
        // coarsely so a string cannot hide a write keyword at the end.
        "with" => {
            let no_strings = strip_sql_strings(&lower);
            let last = no_strings
                .split([';', ')'])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .next_back()
                .unwrap_or("");
            last.starts_with("select")
        }
        // Only safe, non-mutating pragmas.
        "pragma" => {
            let rest = lower.trim_start_matches("pragma").trim();
            matches!(
                rest.split(['=', '(']).next().unwrap_or("").trim(),
                "table_info"
                    | "schema_version"
                    | "user_version"
                    | "compile_options"
                    | "database_list"
                    | "foreign_key_list"
                    | "index_list"
                    | "index_info"
            ) && !rest.contains('=')
        }
        _ => false,
    }
}

fn strip_sql_strings(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\'' {
            out.push(' ');
            while let Some(c2) = chars.next() {
                if c2 == '\'' {
                    if chars.peek() == Some(&'\'') {
                        chars.next();
                        continue;
                    }
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub fn sql_readonly(db: &Path, sql: &str) -> Result<String, String> {
    let trimmed = sql.trim();
    if !looks_like_readonly_sql(trimmed) {
        return Err(
            "only read-only SQL is allowed (SELECT, WITH … SELECT, or a safe PRAGMA)".into(),
        );
    }
    let conn = open_ro(db)?;
    let mut stmt = conn.prepare(trimmed).map_err(|e| e.to_string())?;
    let col_count = stmt.column_count();
    let mut header = Vec::new();
    for i in 0..col_count {
        header.push(stmt.column_name(i).unwrap_or("?").to_string());
    }
    let mut out = header.join(",");
    out.push('\n');
    let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let mut cells = Vec::new();
        for i in 0..col_count {
            let v: rusqlite::types::Value = row.get(i).map_err(|e| e.to_string())?;
            cells.push(value_csv(&v));
        }
        out.push_str(&cells.join(","));
        out.push('\n');
    }
    Ok(out)
}

fn csv_text(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn value_csv(v: &rusqlite::types::Value) -> String {
    match v {
        rusqlite::types::Value::Null => String::new(),
        rusqlite::types::Value::Integer(i) => i.to_string(),
        rusqlite::types::Value::Real(f) => format!("{}", *f as f32),
        rusqlite::types::Value::Text(s) => csv_text(s),
        rusqlite::types::Value::Blob(_) => "<blob>".into(),
    }
}

pub fn run_rebuild(dir: PathBuf, db: Option<PathBuf>) {
    let db = db.unwrap_or_else(|| default_db(&dir));
    let (ok, err) = rebuild(&dir, &db).unwrap_or_else(|e| fail(e));
    println!(
        "indexed {ok} manifest(s) into {}{}",
        db.display(),
        if err > 0 {
            format!(" ({err} failed)")
        } else {
            String::new()
        }
    );
}

pub fn run_query(a: QueryArgs) {
    let csv = query(&a).unwrap_or_else(|e| fail(e));
    print!("{csv}");
}

pub fn run_sql(db: PathBuf, sql: String) {
    let out = sql_readonly(&db, &sql).unwrap_or_else(|e| fail(e));
    print!("{out}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Build, Manifest, Start};
    use accrete_core::Config;

    fn tmp() -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("accrete-idx-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn write_manifest(dir: &Path, label: &str, seed: u64, ticks: u64) -> PathBuf {
        let cfg = Config::default();
        let m = Manifest {
            format: crate::manifest::MANIFEST_FORMAT,
            build: Build {
                accrete_version: "0.1.0".into(),
                git_commit: "deadbeef".into(),
                git_dirty: false,
                cargo_lock_hash: "abc".into(),
            },
            command: vec!["accrete".into(), "run".into()],
            label: label.into(),
            seed,
            start: Start::Fresh { seed },
            start_tick: 0,
            start_state_hash: "0".into(),
            ticks,
            final_tick: ticks,
            final_state_hash: format!("hash{seed}"),
            config: cfg.clone(),
            outputs: vec![format!("{label}_seed{seed}_metrics.csv")],
        };
        // Minimal metrics so population columns fill in.
        let metrics = format!(
            "day,population,total_births,total_deaths_deprivation,hamlets,trail_tiles\n{},{},{},{},{},{}\n",
            ticks / 24,
            100 + seed,
            10,
            0,
            1,
            2
        );
        std::fs::write(dir.join(format!("{label}_seed{seed}_metrics.csv")), metrics).unwrap();
        let path = Manifest::path_for(dir, label, seed);
        let json = serde_json::to_string_pretty(&m).unwrap();
        std::fs::write(&path, json).unwrap();
        path
    }

    #[test]
    fn rebuild_indexes_and_query_filters() {
        let dir = tmp();
        let db = default_db(&dir);
        write_manifest(&dir, "control", 1, 48);
        write_manifest(&dir, "scarce", 2, 48);
        write_manifest(&dir, "control", 3, 24);
        let (ok, err) = rebuild(&dir, &db).unwrap();
        assert_eq!((ok, err), (3, 0));
        let csv = query(&QueryArgs {
            db: db.clone(),
            seed: Some(1),
            label: None,
            commit: None,
            digest: None,
            limit: 50,
        })
        .unwrap();
        assert!(csv.contains("control"), "{csv}");
        assert!(!csv.contains("scarce"), "{csv}");
        let csv = query(&QueryArgs {
            db: db.clone(),
            seed: None,
            label: Some("control".into()),
            commit: Some("dead".into()),
            digest: None,
            limit: 50,
        })
        .unwrap();
        assert_eq!(csv.lines().count(), 3, "{csv}"); // header + 2
        let sql = sql_readonly(&db, "SELECT COUNT(*) AS n FROM runs").unwrap();
        assert!(sql.contains("3"), "{sql}");
        assert!(sql_readonly(&db, "DELETE FROM runs").is_err());
        assert!(
            sql_readonly(&db, "WITH x AS (SELECT 1) DELETE FROM runs").is_err(),
            "WITH … DELETE must be refused"
        );
        assert!(sql_readonly(&db, "PRAGMA journal_mode=WAL").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rebuild_drops_missing_manifests() {
        let dir = tmp();
        let db = default_db(&dir);
        let p = write_manifest(&dir, "gone", 9, 24);
        rebuild(&dir, &db).unwrap();
        std::fs::remove_file(&p).unwrap();
        rebuild(&dir, &db).unwrap();
        let csv = query(&QueryArgs {
            db,
            seed: Some(9),
            label: None,
            commit: None,
            digest: None,
            limit: 10,
        })
        .unwrap();
        assert_eq!(csv.lines().count(), 1, "only header: {csv}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn upsert_is_idempotent_for_the_same_manifest() {
        let dir = tmp();
        let db = default_db(&dir);
        let p = write_manifest(&dir, "run", 1, 24);
        upsert(&db, &p).unwrap();
        upsert(&db, &p).unwrap();
        let n: i64 = open_ro(&db)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM runs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn absolute_stored_paths_are_not_joined_onto_the_db_dir() {
        let dir = tmp();
        let db = default_db(&dir);
        let p = write_manifest(&dir, "abs", 1, 24);
        let abs = p
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        assert!(path_is_absolute(&abs), "{abs}");
        assert!(
            resolve_stored_path(&db, &abs).is_file(),
            "absolute path must not be joined under the DB dir"
        );
        // Stale cleanup: absolute row whose file is gone must be dropped.
        open_rw(&db)
            .unwrap()
            .execute(
                "INSERT INTO runs (
                manifest_path, label, seed, start_kind, start_tick, ticks, final_tick,
                final_state_hash, config_digest, food_regrowth, agent_kind, trade_enabled,
                trails_enabled, git_commit, git_dirty, accrete_version, cargo_lock_hash,
                population, total_births, total_deaths_deprivation, hamlets, trail_tiles, indexed_at
             ) VALUES (?1,'abs',1,'fresh',0,24,24,'h','d',0.1,'normal',1,1,'c',0,'0.1.0','l',NULL,NULL,NULL,NULL,NULL,'0')",
                params![abs],
            )
            .unwrap();
        std::fs::remove_file(&p).unwrap();
        rebuild(&dir, &db).unwrap();
        let n: i64 = open_ro(&db)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM runs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "gone absolute path is dropped");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_manifests_walks_nested_directories() {
        let dir = tmp();
        let nested = dir.join("a").join("b").join("c");
        std::fs::create_dir_all(&nested).unwrap();
        write_manifest(&nested, "deep", 7, 24);
        write_manifest(&dir, "top", 1, 24);
        let found = find_manifests(&dir).unwrap();
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(
            found
                .iter()
                .any(|p| p.ends_with("deep_seed7_manifest.json")),
            "{found:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn query_and_sql_do_not_create_a_missing_db() {
        let dir = tmp();
        let db = default_db(&dir);
        assert!(
            query(&QueryArgs {
                db: db.clone(),
                seed: None,
                label: None,
                commit: None,
                digest: None,
                limit: 10,
            })
            .unwrap_err()
            .contains("no run index")
        );
        assert!(
            sql_readonly(&db, "SELECT 1")
                .unwrap_err()
                .contains("no run index")
        );
        assert!(!db.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
