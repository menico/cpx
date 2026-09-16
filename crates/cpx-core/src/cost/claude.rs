//! Reading Claude Code transcripts for what each turn cost.
//!
//! Claude Code appends one JSON line per event to
//! `<config>/projects/<project>/<session>.jsonl`. Assistant messages carry
//! the model and the token counts the API billed. A streamed response can
//! land as several records with the same message id; the last one carries
//! the final counts, so per file the last record for an id wins.

use super::pricing::Tokens;
use super::Row;
use crate::config::Provider;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// Every transcript under a config directory.
pub fn transcript_files(config_dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk(&config_dir.join("projects"), &mut found);
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

/// The rows in one transcript, one per billed message, summed by day and
/// model. Lines that are not assistant messages with usage are skipped
/// without being parsed.
pub fn scan_file(path: &Path) -> std::io::Result<Vec<Row>> {
    let file = std::fs::File::open(path)?;
    let reader = BufReader::new(file);

    // Keyed by message id (and request id when present) so a streamed
    // response counts once, with its final numbers.
    let mut keyed: BTreeMap<String, (String, String, Tokens)> = BTreeMap::new();
    let mut unkeyed: Vec<(String, String, Tokens)> = Vec::new();

    for line in reader.split(b'\n') {
        let line = line?;
        if !(contains(&line, b"\"type\":\"assistant\"") && contains(&line, b"\"usage\"")) {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&line) else {
            continue;
        };
        if value.get("type").and_then(|v| v.as_str()) != Some("assistant") {
            continue;
        }
        let Some(message) = value.get("message") else {
            continue;
        };
        let Some(model) = message.get("model").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(usage) = message.get("usage") else {
            continue;
        };
        let day = match value.get("timestamp").and_then(|v| v.as_str()).and_then(super::day_of) {
            Some(day) => day,
            None => continue,
        };

        let int = |key: &str| usage.get(key).and_then(|v| v.as_u64()).unwrap_or(0);
        let tokens = Tokens {
            input: int("input_tokens"),
            output: int("output_tokens"),
            cache_read: int("cache_read_input_tokens"),
            cache_write: int("cache_creation_input_tokens"),
            cache_write_1h: usage
                .pointer("/cache_creation/ephemeral_1h_input_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
        };
        if tokens.total() == 0 {
            continue;
        }

        let model = super::pricing::normalize(Provider::Claude, model);
        let message_id = message.get("id").and_then(|v| v.as_str());
        let request_id = value.get("requestId").and_then(|v| v.as_str());
        match message_id {
            Some(id) => {
                let key = format!("{id}\u{0}{}", request_id.unwrap_or(""));
                keyed.insert(key, (day, model, tokens));
            }
            None => unkeyed.push((day, model, tokens)),
        }
    }

    let mut rows: BTreeMap<(String, String), Tokens> = BTreeMap::new();
    for (day, model, tokens) in keyed.into_values().chain(unkeyed) {
        rows.entry((day, model)).or_default().add(&tokens);
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

    fn write(dir: &Path, name: &str, lines: &[&str]) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
        path
    }

    const A1: &str = r#"{"type":"assistant","timestamp":"2026-09-16T08:00:00Z","requestId":"req_1","message":{"id":"msg_1","model":"claude-opus-4-7","usage":{"input_tokens":6,"cache_creation_input_tokens":100,"cache_read_input_tokens":0,"output_tokens":10}}}"#;
    const A1_FINAL: &str = r#"{"type":"assistant","timestamp":"2026-09-16T08:00:01Z","requestId":"req_1","message":{"id":"msg_1","model":"claude-opus-4-7","usage":{"input_tokens":6,"cache_creation_input_tokens":100,"cache_read_input_tokens":0,"output_tokens":150,"cache_creation":{"ephemeral_1h_input_tokens":40}}}}"#;
    const A2: &str = r#"{"type":"assistant","timestamp":"2026-09-17T09:00:00Z","requestId":"req_2","message":{"id":"msg_2","model":"claude-haiku-4-5-20251001","usage":{"input_tokens":50,"output_tokens":5}}}"#;
    const USER: &str = r#"{"type":"user","timestamp":"2026-09-16T08:00:00Z","message":{"role":"user","content":"hi"}}"#;

    #[test]
    fn a_streamed_message_counts_once_with_its_final_numbers() {
        let d = tempfile::TempDir::new().unwrap();
        let path = write(d.path(), "s.jsonl", &[USER, A1, A1_FINAL, "not json", A2]);
        let rows = scan_file(&path).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].day, "2026-09-16");
        assert_eq!(rows[0].model, "claude-opus-4-7");
        assert_eq!(rows[0].tokens.output, 150);
        assert_eq!(rows[0].tokens.cache_write, 100);
        assert_eq!(rows[0].tokens.cache_write_1h, 40);
        assert_eq!(rows[1].model, "claude-haiku-4-5");
        assert_eq!(rows[1].tokens.input, 50);
    }

    #[test]
    fn transcripts_are_found_under_projects_recursively() {
        let d = tempfile::TempDir::new().unwrap();
        let deep = d.path().join("projects/a/subagents");
        std::fs::create_dir_all(&deep).unwrap();
        write(&deep, "x.jsonl", &[A1]);
        write(&d.path().join("projects/a"), "y.jsonl", &[A1]);
        write(&d.path().join("projects/a"), "notes.txt", &["x"]);
        let files = transcript_files(d.path());
        assert_eq!(files.len(), 2);
        assert!(transcript_files(&d.path().join("nowhere")).is_empty());
    }

    #[test]
    fn empty_usage_is_skipped() {
        let d = tempfile::TempDir::new().unwrap();
        let path = write(
            d.path(),
            "s.jsonl",
            &[r#"{"type":"assistant","timestamp":"2026-09-16T08:00:00Z","message":{"id":"m","model":"claude-opus-4-7","usage":{"input_tokens":0,"output_tokens":0}}}"#],
        );
        assert!(scan_file(&path).unwrap().is_empty());
    }
}
