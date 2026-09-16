//! Reading Codex session rollouts for what each turn cost.
//!
//! Codex writes `<home>/sessions/YYYY/MM/DD/rollout-*.jsonl`, one JSON line
//! per event. Every `token_count` event carries the session's running total
//! and the last turn's usage. The totals are cumulative, so a turn's cost is
//! the difference from the previous total — which is robust to a
//! `last_token_usage` that repeats — with the last turn's numbers as the
//! fallback when the totals go backwards.

use super::pricing::Tokens;
use super::Row;
use crate::config::Provider;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// Every rollout under a Codex home, live and archived.
pub fn rollout_files(home_dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for root in ["sessions", "archived_sessions"] {
        walk(&home_dir.join(root), &mut found);
    }
    found.sort();
    found
}

fn walk(dir: &Path, into: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.file_type() else {
            continue;
        };
        if meta.is_dir() {
            walk(&path, into);
        } else if path.extension().is_some_and(|e| e == "jsonl") {
            into.push(path);
        }
    }
}

fn totals(value: Option<&serde_json::Value>) -> Option<Tokens> {
    let value = value?;
    let int = |key: &str| value.get(key).and_then(|v| v.as_u64()).unwrap_or(0);
    Some(Tokens {
        input: int("input_tokens"),
        output: int("output_tokens"),
        cache_read: int("cached_input_tokens").max(int("cache_read_input_tokens")),
        cache_write: int("cache_write_input_tokens"),
        cache_write_1h: 0,
    })
}

fn delta(current: &Tokens, previous: &Tokens) -> Option<Tokens> {
    if current.input < previous.input
        || current.output < previous.output
        || current.cache_read < previous.cache_read
        || current.cache_write < previous.cache_write
    {
        return None;
    }
    Some(Tokens {
        input: current.input - previous.input,
        output: current.output - previous.output,
        cache_read: current.cache_read - previous.cache_read,
        cache_write: current.cache_write - previous.cache_write,
        cache_write_1h: 0,
    })
}

/// The rows in one rollout, summed by day and model.
pub fn scan_file(path: &Path) -> std::io::Result<Vec<Row>> {
    let file = std::fs::File::open(path)?;
    let reader = BufReader::new(file);

    let mut rows: BTreeMap<(String, String), Tokens> = BTreeMap::new();
    let mut model = String::from("unknown");
    let mut previous_total: Option<Tokens> = None;

    for line in reader.split(b'\n') {
        let line = line?;
        let interesting = contains(&line, b"\"turn_context\"") || contains(&line, b"\"token_count\"");
        if !interesting {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&line) else {
            continue;
        };
        let kind = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let payload = value.get("payload");

        if kind == "turn_context" {
            if let Some(name) = payload
                .and_then(|p| p.get("model").or_else(|| p.get("model_name")))
                .and_then(|v| v.as_str())
            {
                model = super::pricing::normalize(Provider::Codex, name);
            }
            continue;
        }
        if kind != "event_msg" || payload.and_then(|p| p.get("type")).and_then(|v| v.as_str()) != Some("token_count") {
            continue;
        }
        let Some(info) = payload.and_then(|p| p.get("info")).filter(|i| i.is_object()) else {
            continue;
        };
        if let Some(name) = info.get("model").or_else(|| info.get("model_name")).and_then(|v| v.as_str()) {
            model = super::pricing::normalize(Provider::Codex, name);
        }
        let Some(day) = value.get("timestamp").and_then(|v| v.as_str()).and_then(super::day_of) else {
            continue;
        };

        let total = totals(info.get("total_token_usage"));
        let last = totals(info.get("last_token_usage"));

        let turn = match (total, previous_total) {
            (Some(total), Some(previous)) => match delta(&total, &previous) {
                Some(d) => Some(d),
                // The total went backwards: a new context or a reset. Trust
                // the last turn's own numbers instead.
                None => last,
            },
            (Some(total), None) => Some(total),
            (None, _) => last,
        };
        if let Some(total) = total {
            previous_total = Some(total);
        }
        let Some(turn) = turn else {
            continue;
        };
        if turn.total() == 0 {
            continue;
        }
        rows.entry((day, model.clone())).or_default().add(&turn);
    }

    Ok(rows
        .into_iter()
        .map(|((day, model), tokens)| Row { day, model, tokens })
        .collect())
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(ts: &str, input: u64, cached: u64, output: u64, last_input: u64, last_output: u64) -> String {
        format!(
            r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{input},"cached_input_tokens":{cached},"cache_write_input_tokens":0,"output_tokens":{output},"reasoning_output_tokens":0,"total_tokens":0}},"last_token_usage":{{"input_tokens":{last_input},"cached_input_tokens":0,"output_tokens":{last_output}}},"model_context_window":258400}}}}}}"#
        )
    }

    fn write(dir: &Path, lines: &[String]) -> PathBuf {
        let path = dir.join("rollout-2026-09-16T10-00-00-abc.jsonl");
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
        path
    }

    #[test]
    fn turns_are_the_deltas_of_the_running_total() {
        let d = tempfile::TempDir::new().unwrap();
        let path = write(
            d.path(),
            &[
                r#"{"timestamp":"2026-09-16T10:00:00Z","type":"session_meta","payload":{"id":"s"}}"#.to_string(),
                r#"{"timestamp":"2026-09-16T10:00:00Z","type":"turn_context","payload":{"cwd":"/x","model":"gpt-5.4"}}"#.to_string(),
                r#"{"timestamp":"2026-09-16T10:00:01Z","type":"event_msg","payload":{"type":"token_count","info":null}}"#.to_string(),
                count("2026-09-16T10:00:02Z", 1000, 500, 100, 1000, 100),
                // A repeated event with the same totals adds nothing.
                count("2026-09-16T10:00:03Z", 1000, 500, 100, 1000, 100),
                count("2026-09-16T10:00:04Z", 1600, 900, 130, 600, 30),
            ],
        );
        let rows = scan_file(&path).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].model, "gpt-5.4");
        assert_eq!(rows[0].day, "2026-09-16");
        assert_eq!(rows[0].tokens.input, 1600);
        assert_eq!(rows[0].tokens.cache_read, 900);
        assert_eq!(rows[0].tokens.output, 130);
    }

    #[test]
    fn a_total_that_goes_backwards_falls_back_to_the_last_turn() {
        let d = tempfile::TempDir::new().unwrap();
        let path = write(
            d.path(),
            &[
                r#"{"timestamp":"2026-09-16T10:00:00Z","type":"turn_context","payload":{"model":"gpt-5.4"}}"#.to_string(),
                count("2026-09-16T10:00:02Z", 1000, 0, 100, 1000, 100),
                count("2026-09-16T10:00:04Z", 300, 0, 10, 300, 10),
            ],
        );
        let rows = scan_file(&path).unwrap();
        assert_eq!(rows[0].tokens.input, 1300);
        assert_eq!(rows[0].tokens.output, 110);
    }

    #[test]
    fn the_model_follows_the_latest_turn_context() {
        let d = tempfile::TempDir::new().unwrap();
        let path = write(
            d.path(),
            &[
                r#"{"timestamp":"2026-09-16T10:00:00Z","type":"turn_context","payload":{"model":"gpt-5.4"}}"#.to_string(),
                count("2026-09-16T10:00:02Z", 100, 0, 10, 100, 10),
                r#"{"timestamp":"2026-09-16T10:00:03Z","type":"turn_context","payload":{"model":"gpt-5.5"}}"#.to_string(),
                count("2026-09-17T10:00:04Z", 300, 0, 40, 200, 30),
            ],
        );
        let rows = scan_file(&path).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].model.as_str(), rows[0].tokens.input), ("gpt-5.4", 100));
        assert_eq!((rows[1].model.as_str(), rows[1].tokens.input), ("gpt-5.5", 200));
    }

    #[test]
    fn rollouts_are_found_in_dated_and_archived_directories() {
        let d = tempfile::TempDir::new().unwrap();
        let dated = d.path().join("sessions/2026/09/16");
        let archived = d.path().join("archived_sessions");
        std::fs::create_dir_all(&dated).unwrap();
        std::fs::create_dir_all(&archived).unwrap();
        std::fs::write(dated.join("rollout-a.jsonl"), "").unwrap();
        std::fs::write(archived.join("rollout-b.jsonl"), "").unwrap();
        std::fs::write(d.path().join("sessions/index.json"), "").unwrap();
        assert_eq!(rollout_files(d.path()).len(), 2);
    }
}
