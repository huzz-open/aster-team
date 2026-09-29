//! Versioned public-model prices. Internal accounts and channels deliberately
//! have no place in this contract and cannot select a different member price.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::Date;

use super::schedule::{PriceSchedule, ScheduleError};
use super::{BillingError, Currency, Money, TokenPrices, TokenUsage, image_charge};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "rates", rename_all = "snake_case")]
pub enum RateCard {
    Tokens(TokenPrices),
    ContextTokens {
        input_threshold: u64,
        short: TokenPrices,
        long: TokenPrices,
    },
    Images {
        unit_prices: Vec<ImageUnitPrice>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImageSpec {
    pub size: String,
    pub quality: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImageUnitPrice {
    pub spec: ImageSpec,
    pub unit_price: Money,
}

impl RateCard {
    fn token_dimensions(prices: &TokenPrices) -> [bool; 6] {
        [
            prices.input,
            prices.cached_read,
            prices.cached_write,
            prices.output,
            prices.image_input,
            prices.image_output,
        ]
        .map(|price| price.is_some())
    }

    fn same_dimensions(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Tokens(a), Self::Tokens(b)) => {
                Self::token_dimensions(a) == Self::token_dimensions(b)
            }
            (
                Self::ContextTokens {
                    input_threshold: a_threshold,
                    short: a_short,
                    long: a_long,
                },
                Self::ContextTokens {
                    input_threshold: b_threshold,
                    short: b_short,
                    long: b_long,
                },
            ) => {
                a_threshold == b_threshold
                    && Self::token_dimensions(a_short) == Self::token_dimensions(b_short)
                    && Self::token_dimensions(a_long) == Self::token_dimensions(b_long)
            }
            (Self::Images { unit_prices: a }, Self::Images { unit_prices: b }) => {
                let a: std::collections::HashSet<_> = a.iter().map(|price| &price.spec).collect();
                let b: std::collections::HashSet<_> = b.iter().map(|price| &price.spec).collect();
                a == b
            }
            _ => false,
        }
    }

    fn validate(&self, currency: Currency) -> Result<(), BillingError> {
        match self {
            Self::Tokens(prices) => {
                prices.validate()?;
                prices.reference(currency)?;
                Ok(())
            }
            Self::ContextTokens {
                input_threshold,
                short,
                long,
            } => {
                if *input_threshold == 0
                    || Self::token_dimensions(short) != Self::token_dimensions(long)
                {
                    return Err(BillingError::InconsistentRateCard);
                }
                short.reference(currency)?;
                long.reference(currency)?;
                Ok(())
            }
            Self::Images { unit_prices } => {
                if unit_prices.is_empty() {
                    return Err(BillingError::UnsupportedImageSpec);
                }
                let mut seen = std::collections::HashSet::new();
                for item in unit_prices {
                    if item.spec.size.trim().is_empty()
                        || item.spec.quality.trim().is_empty()
                        || !seen.insert(&item.spec)
                    {
                        return Err(BillingError::UnsupportedImageSpec);
                    }
                    if item.unit_price.currency != currency {
                        return Err(BillingError::CurrencyMismatch);
                    }
                    image_charge(item.unit_price, 1)?;
                }
                Ok(())
            }
        }
    }

    pub fn charge(&self, usage: VerifiedUsage, currency: Currency) -> Result<Money, BillingError> {
        self.validate(currency)?;
        match (self, usage) {
            (Self::Tokens(prices), VerifiedUsage::Tokens(usage)) => prices.charge(currency, usage),
            (
                Self::ContextTokens {
                    input_threshold,
                    short,
                    long,
                },
                VerifiedUsage::Tokens(usage),
            ) => {
                let total_input = [
                    usage.input,
                    usage.cached_read,
                    usage.cached_write,
                    usage.image_input,
                ]
                .into_iter()
                .try_fold(0_u64, |total, value| {
                    total.checked_add(value).ok_or(BillingError::Overflow)
                })?;
                if total_input > *input_threshold {
                    long.charge(currency, usage)
                } else {
                    short.charge(currency, usage)
                }
            }
            (
                Self::Images { .. },
                VerifiedUsage::Images {
                    confirmed_count,
                    spec,
                },
            ) => {
                let unit_price = self.image_price(&spec, currency)?;
                image_charge(unit_price, confirmed_count)
            }
            _ => Err(BillingError::UnsupportedUsage),
        }
    }

    pub fn reference(
        &self,
        currency: Currency,
        request: &ReferenceRequest,
    ) -> Result<Money, BillingError> {
        self.validate(currency)?;
        match self {
            Self::Tokens(prices) => prices.reference(currency),
            Self::ContextTokens { short, long, .. } => {
                let short = short.reference(currency)?;
                let long = long.reference(currency)?;
                Ok(if short.nanos > long.nanos {
                    short
                } else {
                    long
                })
            }
            Self::Images { .. } => match request {
                ReferenceRequest::Images {
                    requested_count,
                    spec,
                } => {
                    if *requested_count == 0 {
                        return Err(BillingError::InvalidImageCount);
                    }
                    image_charge(self.image_price(spec, currency)?, *requested_count)
                }
                ReferenceRequest::TokenOneMillion => Err(BillingError::UnsupportedUsage),
            },
        }
    }

    fn image_price(&self, spec: &ImageSpec, currency: Currency) -> Result<Money, BillingError> {
        let Self::Images { unit_prices } = self else {
            return Err(BillingError::UnsupportedUsage);
        };
        let price = unit_prices
            .iter()
            .find(|item| &item.spec == spec)
            .ok_or(BillingError::UnsupportedImageSpec)?
            .unit_price;
        if price.currency != currency {
            return Err(BillingError::CurrencyMismatch);
        }
        Ok(price)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum VerifiedUsage {
    /// Categories are already normalized and mutually exclusive.
    Tokens(TokenUsage),
    /// Only supplier-confirmed billable images, not requested image count.
    Images {
        confirmed_count: u32,
        spec: ImageSpec,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReferenceRequest {
    TokenOneMillion,
    Images {
        requested_count: u32,
        spec: ImageSpec,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PriceTier {
    pub id: String,
    pub schedule: PriceSchedule<RateCard>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelPriceVersion {
    pub public_model: String,
    pub version: String,
    pub currency: Currency,
    pub tiers: Vec<PriceTier>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SelectedPrice {
    pub public_model: String,
    pub version: String,
    pub tier: String,
    pub currency: Currency,
    pub rate: RateCard,
}

impl ModelPriceVersion {
    pub fn validate(&self) -> Result<(), PlanError> {
        if self.public_model.trim().is_empty()
            || self.version.trim().is_empty()
            || self.tiers.is_empty()
        {
            return Err(PlanError::InvalidIdentity);
        }
        let mut seen = std::collections::HashSet::new();
        let dimensions = &self.tiers[0].schedule.base;
        for tier in &self.tiers {
            if tier.id.trim().is_empty() || !seen.insert(tier.id.as_str()) {
                return Err(PlanError::InvalidTier);
            }
            tier.schedule.validate()?;
            tier.schedule.base.validate(self.currency)?;
            if !tier.schedule.base.same_dimensions(dimensions) {
                return Err(BillingError::InconsistentRateCard.into());
            }
            for window in &tier.schedule.windows {
                window.price.validate(self.currency)?;
                if !window.price.same_dimensions(&tier.schedule.base) {
                    return Err(BillingError::InconsistentRateCard.into());
                }
            }
        }
        Ok(())
    }

    pub fn select(
        &self,
        public_model: &str,
        tier: &str,
        local_date: Date,
        local_minute: u16,
    ) -> Result<SelectedPrice, PlanError> {
        self.validate()?;
        if self.public_model != public_model {
            return Err(PlanError::ModelMismatch);
        }
        let selected_tier = self
            .tiers
            .iter()
            .find(|candidate| candidate.id == tier)
            .ok_or(PlanError::UnknownTier)?;
        let rate = selected_tier
            .schedule
            .price_at(local_date, local_minute)?
            .clone();
        Ok(SelectedPrice {
            public_model: self.public_model.clone(),
            version: self.version.clone(),
            tier: selected_tier.id.clone(),
            currency: self.currency,
            rate,
        })
    }
}

impl SelectedPrice {
    pub fn charge(&self, usage: VerifiedUsage) -> Result<Money, BillingError> {
        self.rate.charge(usage, self.currency)
    }

    pub fn reference(&self, request: &ReferenceRequest) -> Result<Money, BillingError> {
        self.rate.reference(self.currency, request)
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum PlanError {
    #[error("invalid public model or price version")]
    InvalidIdentity,
    #[error("invalid or duplicate service tier")]
    InvalidTier,
    #[error("requested model does not match the price version")]
    ModelMismatch,
    #[error("requested service tier has no price")]
    UnknownTier,
    #[error(transparent)]
    Schedule(#[from] ScheduleError),
    #[error(transparent)]
    Billing(#[from] BillingError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::billing::schedule::PriceWindow;
    use time::macros::date;

    fn text_price(nanos: i64) -> RateCard {
        RateCard::Tokens(TokenPrices {
            input: Some(nanos),
            output: Some(nanos),
            ..TokenPrices::default()
        })
    }

    #[test]
    fn model_and_tier_select_a_locked_price_without_a_channel_key() {
        let plan = ModelPriceVersion {
            public_model: "model-a".into(),
            version: "v2".into(),
            currency: Currency::Usd,
            tiers: vec![PriceTier {
                id: "standard".into(),
                schedule: PriceSchedule {
                    base: text_price(1_000_000_000),
                    windows: vec![PriceWindow {
                        start_date: None,
                        end_date: None,
                        weekdays: 0x7f,
                        start_minute: 0,
                        end_minute: 60,
                        price: text_price(500_000_000),
                    }],
                },
            }],
        };
        plan.validate().unwrap();
        let selected = plan
            .select("model-a", "standard", date!(2024 - 01 - 01), 30)
            .unwrap();
        assert_eq!(selected.version, "v2");
        assert_eq!(
            selected
                .reference(&ReferenceRequest::TokenOneMillion)
                .unwrap()
                .nanos,
            500_000_000
        );
        assert_eq!(
            selected
                .charge(VerifiedUsage::Tokens(TokenUsage {
                    input: 1_000_000,
                    ..TokenUsage::default()
                }))
                .unwrap()
                .nanos,
            500_000_000
        );
        assert_eq!(
            plan.select(
                "internal-channel-model",
                "standard",
                date!(2024 - 01 - 01),
                30
            ),
            Err(PlanError::ModelMismatch)
        );
        assert_eq!(
            plan.select("model-a", "fast", date!(2024 - 01 - 01), 30),
            Err(PlanError::UnknownTier)
        );
        let mut incompatible = plan.clone();
        incompatible.tiers[0].schedule.windows[0].price = RateCard::Images {
            unit_prices: vec![ImageUnitPrice {
                spec: ImageSpec {
                    size: "1024x1024".into(),
                    quality: "high".into(),
                },
                unit_price: Money::parse(Currency::Usd, "0.25").unwrap(),
            }],
        };
        assert_eq!(
            incompatible.validate(),
            Err(PlanError::Billing(BillingError::InconsistentRateCard))
        );
    }

    #[test]
    fn image_count_is_confirmed_and_cannot_be_charged_as_tokens() {
        let high = ImageSpec {
            size: "1024x1024".into(),
            quality: "high".into(),
        };
        let low = ImageSpec {
            size: "1024x1024".into(),
            quality: "low".into(),
        };
        let rate = RateCard::Images {
            unit_prices: vec![
                ImageUnitPrice {
                    spec: high.clone(),
                    unit_price: Money::parse(Currency::Cny, "0.25").unwrap(),
                },
                ImageUnitPrice {
                    spec: low.clone(),
                    unit_price: Money::parse(Currency::Cny, "0.10").unwrap(),
                },
            ],
        };
        rate.validate(Currency::Cny).unwrap();
        assert_eq!(
            rate.charge(
                VerifiedUsage::Images {
                    confirmed_count: 2,
                    spec: high.clone()
                },
                Currency::Cny
            )
            .unwrap()
            .nanos,
            500_000_000
        );
        assert_eq!(
            rate.reference(
                Currency::Cny,
                &ReferenceRequest::Images {
                    requested_count: 2,
                    spec: low
                }
            )
            .unwrap()
            .nanos,
            200_000_000
        );
        assert_eq!(
            rate.charge(
                VerifiedUsage::Images {
                    confirmed_count: 1,
                    spec: ImageSpec {
                        size: "512x512".into(),
                        quality: "high".into()
                    }
                },
                Currency::Cny
            ),
            Err(BillingError::UnsupportedImageSpec)
        );
        assert_eq!(
            rate.charge(VerifiedUsage::Tokens(TokenUsage::default()), Currency::Cny),
            Err(BillingError::UnsupportedUsage)
        );
        assert_eq!(
            rate.reference(
                Currency::Cny,
                &ReferenceRequest::Images {
                    requested_count: 0,
                    spec: high.clone(),
                }
            ),
            Err(BillingError::InvalidImageCount)
        );
        let duplicate = RateCard::Images {
            unit_prices: vec![
                ImageUnitPrice {
                    spec: high.clone(),
                    unit_price: Money::parse(Currency::Cny, "0.25").unwrap(),
                },
                ImageUnitPrice {
                    spec: high,
                    unit_price: Money::parse(Currency::Cny, "0.10").unwrap(),
                },
            ],
        };
        assert_eq!(
            duplicate.validate(Currency::Cny),
            Err(BillingError::UnsupportedImageSpec)
        );
    }

    #[test]
    fn token_priced_image_uses_million_token_reference_before_usage_is_known() {
        let rate = RateCard::Tokens(TokenPrices {
            input: Some(1_000_000_000),
            output: Some(2_000_000_000),
            ..TokenPrices::default()
        });
        let reference = rate
            .reference(
                Currency::Usd,
                &ReferenceRequest::Images {
                    requested_count: 2,
                    spec: ImageSpec {
                        size: "1024x1024".into(),
                        quality: "high".into(),
                    },
                },
            )
            .unwrap();
        assert_eq!(reference.nanos, 2_000_000_000);
        assert_eq!(
            rate.charge(
                VerifiedUsage::Tokens(TokenUsage {
                    input: 100_000,
                    output: 10_000,
                    ..TokenUsage::default()
                }),
                Currency::Usd,
            )
            .unwrap()
            .nanos,
            120_000_000
        );
    }

    #[test]
    fn context_threshold_selects_one_price_for_the_entire_request() {
        let rate = RateCard::ContextTokens {
            input_threshold: 272_000,
            short: TokenPrices {
                input: Some(2_000_000_000),
                cached_read: Some(200_000_000),
                output: Some(10_000_000_000),
                ..TokenPrices::default()
            },
            long: TokenPrices {
                input: Some(4_000_000_000),
                cached_read: Some(400_000_000),
                output: Some(15_000_000_000),
                ..TokenPrices::default()
            },
        };
        let usage = |input, cached_read| {
            VerifiedUsage::Tokens(TokenUsage {
                input,
                cached_read,
                output: 100_000,
                ..TokenUsage::default()
            })
        };
        assert_eq!(
            rate.charge(usage(272_000, 0), Currency::Usd).unwrap().nanos,
            1_544_000_000
        );
        assert_eq!(
            rate.charge(usage(272_001, 0), Currency::Usd).unwrap().nanos,
            2_588_004_000
        );
        assert_eq!(
            rate.charge(usage(200_000, 72_001), Currency::Usd)
                .unwrap()
                .nanos,
            2_328_800_400
        );
        assert_eq!(
            rate.reference(Currency::Usd, &ReferenceRequest::TokenOneMillion)
                .unwrap()
                .nanos,
            15_000_000_000
        );
        let mut invalid = rate.clone();
        if let RateCard::ContextTokens {
            input_threshold, ..
        } = &mut invalid
        {
            *input_threshold = 0;
        }
        assert_eq!(
            invalid.validate(Currency::Usd),
            Err(BillingError::InconsistentRateCard)
        );
    }
}
