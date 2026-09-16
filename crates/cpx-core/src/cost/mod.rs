//! What the local transcripts say each account has spent.
//!
//! Both tools log every turn with the token counts the API billed, so a
//! spend estimate needs no network and no token: it is a sum over files
//! the tool already wrote. Subscription usage is not billed per token, so
//! for most people this is the *value* of what they used rather than a
//! bill — which is exactly what makes it worth watching.
//!
//! Transcripts run to gigabytes. Each file's rows are cached by size and
//! modification time under the cpx root, so a refresh re-reads only what
//! changed.

pub mod claude;
pub mod codex;
pub mod pricing;

use crate::config::Provider;
use pricing::Tokens;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One (day, model) sum from one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    /// `YYYY-MM-DD` in local time.
    pub day: String,
    pub model: String,
    pub tokens: Tokens,
}

/// A day's spend across models.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DaySpend {
    pub day: String,
    pub tokens: Tokens,
    /// The priced part. `None` only when nothing that day could be priced.
    pub cost_usd: Option<f64>,
    /// True when some of the day's tokens were on a model with no price.
    pub partly_unpriced: bool,
}

/// Tokens and money over a period.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Spend {
    pub tokens: Tokens,
    pub cost_usd: f64,
    pub partly_unpriced: bool,
}

/// Spend on one model over the whole report.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSpend {
    pub model: String,
    pub tokens: Tokens,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CostReport {
    pub provider: Provider,
    /// Oldest first, only days with activity.
    pub days: Vec<DaySpend>,
    pub today: Spend,
    pub last_7_days: Spend,
    pub last_30_days: Spend,
    pub models: Vec<ModelSpend>,
    /// Models that appeared but have no price in the table.
    pub unpriced_models: Vec<String>,
    pub files_scanned: usize,
    pub files_from_cache: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CachedFile {
    size: u64,
    mtime_ns: i128,
    rows: Vec<Row>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Cache {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    files: BTreeMap<PathBuf, CachedFile>,
}

const CACHE_VERSION: u32 = 1;

/// Where the cache for one account's transcripts lives: under the cpx
/// root, never inside the account's own directory.
pub fn cache_path(root: &Path, dir: &Path) -> PathBuf {
    let digest = crate::state::sha256_bytes(dir.as_os_str().as_encoded_bytes());
    root.join("cache").join("cost").join(format!("{}.json", &digest[..16]))
}

/// The local calendar day of an RFC 3339 timestamp.
pub(crate) fn day_of(timestamp: &str) -> Option<String> {
    let parsed = chrono::DateTime::parse_from_rfc3339(timestamp.trim()).ok()?;
    Some(parsed.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string())
}

fn today() -> chrono::NaiveDate {
    chrono::Local::now().date_naive()
}

/// Scan an account's transcripts, using and refreshing `cache` at
/// `cache_file`, and report the last `days` days.
pub fn scan(provider: Provider, dir: &Path, cache_file: Option<&Path>, days: u32) -> CostReport {
    let files = match provider {
        Provider::Claude => claude::transcript_files(dir),
        Provider::Codex => codex::rollout_files(dir),
    };

    let mut cache: Cache = cache_file
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .filter(|c: &Cache| c.version == CACHE_VERSION)
        .unwrap_or_default();
    cache.version = CACHE_VERSION;

    let mut fresh: BTreeMap<PathBuf, CachedFile> = BTreeMap::new();
    let mut from_cache = 0;
    for path in &files {
        let Ok(meta) = std::fs::metadata(path) else {
            continue;
        };
        let size = meta.len();
        let mtime_ns = meta
            .modified()
            .ok()
            .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as i128)
            .unwrap_or(0);
        if let Some(cached) = cache.files.get(path) {
            if cached.size == size && cached.mtime_ns == mtime_ns {
                from_cache += 1;
                fresh.insert(path.clone(), cached.clone());
                continue;
            }
        }
        let rows = match provider {
            Provider::Claude => claude::scan_file(path),
            Provider::Codex => codex::scan_file(path),
        }
        .unwrap_or_default();
        fresh.insert(path.clone(), CachedFile { size, mtime_ns, rows });
    }
    cache.files = fresh;

    if let Some(path) = cache_file {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string(&cache) {
            let _ = std::fs::write(path, text);
        }
    }

    report(provider, cache.files.values().flat_map(|f| f.rows.iter()), days, files.len(), from_cache, today())
}

/// Build the report from rows, relative to `today`.
pub fn report<'a>(
    provider: Provider,
    rows: impl Iterator<Item = &'a Row>,
    days: u32,
    files_scanned: usize,
    files_from_cache: usize,
    today: chrono::NaiveDate,
) -> CostReport {
    let since = today - chrono::Duration::days(days.saturating_sub(1) as i64);
    let since_key = since.format("%Y-%m-%d").to_string();
    let today_key = today.format("%Y-%m-%d").to_string();
    let week_key = (today - chrono::Duration::days(6)).format("%Y-%m-%d").to_string();
    let month_key = (today - chrono::Duration::days(29)).format("%Y-%m-%d").to_string();

    let mut by_day: BTreeMap<String, DaySpend> = BTreeMap::new();
    let mut by_model: BTreeMap<String, (Tokens, Option<f64>)> = BTreeMap::new();
    let mut unpriced: Vec<String> = Vec::new();

    for row in rows {
        if row.day < since_key {
            continue;
        }
        let cost = pricing::cost_usd(provider, &row.model, &row.tokens);
        if cost.is_none() && !unpriced.contains(&row.model) {
            unpriced.push(row.model.clone());
        }

        let day = by_day.entry(row.day.clone()).or_insert_with(|| DaySpend {
            day: row.day.clone(),
            tokens: Tokens::default(),
            cost_usd: None,
            partly_unpriced: false,
        });
        day.tokens.add(&row.tokens);
        match cost {
            Some(cost) => day.cost_usd = Some(day.cost_usd.unwrap_or(0.0) + cost),
            None => day.partly_unpriced = true,
        }

        let model = by_model.entry(row.model.clone()).or_insert((Tokens::default(), cost.map(|_| 0.0)));
        model.0.add(&row.tokens);
        if let (Some(total), Some(cost)) = (&mut model.1, cost) {
            *total += cost;
        }
    }

    let sum = |from: &str| -> Spend {
        let mut spend = Spend::default();
        for day in by_day.values().filter(|d| d.day.as_str() >= from) {
            spend.tokens.add(&day.tokens);
            spend.cost_usd += day.cost_usd.unwrap_or(0.0);
            spend.partly_unpriced |= day.partly_unpriced;
        }
        spend
    };

    let mut models: Vec<ModelSpend> = by_model
        .into_iter()
        .map(|(model, (tokens, cost_usd))| ModelSpend { model, tokens, cost_usd })
        .collect();
    models.sort_by(|a, b| {
        b.cost_usd
            .unwrap_or(0.0)
            .partial_cmp(&a.cost_usd.unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.tokens.total().cmp(&a.tokens.total()))
    });
    unpriced.sort();

    CostReport {
        provider,
        today: sum(&today_key),
        last_7_days: sum(&week_key),
        last_30_days: sum(&month_key),
        days: by_day.into_values().collect(),
        models,
        unpriced_models: unpriced,
        files_scanned,
        files_from_cache,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(day: &str, model: &str, input: u64, output: u64) -> Row {
        Row {
            day: day.into(),
            model: model.into(),
            tokens: Tokens {
                input,
                output,
                ..Default::default()
            },
        }
    }

    fn date(text: &str) -> chrono::NaiveDate {
        chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn the_report_sums_by_day_and_period() {
        let rows = [
            row("2026-09-16", "claude-sonnet-4-6", 1_000_000, 0),
            row("2026-09-16", "claude-haiku-4-5", 0, 1_000_000),
            row("2026-09-12", "claude-sonnet-4-6", 1_000_000, 0),
            row("2026-08-01", "claude-sonnet-4-6", 1_000_000, 0),
            row("2026-06-01", "claude-sonnet-4-6", 1_000_000, 0),
        ];
        let report = report(Provider::Claude, rows.iter(), 30, 5, 2, date("2026-09-16"));
        assert_eq!(report.days.len(), 2, "{:?}", report.days);
        assert!((report.today.cost_usd - 8.0).abs() < 1e-9, "{}", report.today.cost_usd);
        assert!((report.last_7_days.cost_usd - 11.0).abs() < 1e-9);
        assert!((report.last_30_days.cost_usd - 11.0).abs() < 1e-9);
        assert_eq!(report.models[0].model, "claude-sonnet-4-6");
        assert_eq!(report.models[1].model, "claude-haiku-4-5");
        assert_eq!(report.files_scanned, 5);
        assert_eq!(report.files_from_cache, 2);
        assert!(report.unpriced_models.is_empty());
    }

    #[test]
    fn unknown_models_are_listed_and_leave_the_day_partly_unpriced() {
        let rows = [row("2026-09-16", "claude-sonnet-4-6", 1_000_000, 0), row("2026-09-16", "claude-9", 5, 5)];
        let report = report(Provider::Claude, rows.iter(), 7, 1, 0, date("2026-09-16"));
        assert_eq!(report.unpriced_models, vec!["claude-9"]);
        assert!(report.today.partly_unpriced);
        assert!((report.today.cost_usd - 3.0).abs() < 1e-9);
        assert_eq!(report.days[0].tokens.input, 1_000_005);
        assert_eq!(report.models.iter().find(|m| m.model == "claude-9").unwrap().cost_usd, None);
    }

    #[test]
    fn scanning_uses_the_cache_for_unchanged_files() {
        let d = tempfile::TempDir::new().unwrap();
        let projects = d.path().join("home/projects/p");
        std::fs::create_dir_all(&projects).unwrap();
        std::fs::write(
            projects.join("a.jsonl"),
            r#"{"type":"assistant","timestamp":"2026-09-16T08:00:00Z","message":{"id":"m1","model":"claude-opus-4-7","usage":{"input_tokens":10,"output_tokens":5}}}
"#,
        )
        .unwrap();
        let cache = d.path().join("root/cache/cost/x.json");

        let first = scan(Provider::Claude, &d.path().join("home"), Some(&cache), 3650);
        assert_eq!(first.files_scanned, 1);
        assert_eq!(first.files_from_cache, 0);
        assert_eq!(first.days[0].tokens.input, 10);
        assert!(cache.is_file());

        let second = scan(Provider::Claude, &d.path().join("home"), Some(&cache), 3650);
        assert_eq!(second.files_from_cache, 1);
        assert_eq!(second.days, first.days);

        // Appending re-reads the file.
        let mut text = std::fs::read_to_string(projects.join("a.jsonl")).unwrap();
        text.push_str(r#"{"type":"assistant","timestamp":"2026-09-16T09:00:00Z","message":{"id":"m2","model":"claude-opus-4-7","usage":{"input_tokens":10,"output_tokens":5}}}
"#);
        std::fs::write(projects.join("a.jsonl"), text).unwrap();
        let third = scan(Provider::Claude, &d.path().join("home"), Some(&cache), 3650);
        assert_eq!(third.files_from_cache, 0);
        assert_eq!(third.days[0].tokens.input, 20);
    }

    #[test]
    fn the_cache_lives_under_the_root_keyed_by_directory() {
        let a = cache_path(Path::new("/r"), Path::new("/Users/x/.claude-work"));
        let b = cache_path(Path::new("/r"), Path::new("/Users/x/.claude-home"));
        assert!(a.starts_with("/r/cache/cost"));
        assert_ne!(a, b);
    }

    #[test]
    fn day_keys_are_local_calendar_days() {
        assert!(day_of("2026-09-16T08:00:00Z").is_some());
        assert_eq!(day_of("nope"), None);
    }
}
