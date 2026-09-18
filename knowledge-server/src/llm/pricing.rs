use crate::llm::types::Usage;

#[derive(Clone, Debug)]
pub struct ModelPricing {
    pub input_per_1k: i64,
    pub cache_read_per_1k: i64,
    pub cache_creation_per_1k: i64,
    pub output_per_1k: i64,
    pub reasoning_per_1k: i64,
}

impl ModelPricing {
    pub fn per_million(input: i64, output: i64) -> Self {
        Self {
            input_per_1k: input / 1000,
            output_per_1k: output / 1000,
            cache_read_per_1k: input / 10000,
            cache_creation_per_1k: input * 125 / 100000,
            reasoning_per_1k: output / 1000,
        }
    }
}

pub fn price(usage: &Usage, pricing: Option<&ModelPricing>) -> i64 {
    let Some(p) = pricing else { return 0 };
    let input = usage.input_tokens as i64 * p.input_per_1k;
    let output = usage.output_tokens as i64 * p.output_per_1k;
    let cache_read = usage.cache_read_tokens as i64 * p.cache_read_per_1k;
    let cache_creation = usage.cache_creation_tokens as i64 * p.cache_creation_per_1k;
    let reasoning = usage.reasoning_tokens as i64 * p.reasoning_per_1k;
    (input + output + cache_read + cache_creation + reasoning) / 1000
}

pub fn format_cost(microusd: i64) -> String {
    let dollars = microusd as f64 / 1_000_000.0;
    format!("${dollars:.6}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_pricing() -> ModelPricing {
        ModelPricing {
            input_per_1k: 3000,
            cache_read_per_1k: 300,
            cache_creation_per_1k: 3750,
            output_per_1k: 15000,
            reasoning_per_1k: 15000,
        }
    }

    #[test]
    fn price_zero_when_no_pricing() {
        let usage = Usage { input_tokens: 100, output_tokens: 50, cache_read_tokens: 0, cache_creation_tokens: 0, reasoning_tokens: 0 };
        assert_eq!(price(&usage, None), 0);
    }

    #[test]
    fn price_zero_when_no_usage() {
        let usage = Usage::default();
        assert_eq!(price(&usage, Some(&sample_pricing())), 0);
    }

    #[test]
    fn price_sums_all_buckets() {
        let usage = Usage {
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 200,
            cache_creation_tokens: 100,
            reasoning_tokens: 50,
        };
        let p = sample_pricing();
        let expected = (1000 * 3000 + 500 * 15000 + 200 * 300 + 100 * 3750 + 50 * 15000) / 1000;
        assert_eq!(price(&usage, Some(&p)), expected);
    }

    #[test]
    fn price_input_only() {
        let usage = Usage { input_tokens: 1000, output_tokens: 0, cache_read_tokens: 0, cache_creation_tokens: 0, reasoning_tokens: 0 };
        let p = sample_pricing();
        assert_eq!(price(&usage, Some(&p)), 3000);
    }

    #[test]
    fn price_cache_read_cheaper_than_input() {
        let input_usage = Usage { input_tokens: 1000, output_tokens: 0, cache_read_tokens: 0, cache_creation_tokens: 0, reasoning_tokens: 0 };
        let cached_usage = Usage { input_tokens: 0, output_tokens: 0, cache_read_tokens: 1000, cache_creation_tokens: 0, reasoning_tokens: 0 };
        let p = sample_pricing();
        assert!(price(&cached_usage, Some(&p)) < price(&input_usage, Some(&p)));
    }

    #[test]
    fn per_million_derives_rates() {
        let p = ModelPricing::per_million(3_000_000, 15_000_000);
        assert_eq!(p.input_per_1k, 3000);
        assert_eq!(p.output_per_1k, 15000);
        assert_eq!(p.cache_read_per_1k, 300);
    }

    #[test]
    fn format_cost_displays_dollars() {
        assert_eq!(format_cost(42100), "$0.042100");
        assert_eq!(format_cost(0), "$0.000000");
        assert_eq!(format_cost(2_829_200), "$2.829200");
    }
}
