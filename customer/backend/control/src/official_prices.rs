//! A deliberately narrow adapter for OpenAI's public Markdown price table.
//! Only exact model IDs whose Standard rates fit our usage dimensions are
//! eligible. Changed/ambiguous source data fails closed to a dated snapshot.

use aster_policy_core::billing::plan::{ModelPriceVersion, PriceTier, RateCard};
use aster_policy_core::billing::schedule::PriceSchedule;
use aster_policy_core::billing::{Currency, Money, TokenPrices};

pub(super) const SOURCE_URL: &str = "https://developers.openai.com/api/docs/pricing.md";
pub(super) const BUILTIN_VERIFIED_AT: &str = "2026-09-28";
const MAX_DOCUMENT_BYTES: usize = 1_000_000;

// Values were checked against the Standard table at SOURCE_URL on 2026-09-28.
// Do not add models with unverified modality, regional or time-dependent
// prices until the runtime can select those rates from verified request usage.
const DEFAULTS: &[(&str, &str, &str, &str)] = &[
    ("gpt-4o", "2.50", "1.25", "10.00"),
    ("gpt-4o-mini", "0.15", "0.075", "0.60"),
    ("gpt-4.1", "2.00", "0.50", "8.00"),
    ("gpt-4.1-mini", "0.40", "0.10", "1.60"),
    ("gpt-4.1-nano", "0.10", "0.025", "0.40"),
    ("gpt-5", "1.25", "0.125", "10.00"),
    ("gpt-5-mini", "0.25", "0.025", "2.00"),
    ("gpt-5-nano", "0.05", "0.005", "0.40"),
    ("gpt-5.1", "1.25", "0.125", "10.00"),
    ("gpt-5.2", "1.75", "0.175", "14.00"),
    ("o3", "2.00", "0.50", "8.00"),
    ("o4-mini", "1.10", "0.275", "4.40"),
];
const LONG_CONTEXT_THRESHOLD: u64 = 272_000;
// Standard: short input, cached input, cache writes, output; then long rates.
const CONTEXT_DEFAULTS: &[(&str, [&str; 8])] = &[
    (
        "gpt-5.5",
        ["5.00", "0.50", "-", "30.00", "10.00", "1.00", "-", "45.00"],
    ),
    (
        "gpt-5.6-sol",
        [
            "4.00", "0.40", "5.00", "20.00", "8.00", "0.80", "10.00", "30.00",
        ],
    ),
    (
        "gpt-5.6-terra",
        [
            "2.00", "0.20", "2.50", "12.00", "4.00", "0.40", "5.00", "18.00",
        ],
    ),
    (
        "gpt-5.6-luna",
        [
            "0.20", "0.02", "0.25", "1.20", "0.40", "0.04", "0.50", "1.80",
        ],
    ),
    (
        "gpt-6-astra",
        [
            "10.00", "1.00", "12.50", "50.00", "20.00", "2.00", "25.00", "75.00",
        ],
    ),
    (
        "gpt-6-sol",
        [
            "2.00", "0.20", "2.50", "10.00", "4.00", "0.40", "5.00", "15.00",
        ],
    ),
    (
        "gpt-6-luna",
        [
            "0.10", "0.01", "0.125", "0.50", "0.20", "0.02", "0.25", "0.75",
        ],
    ),
];

pub(super) fn supported(public_model: &str) -> bool {
    DEFAULTS.iter().any(|(name, ..)| *name == public_model)
        || CONTEXT_DEFAULTS
            .iter()
            .any(|(name, ..)| *name == public_model)
}

pub(super) fn builtin(public_model: &str) -> Option<RateCard> {
    if let Some((_, input, cached_read, output)) =
        DEFAULTS.iter().find(|(name, ..)| *name == public_model)
    {
        return Some(RateCard::Tokens(TokenPrices {
            input: Some(nanos(input)?),
            cached_read: Some(nanos(cached_read)?),
            output: Some(nanos(output)?),
            ..TokenPrices::default()
        }));
    }
    let (_, values) = CONTEXT_DEFAULTS
        .iter()
        .find(|(name, ..)| *name == public_model)?;
    Some(RateCard::ContextTokens {
        input_threshold: LONG_CONTEXT_THRESHOLD,
        short: context_prices(&values[..4])?,
        long: context_prices(&values[4..])?,
    })
}

fn context_prices(values: &[&str]) -> Option<TokenPrices> {
    let [input, cached_read, cached_write, output] = values else {
        return None;
    };
    Some(TokenPrices {
        input: Some(nanos(input)?),
        cached_read: Some(nanos(cached_read)?),
        cached_write: if *cached_write == "-" {
            None
        } else {
            Some(nanos(cached_write)?)
        },
        output: Some(nanos(output)?),
        ..TokenPrices::default()
    })
}

fn nanos(value: &str) -> Option<i64> {
    Money::parse(Currency::Usd, value)
        .ok()
        .map(|money| money.nanos)
}

pub(super) fn plan(public_model: &str, version: String, rate: RateCard) -> ModelPriceVersion {
    ModelPriceVersion {
        public_model: public_model.to_owned(),
        version,
        currency: Currency::Usd,
        tiers: vec![PriceTier {
            id: "standard".to_owned(),
            schedule: PriceSchedule {
                base: rate,
                windows: vec![],
            },
        }],
    }
}

/// Parse only the first, explicitly Standard, text-token table. Duplicate or
/// altered rows or missing columns are never guessed.
pub(super) fn parse_standard(document: &str, public_model: &str) -> Option<RateCard> {
    if !supported(public_model) || document.len() > MAX_DOCUMENT_BYTES {
        return None;
    }
    let table = document
        .split_once("### Standard pricing data\n")?
        .1
        .split_once("\n### Batch pricing data")?
        .0;
    let expected_header = "| Model | Short context input | Short context cached input | Short context cache writes | Short context output | Long context input | Long context cached input | Long context cache writes | Long context output |";
    if table
        .lines()
        .filter(|line| *line == expected_header)
        .count()
        != 1
    {
        return None;
    }
    let mut found = None;
    for line in table.lines().filter(|line| line.starts_with('|')) {
        let columns: Vec<_> = line.trim_matches('|').split('|').map(str::trim).collect();
        let official_name = if public_model == "gpt-5.5" {
            "gpt-5.5 (<272K context length)"
        } else {
            public_model
        };
        if columns.len() != 9 || columns[0] != official_name {
            continue;
        }
        if found.is_some() {
            return None;
        }
        let money = |value: &str| nanos(value.strip_prefix('$')?);
        found = if CONTEXT_DEFAULTS
            .iter()
            .any(|(name, ..)| *name == public_model)
        {
            let amounts: Vec<_> = columns[1..]
                .iter()
                .map(|value| {
                    if *value == "-" {
                        Some(None)
                    } else {
                        money(value).map(Some)
                    }
                })
                .collect::<Option<_>>()?;
            let short = TokenPrices {
                input: amounts[0],
                cached_read: amounts[1],
                cached_write: amounts[2],
                output: amounts[3],
                ..TokenPrices::default()
            };
            let long = TokenPrices {
                input: amounts[4],
                cached_read: amounts[5],
                cached_write: amounts[6],
                output: amounts[7],
                ..TokenPrices::default()
            };
            if short.input.is_none()
                || short.output.is_none()
                || long.input.is_none()
                || long.output.is_none()
                || (short.cached_read.is_some() != long.cached_read.is_some())
                || (short.cached_write.is_some() != long.cached_write.is_some())
            {
                return None;
            }
            Some(RateCard::ContextTokens {
                input_threshold: LONG_CONTEXT_THRESHOLD,
                short,
                long,
            })
        } else {
            if columns[3] != "-" || columns[5..].iter().any(|value| *value != "-") {
                return None;
            }
            Some(RateCard::Tokens(TokenPrices {
                input: Some(money(columns[1])?),
                cached_read: (columns[2] != "-").then(|| money(columns[2])).flatten(),
                output: Some(money(columns[4])?),
                ..TokenPrices::default()
            }))
        };
        if columns[2] != "-"
            && matches!(&found, Some(RateCard::Tokens(prices)) if prices.cached_read.is_none())
        {
            return None;
        }
    }
    found
}

pub(super) async fn fetch_standard(public_model: &str) -> Option<RateCard> {
    if !supported(public_model) {
        return None;
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .ok()?;
    let mut response = client
        .get(SOURCE_URL)
        .header(reqwest::header::ACCEPT, "text/markdown")
        .send()
        .await
        .ok()?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|size| size > MAX_DOCUMENT_BYTES as u64)
    {
        return None;
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if bytes.len().checked_add(chunk.len())? > MAX_DOCUMENT_BYTES {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    parse_standard(std::str::from_utf8(&bytes).ok()?, public_model)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "### Standard pricing data\n| Model | Short context input | Short context cached input | Short context cache writes | Short context output | Long context input | Long context cached input | Long context cache writes | Long context output |\n| --- | --- | --- | --- | --- | --- | --- | --- | --- |\n| gpt-4o-mini | $0.15 | $0.075 | - | $0.60 | - | - | - | - |\n\n### Batch pricing data";

    #[test]
    fn official_table_requires_exact_standard_model_and_dimensions() {
        assert_eq!(parse_standard(TABLE, "gpt-4o-mini"), builtin("gpt-4o-mini"));
        assert!(parse_standard(TABLE, "gpt-6-sol").is_none());
        assert!(
            parse_standard(
                &TABLE.replace("| - | - | - | - |", "| $1 | - | - | - |"),
                "gpt-4o-mini"
            )
            .is_none()
        );
        assert!(
            parse_standard(&TABLE.replace("### Batch pricing data", ""), "gpt-4o-mini").is_none()
        );
        assert!(parse_standard(&TABLE.replace("$0.075", "$garbage"), "gpt-4o-mini").is_none());
        assert!(
            parse_standard(
                &TABLE.replace(
                    "### Batch pricing data",
                    "| gpt-4o-mini | $0.2 | $0.1 | - | $1 | - | - | - | - |\n### Batch pricing data"
                ),
                "gpt-4o-mini"
            )
            .is_none()
        );
    }

    #[test]
    fn every_builtin_price_is_a_valid_standard_plan() {
        for (model, ..) in DEFAULTS {
            plan(model, "snapshot".to_owned(), builtin(model).unwrap())
                .validate()
                .unwrap();
        }
        for (model, ..) in CONTEXT_DEFAULTS {
            plan(model, "snapshot".to_owned(), builtin(model).unwrap())
                .validate()
                .unwrap();
        }
    }

    #[test]
    fn official_gpt6_requires_all_short_and_long_rates() {
        let table = TABLE.replace(
            "| gpt-4o-mini | $0.15 | $0.075 | - | $0.60 | - | - | - | - |",
            "| gpt-6-sol | $2.00 | $0.20 | $2.50 | $10.00 | $4.00 | $0.40 | $5.00 | $15.00 |",
        );
        assert_eq!(parse_standard(&table, "gpt-6-sol"), builtin("gpt-6-sol"));
        assert!(parse_standard(&table.replace("$5.00", "-"), "gpt-6-sol").is_none());
    }

    #[test]
    fn current_standard_models_have_exact_official_rows() {
        let table = TABLE.replace(
            "| gpt-4o-mini | $0.15 | $0.075 | - | $0.60 | - | - | - | - |",
            "| gpt-5.5 (<272K context length) | $5.00 | $0.50 | - | $30.00 | $10.00 | $1.00 | - | $45.00 |\n| gpt-5.6-sol | $4.00 | $0.40 | $5.00 | $20.00 | $8.00 | $0.80 | $10.00 | $30.00 |\n| gpt-5.6-terra | $2.00 | $0.20 | $2.50 | $12.00 | $4.00 | $0.40 | $5.00 | $18.00 |\n| gpt-5.6-luna | $0.20 | $0.02 | $0.25 | $1.20 | $0.40 | $0.04 | $0.50 | $1.80 |",
        );
        for model in ["gpt-5.5", "gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"] {
            assert_eq!(parse_standard(&table, model), builtin(model));
        }
    }
}
