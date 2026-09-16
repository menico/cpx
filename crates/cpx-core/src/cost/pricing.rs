//! What tokens cost, per model, in USD per token.
//!
//! The tables are the published list prices at the time of writing; they
//! are what turns a transcript's token counts into a dollar figure. A model
//! the table does not know is reported as unpriced rather than priced at
//! zero: a zero would read as "free", and nothing about an unknown model is
//! free.

use crate::config::Provider;

/// Per-token USD rates for one model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rates {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    /// Writing to the prompt cache. `None` bills at the input rate (Codex)
    /// or is the 5-minute cache write rate (Claude).
    pub cache_write: f64,
    /// Prompts longer than this many tokens bill at `above` instead.
    pub long_context: Option<(u64, LongContextRates)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LongContextRates {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

const M: f64 = 1e-6;

/// Long-context pricing: the threshold in tokens, then four rates.
type LongRow = (u64, f64, f64, f64, f64);
/// One table row: the model, four rates in USD per million, and optional
/// long-context pricing.
type PriceRow = (&'static str, f64, f64, f64, f64, Option<LongRow>);

/// Claude: (model, input, output, cache write 5m, cache read, long-context threshold + rates)
const CLAUDE: &[PriceRow] = &[
    ("claude-fable-5-1", 10.0, 50.0, 12.5, 1.0, None),
    ("claude-fable-5", 10.0, 50.0, 12.5, 1.0, None),
    ("claude-opus-5", 5.0, 25.0, 6.25, 0.5, None),
    ("claude-sonnet-5", 3.0, 15.0, 3.75, 0.3, Some((200_000, 6.0, 22.5, 7.5, 0.6))),
    ("claude-haiku-4-5", 1.0, 5.0, 1.25, 0.1, None),
    ("claude-opus-4-8", 5.0, 25.0, 6.25, 0.5, None),
    ("claude-opus-4-7", 5.0, 25.0, 6.25, 0.5, None),
    ("claude-opus-4-6", 5.0, 25.0, 6.25, 0.5, None),
    ("claude-opus-4-5", 5.0, 25.0, 6.25, 0.5, None),
    ("claude-sonnet-4-6", 3.0, 15.0, 3.75, 0.3, None),
    ("claude-sonnet-4-5", 3.0, 15.0, 3.75, 0.3, Some((200_000, 6.0, 22.5, 7.5, 0.6))),
    ("claude-opus-4-1", 15.0, 75.0, 18.75, 1.5, None),
    ("claude-opus-4", 15.0, 75.0, 18.75, 1.5, None),
    ("claude-sonnet-4", 3.0, 15.0, 3.75, 0.3, Some((200_000, 6.0, 22.5, 7.5, 0.6))),
    ("claude-3-7-sonnet", 3.0, 15.0, 3.75, 0.3, None),
    ("claude-3-5-haiku", 0.8, 4.0, 1.0, 0.08, None),
];

/// Codex / OpenAI: (model, input, output, cache read, cache write or -1 for "input rate", long-context)
const CODEX: &[PriceRow] = &[
    ("gpt-5", 1.25, 10.0, 0.125, -1.0, None),
    ("gpt-5-codex", 1.25, 10.0, 0.125, -1.0, None),
    ("gpt-5-mini", 0.25, 2.0, 0.025, -1.0, None),
    ("gpt-5-nano", 0.05, 0.4, 0.005, -1.0, None),
    ("gpt-5-pro", 15.0, 120.0, 15.0, -1.0, None),
    ("gpt-5.1", 1.25, 10.0, 0.125, -1.0, None),
    ("gpt-5.1-codex", 1.25, 10.0, 0.125, -1.0, None),
    ("gpt-5.1-codex-max", 1.25, 10.0, 0.125, -1.0, None),
    ("gpt-5.1-codex-mini", 0.25, 2.0, 0.025, -1.0, None),
    ("gpt-5.2", 1.75, 14.0, 0.175, -1.0, None),
    ("gpt-5.2-codex", 1.75, 14.0, 0.175, -1.0, None),
    ("gpt-5.2-pro", 21.0, 168.0, 21.0, -1.0, None),
    ("gpt-5.3-codex", 1.75, 14.0, 0.175, -1.0, None),
    ("gpt-5.3-codex-spark", 0.0, 0.0, 0.0, 0.0, None),
    ("gpt-5.4", 2.5, 15.0, 0.25, -1.0, Some((272_000, 5.0, 22.5, 0.5, 5.0))),
    ("gpt-5.4-mini", 0.75, 4.5, 0.075, -1.0, None),
    ("gpt-5.4-nano", 0.2, 1.25, 0.02, -1.0, None),
    ("gpt-5.4-pro", 30.0, 180.0, 30.0, -1.0, None),
    ("gpt-5.5", 5.0, 30.0, 0.5, -1.0, Some((272_000, 10.0, 45.0, 1.0, 10.0))),
    ("gpt-5.5-pro", 30.0, 180.0, 30.0, -1.0, None),
    ("gpt-5.6-sol", 5.0, 30.0, 0.5, 6.25, Some((272_000, 10.0, 45.0, 1.0, 12.5))),
    ("gpt-5.6-terra", 2.0, 12.0, 0.2, 2.5, Some((272_000, 4.0, 18.0, 0.4, 5.0))),
    ("gpt-5.6-luna", 0.2, 1.2, 0.02, 0.25, Some((272_000, 0.4, 1.8, 0.04, 0.5))),
    ("gpt-6-astra", 10.0, 50.0, 1.0, 12.5, Some((272_000, 20.0, 75.0, 2.0, 25.0))),
];

fn from_row(row: &PriceRow, claude: bool) -> Rates {
    let (_, input, output, a, b, long) = *row;
    // The Claude rows list (write, read); the Codex rows list (read, write).
    let (cache_write, cache_read) = if claude { (a, b) } else { (if b < 0.0 { input } else { b }, a) };
    Rates {
        input: input * M,
        output: output * M,
        cache_read: cache_read * M,
        cache_write: cache_write * M,
        long_context: long.map(|(threshold, li, lo, lc1, lc2)| {
            let (write, read) = if claude { (lc1, lc2) } else { (lc2, lc1) };
            (
                threshold,
                LongContextRates {
                    input: li * M,
                    output: lo * M,
                    cache_read: read * M,
                    cache_write: write * M,
                },
            )
        }),
    }
}

/// Reduce a model name to the table's spelling: strip vendor prefixes and
/// date suffixes, and route aliases.
pub fn normalize(provider: Provider, model: &str) -> String {
    let mut name = model.trim().to_string();
    match provider {
        Provider::Claude => {
            if let Some(rest) = name.strip_prefix("anthropic.") {
                name = rest.to_string();
            }
            // Bedrock cross-region ids: `us.anthropic.claude-...-v1:0`.
            if let Some(index) = name.rfind(".claude-") {
                name = name[index + 1..].to_string();
            }
            if let Some(index) = name.rfind("-v") {
                if name[index + 2..].chars().all(|c| c.is_ascii_digit() || c == ':') && name[index + 2..].contains(':') {
                    name.truncate(index);
                }
            }
            // Dated snapshots: `claude-opus-4-5-20251101`.
            if name.len() > 9 {
                let (base, suffix) = name.split_at(name.len() - 9);
                if suffix.starts_with('-') && suffix[1..].chars().all(|c| c.is_ascii_digit()) {
                    name = base.to_string();
                }
            }
        }
        Provider::Codex => {
            if let Some(rest) = name.strip_prefix("openai/") {
                name = rest.to_string();
            }
            if name == "gpt-5.6" {
                name = "gpt-5.6-sol".to_string();
            }
            if name == "gpt-reserve" {
                name = "gpt-5.6-luna".to_string();
            }
            // Dated snapshots: `gpt-5.4-2026-03-01`.
            if name.len() > 11 {
                let (base, suffix) = name.split_at(name.len() - 11);
                let dated = suffix.starts_with('-')
                    && suffix[1..].chars().all(|c| c.is_ascii_digit() || c == '-')
                    && suffix.matches('-').count() == 3;
                if dated && lookup(provider, base).is_some() {
                    name = base.to_string();
                }
            }
        }
    }
    name
}

/// The rates for a normalized model name, if the table knows it.
pub fn lookup(provider: Provider, model: &str) -> Option<Rates> {
    match provider {
        Provider::Claude => CLAUDE.iter().find(|r| r.0 == model).map(|r| from_row(r, true)),
        Provider::Codex => CODEX.iter().find(|r| r.0 == model).map(|r| from_row(r, false)),
    }
}

/// Token counts as the transcripts report them. Claude bills every bucket
/// additively; Codex's `input` already contains the cached reads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    /// Claude only: the part of `cache_write` kept for an hour, billed at
    /// twice the input rate.
    #[serde(default)]
    pub cache_write_1h: u64,
}

impl Tokens {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }

    pub fn add(&mut self, other: &Tokens) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
        self.cache_write_1h += other.cache_write_1h;
    }
}

/// The cost of some tokens on a model, or `None` when the model is unknown.
pub fn cost_usd(provider: Provider, model: &str, tokens: &Tokens) -> Option<f64> {
    let rates = lookup(provider, model)?;
    let n = |v: u64| v as f64;
    Some(match provider {
        Provider::Claude => {
            let write_1h = tokens.cache_write_1h.min(tokens.cache_write);
            let write_5m = tokens.cache_write - write_1h;
            let long = tokens.input + tokens.cache_read + tokens.cache_write;
            let (input, output, read, write) = match rates.long_context {
                Some((threshold, above)) if long > threshold => (above.input, above.output, above.cache_read, above.cache_write),
                _ => (rates.input, rates.output, rates.cache_read, rates.cache_write),
            };
            n(tokens.input) * input
                + n(tokens.cache_read) * read
                + n(write_5m) * write
                + n(write_1h) * input * 2.0
                + n(tokens.output) * output
        }
        Provider::Codex => {
            let total_input = tokens.input;
            let cached = tokens.cache_read.min(total_input);
            let after_cache = total_input - cached;
            let cache_write = tokens.cache_write.min(after_cache);
            let uncached = after_cache - cache_write;
            let (input, output, read, write) = match rates.long_context {
                Some((threshold, above)) if total_input > threshold => (above.input, above.output, above.cache_read, above.cache_write),
                _ => (rates.input, rates.output, rates.cache_read, rates.cache_write),
            };
            n(uncached) * input + n(cached) * read + n(cache_write) * write + n(tokens.output) * output
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_names_normalise_to_the_table() {
        assert_eq!(normalize(Provider::Claude, "claude-opus-4-5-20251101"), "claude-opus-4-5");
        assert_eq!(normalize(Provider::Claude, "claude-fable-5-1"), "claude-fable-5-1");
        assert_eq!(normalize(Provider::Claude, "anthropic.claude-sonnet-4-5-20250929-v1:0"), "claude-sonnet-4-5");
        assert_eq!(normalize(Provider::Claude, "us.anthropic.claude-opus-4-6-v1:0"), "claude-opus-4-6");
        assert_eq!(normalize(Provider::Claude, "<synthetic>"), "<synthetic>");
    }

    #[test]
    fn codex_names_normalise_to_the_table() {
        assert_eq!(normalize(Provider::Codex, "openai/gpt-5.4"), "gpt-5.4");
        assert_eq!(normalize(Provider::Codex, "gpt-5.6"), "gpt-5.6-sol");
        assert_eq!(normalize(Provider::Codex, "gpt-5.4-2026-03-01"), "gpt-5.4");
        assert_eq!(normalize(Provider::Codex, "gpt-9-2026-03-01"), "gpt-9-2026-03-01");
    }

    #[test]
    fn claude_cost_bills_every_bucket_additively() {
        // Sonnet 4.6: $3 in, $15 out, $3.75 write, $0.30 read per million.
        let tokens = Tokens {
            input: 1_000_000,
            output: 1_000_000,
            cache_read: 1_000_000,
            cache_write: 1_000_000,
            cache_write_1h: 0,
        };
        let cost = cost_usd(Provider::Claude, "claude-sonnet-4-6", &tokens).unwrap();
        assert!((cost - 22.05).abs() < 1e-9, "{cost}");
    }

    #[test]
    fn claude_one_hour_cache_writes_cost_twice_the_input_rate() {
        let tokens = Tokens {
            cache_write: 1_000_000,
            cache_write_1h: 1_000_000,
            ..Default::default()
        };
        let cost = cost_usd(Provider::Claude, "claude-sonnet-4-6", &tokens).unwrap();
        assert!((cost - 6.0).abs() < 1e-9, "{cost}");
    }

    #[test]
    fn claude_long_prompts_use_the_long_context_rates() {
        let short = Tokens { input: 100_000, ..Default::default() };
        let long = Tokens { input: 300_000, ..Default::default() };
        assert!((cost_usd(Provider::Claude, "claude-sonnet-4-5", &short).unwrap() - 0.3).abs() < 1e-9);
        assert!((cost_usd(Provider::Claude, "claude-sonnet-4-5", &long).unwrap() - 1.8).abs() < 1e-9);
    }

    #[test]
    fn codex_cached_reads_are_a_subset_of_input() {
        // gpt-5.4: $2.50 in, $15 out, $0.25 cached per million.
        let tokens = Tokens {
            input: 100_000,
            output: 10_000,
            cache_read: 60_000,
            ..Default::default()
        };
        let cost = cost_usd(Provider::Codex, "gpt-5.4", &tokens).unwrap();
        let expected = 40_000.0 * 2.5e-6 + 60_000.0 * 0.25e-6 + 10_000.0 * 15e-6;
        assert!((cost - expected).abs() < 1e-9, "{cost} vs {expected}");
    }

    #[test]
    fn codex_cache_writes_bill_at_the_input_rate_when_unpriced() {
        let tokens = Tokens { input: 100_000, cache_write: 100_000, ..Default::default() };
        let cost = cost_usd(Provider::Codex, "gpt-5.4", &tokens).unwrap();
        assert!((cost - 0.25).abs() < 1e-9, "{cost}");
        let sol = cost_usd(Provider::Codex, "gpt-5.6-sol", &tokens).unwrap();
        assert!((sol - 0.625).abs() < 1e-9, "{sol}");
    }

    #[test]
    fn codex_long_prompts_use_the_long_context_rates() {
        let tokens = Tokens { input: 300_000, ..Default::default() };
        let cost = cost_usd(Provider::Codex, "gpt-5.4", &tokens).unwrap();
        assert!((cost - 1.5).abs() < 1e-9, "{cost}");
    }

    #[test]
    fn unknown_models_are_unpriced_not_free() {
        assert_eq!(cost_usd(Provider::Claude, "claude-9", &Tokens::default()), None);
        assert_eq!(cost_usd(Provider::Codex, "gpt-9", &Tokens::default()), None);
        assert_eq!(cost_usd(Provider::Codex, "gpt-5.3-codex-spark", &Tokens { input: 5, ..Default::default() }), Some(0.0));
    }
}
