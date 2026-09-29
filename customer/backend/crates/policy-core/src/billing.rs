#![forbid(unsafe_code)]

//! Exact arithmetic for the Customer-local monetary billing model.
//! One currency unit is represented by one billion nanos. Prices are per
//! million tokens; no binary floating point enters a charge or exchange rate.

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod admission;
pub mod plan;
pub mod schedule;

pub const NANOS_PER_UNIT: i128 = 1_000_000_000;
pub const TOKENS_PER_MILLION: i128 = 1_000_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Currency {
    Cny,
    Usd,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Money {
    pub currency: Currency,
    pub nanos: i64,
}

impl Money {
    pub const fn zero(currency: Currency) -> Self {
        Self { currency, nanos: 0 }
    }

    pub fn parse(currency: Currency, value: &str) -> Result<Self, BillingError> {
        let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
        if whole.is_empty()
            || !whole.bytes().all(|byte| byte.is_ascii_digit())
            || fraction.len() > 9
            || !fraction.bytes().all(|byte| byte.is_ascii_digit())
            || (value.contains('.') && fraction.is_empty())
        {
            return Err(BillingError::InvalidDecimal);
        }
        let whole = whole.parse::<i128>().map_err(|_| BillingError::Overflow)?;
        let fraction = if fraction.is_empty() {
            0
        } else {
            fraction
                .parse::<i128>()
                .map_err(|_| BillingError::Overflow)?
                * 10_i128.pow(9 - fraction.len() as u32)
        };
        let nanos = whole
            .checked_mul(NANOS_PER_UNIT)
            .and_then(|value| value.checked_add(fraction))
            .and_then(|value| i64::try_from(value).ok())
            .ok_or(BillingError::Overflow)?;
        Ok(Self { currency, nanos })
    }

    pub fn checked_add(self, other: Self) -> Result<Self, BillingError> {
        if self.currency != other.currency {
            return Err(BillingError::CurrencyMismatch);
        }
        Ok(Self {
            currency: self.currency,
            nanos: self
                .nanos
                .checked_add(other.nanos)
                .ok_or(BillingError::Overflow)?,
        })
    }

    pub fn decimal(self) -> String {
        let negative = self.nanos < 0;
        let magnitude = i128::from(self.nanos).abs();
        let whole = magnitude / NANOS_PER_UNIT;
        let fraction = magnitude % NANOS_PER_UNIT;
        let sign = if negative { "-" } else { "" };
        if fraction == 0 {
            format!("{sign}{whole}")
        } else {
            let fraction = format!("{fraction:09}");
            format!("{sign}{whole}.{}", fraction.trim_end_matches('0'))
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExchangeRate {
    pub from: Currency,
    pub to: Currency,
    /// Destination currency nanos per one source currency unit.
    pub destination_nanos_per_unit: i64,
}

impl ExchangeRate {
    pub fn convert(self, amount: Money) -> Result<Money, BillingError> {
        if amount.currency != self.from || self.from == self.to {
            return Err(BillingError::CurrencyMismatch);
        }
        if self.destination_nanos_per_unit <= 0 {
            return Err(BillingError::InvalidRate);
        }
        let numerator = i128::from(amount.nanos)
            .checked_mul(i128::from(self.destination_nanos_per_unit))
            .ok_or(BillingError::Overflow)?;
        let nanos = round_div(numerator, NANOS_PER_UNIT)?;
        Ok(Money {
            currency: self.to,
            nanos: i64::try_from(nanos).map_err(|_| BillingError::Overflow)?,
        })
    }

    pub fn convert_reverse(self, amount: Money) -> Result<Money, BillingError> {
        if amount.currency != self.to || self.from == self.to {
            return Err(BillingError::CurrencyMismatch);
        }
        if self.destination_nanos_per_unit <= 0 {
            return Err(BillingError::InvalidRate);
        }
        let numerator = i128::from(amount.nanos)
            .checked_mul(NANOS_PER_UNIT)
            .ok_or(BillingError::Overflow)?;
        let nanos = round_div(numerator, i128::from(self.destination_nanos_per_unit))?;
        Ok(Money {
            currency: self.from,
            nanos: i64::try_from(nanos).map_err(|_| BillingError::Overflow)?,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TokenUsage {
    pub input: u64,
    pub cached_read: u64,
    pub cached_write: u64,
    pub output: u64,
    pub image_input: u64,
    pub image_output: u64,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TokenPrices {
    /// Each value is currency nanos per one million tokens. `None` means that
    /// the category is unsupported, not that it is free.
    pub input: Option<i64>,
    pub cached_read: Option<i64>,
    pub cached_write: Option<i64>,
    pub output: Option<i64>,
    pub image_input: Option<i64>,
    pub image_output: Option<i64>,
}

impl TokenPrices {
    pub fn charge(self, currency: Currency, usage: TokenUsage) -> Result<Money, BillingError> {
        self.validate()?;
        let lines = [
            (usage.input, self.input),
            (usage.cached_read, self.cached_read),
            (usage.cached_write, self.cached_write),
            (usage.output, self.output),
            (usage.image_input, self.image_input),
            (usage.image_output, self.image_output),
        ];
        let mut numerator = 0_i128;
        for (tokens, price) in lines {
            if tokens == 0 {
                continue;
            }
            let price = price.ok_or(BillingError::UnsupportedUsage)?;
            if price < 0 {
                return Err(BillingError::InvalidRate);
            }
            let line = i128::from(tokens)
                .checked_mul(i128::from(price))
                .ok_or(BillingError::Overflow)?;
            numerator = numerator.checked_add(line).ok_or(BillingError::Overflow)?;
        }
        let nanos = round_div(numerator, TOKENS_PER_MILLION)?;
        Ok(Money {
            currency,
            nanos: i64::try_from(nanos).map_err(|_| BillingError::Overflow)?,
        })
    }

    /// A model's one-million-token reference expense, without assuming a
    /// particular input/output mix. This is admission guidance, not a cap.
    pub fn reference(self, currency: Currency) -> Result<Money, BillingError> {
        self.validate()?;
        let maximum = [
            self.input,
            self.cached_read,
            self.cached_write,
            self.output,
            self.image_input,
            self.image_output,
        ]
        .into_iter()
        .flatten()
        .max()
        .ok_or(BillingError::UnsupportedUsage)?;
        if maximum < 0 {
            return Err(BillingError::InvalidRate);
        }
        Ok(Money {
            currency,
            nanos: maximum,
        })
    }

    pub fn validate(self) -> Result<(), BillingError> {
        if [
            self.input,
            self.cached_read,
            self.cached_write,
            self.output,
            self.image_input,
            self.image_output,
        ]
        .into_iter()
        .flatten()
        .any(|price| price < 0)
        {
            return Err(BillingError::InvalidRate);
        }
        Ok(())
    }
}

pub fn image_charge(unit_price: Money, confirmed_count: u32) -> Result<Money, BillingError> {
    if unit_price.nanos < 0 {
        return Err(BillingError::InvalidRate);
    }
    Ok(Money {
        currency: unit_price.currency,
        nanos: unit_price
            .nanos
            .checked_mul(i64::from(confirmed_count))
            .ok_or(BillingError::Overflow)?,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdmissionSnapshot {
    pub balance: Money,
    pub in_flight_reference: Money,
    pub in_flight_count: u32,
    pub max_parallel: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionDecision {
    Admit,
    Queue,
    RejectInsufficientBalance,
}

impl AdmissionSnapshot {
    /// Evaluate the member's current state. A queued request must call this
    /// again with a fresh balance and price reference when it wakes.
    pub fn decide(self, request_reference: Money) -> Result<AdmissionDecision, BillingError> {
        if self.balance.currency != self.in_flight_reference.currency
            || self.balance.currency != request_reference.currency
        {
            return Err(BillingError::CurrencyMismatch);
        }
        if self.in_flight_reference.nanos < 0 || request_reference.nanos < 0 {
            return Err(BillingError::InvalidRate);
        }
        if self.max_parallel == Some(0) {
            return Err(BillingError::InvalidParallelLimit);
        }
        if self.balance.nanos <= 0 {
            return Ok(if self.in_flight_count == 0 {
                AdmissionDecision::RejectInsufficientBalance
            } else {
                AdmissionDecision::Queue
            });
        }
        if self
            .max_parallel
            .is_some_and(|limit| self.in_flight_count >= limit)
        {
            return Ok(AdmissionDecision::Queue);
        }
        if self.in_flight_count == 0 {
            return Ok(AdmissionDecision::Admit);
        }
        let exposure =
            i128::from(self.in_flight_reference.nanos) + i128::from(request_reference.nanos);
        Ok(if i128::from(self.balance.nanos) >= exposure {
            AdmissionDecision::Admit
        } else {
            AdmissionDecision::Queue
        })
    }
}

fn round_div(numerator: i128, denominator: i128) -> Result<i128, BillingError> {
    if numerator < 0 || denominator <= 0 {
        return Err(BillingError::InvalidRate);
    }
    numerator
        .checked_add(denominator / 2)
        .map(|value| value / denominator)
        .ok_or(BillingError::Overflow)
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum BillingError {
    #[error("invalid decimal amount")]
    InvalidDecimal,
    #[error("unsupported usage category")]
    UnsupportedUsage,
    #[error("image size and quality have no matching price")]
    UnsupportedImageSpec,
    #[error("requested image count must be positive")]
    InvalidImageCount,
    #[error("time window changes the model's billing dimensions")]
    InconsistentRateCard,
    #[error("currency mismatch")]
    CurrencyMismatch,
    #[error("invalid price or exchange rate")]
    InvalidRate,
    #[error("billing amount overflow")]
    Overflow,
    #[error("parallel request limit must be positive")]
    InvalidParallelLimit,
    #[error("in-flight request tracker is unavailable")]
    TrackerUnavailable,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_amounts_are_exact_and_reject_ambiguous_input() {
        assert_eq!(
            Money::parse(Currency::Usd, "1.025").unwrap().nanos,
            1_025_000_000
        );
        for input in ["", "-1", "1.", ".1", "1.0000000001", "1e3", " 1", "1,000"] {
            assert_eq!(
                Money::parse(Currency::Usd, input),
                Err(BillingError::InvalidDecimal)
            );
        }
        assert_eq!(
            Money::parse(Currency::Usd, "9223372037"),
            Err(BillingError::Overflow)
        );
    }

    #[test]
    fn token_categories_charge_once_and_keep_small_usage() {
        let prices = TokenPrices {
            input: Some(1_000_000_000),
            cached_read: Some(200_000_000),
            output: Some(4_000_000_000),
            ..TokenPrices::default()
        };
        let usage = TokenUsage {
            input: 500_000,
            cached_read: 500_000,
            output: 100_000,
            ..TokenUsage::default()
        };
        assert_eq!(
            prices.charge(Currency::Usd, usage).unwrap().nanos,
            1_000_000_000
        );
        assert_eq!(
            prices.reference(Currency::Usd).unwrap().nanos,
            4_000_000_000
        );
        assert_eq!(
            prices.charge(
                Currency::Usd,
                TokenUsage {
                    image_output: 1,
                    ..usage
                }
            ),
            Err(BillingError::UnsupportedUsage)
        );
        assert_eq!(
            TokenPrices {
                input: Some(1),
                ..TokenPrices::default()
            }
            .charge(
                Currency::Usd,
                TokenUsage {
                    input: 1,
                    ..TokenUsage::default()
                }
            )
            .unwrap()
            .nanos,
            0
        );
    }

    #[test]
    fn exchange_and_image_charges_are_exact() {
        let dollars = Money::parse(Currency::Usd, "0.5").unwrap();
        let rate = ExchangeRate {
            from: Currency::Usd,
            to: Currency::Cny,
            destination_nanos_per_unit: 7_200_000_000,
        };
        let yuan = rate.convert(dollars).unwrap();
        assert_eq!(yuan.nanos, 3_600_000_000);
        assert_eq!(rate.convert_reverse(yuan).unwrap(), dollars);
        assert_eq!(yuan.decimal(), "3.6");
        assert_eq!(
            Money {
                currency: Currency::Cny,
                nanos: -500_000_001
            }
            .decimal(),
            "-0.500000001"
        );
        assert_eq!(image_charge(yuan, 3).unwrap().nanos, 10_800_000_000);
        assert_eq!(
            yuan.checked_add(dollars),
            Err(BillingError::CurrencyMismatch)
        );
    }

    #[test]
    fn missing_price_is_not_a_free_price() {
        let usage = TokenUsage {
            output: 1,
            ..TokenUsage::default()
        };
        assert_eq!(
            TokenPrices::default().charge(Currency::Cny, usage),
            Err(BillingError::UnsupportedUsage)
        );
        assert_eq!(
            TokenPrices {
                output: Some(0),
                ..TokenPrices::default()
            }
            .charge(Currency::Cny, usage)
            .unwrap(),
            Money::zero(Currency::Cny)
        );
    }

    #[test]
    fn admission_uses_exposure_and_allows_only_one_positive_balance_exception() {
        let usd = Currency::Usd;
        let reference = Money {
            currency: usd,
            nanos: 4_000_000_000,
        };
        let empty = AdmissionSnapshot {
            balance: Money {
                currency: usd,
                nanos: 1_000_000_000,
            },
            in_flight_reference: Money::zero(usd),
            in_flight_count: 0,
            max_parallel: None,
        };
        assert_eq!(empty.decide(reference).unwrap(), AdmissionDecision::Admit);
        assert_eq!(
            AdmissionSnapshot {
                in_flight_reference: reference,
                in_flight_count: 1,
                ..empty
            }
            .decide(reference)
            .unwrap(),
            AdmissionDecision::Queue
        );
        assert_eq!(
            AdmissionSnapshot {
                balance: Money::zero(usd),
                ..empty
            }
            .decide(reference)
            .unwrap(),
            AdmissionDecision::RejectInsufficientBalance
        );
        assert_eq!(
            AdmissionSnapshot {
                max_parallel: Some(1),
                in_flight_count: 1,
                ..empty
            }
            .decide(Money::zero(usd))
            .unwrap(),
            AdmissionDecision::Queue
        );
    }
}
