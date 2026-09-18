use crate::config::LlmProviderConfig;
use crate::llm::types::Usage;
use std::collections::HashMap;

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

pub fn default_pricing_for(kind: &str, model: &str) -> Option<ModelPricing> {
    let m = model.to_lowercase();
    let k = kind.to_lowercase();
    if k.contains("anthropic") || m.contains("claude") {
        if m.contains("sonnet-4") || m.contains("sonnet-4-6") || m.contains("claude-sonnet-4") {
            return Some(ModelPricing { input_per_1k: 3000, cache_read_per_1k: 300, cache_creation_per_1k: 3750, output_per_1k: 15000, reasoning_per_1k: 0 });
        }
        if m.contains("opus") {
            return Some(ModelPricing { input_per_1k: 15000, cache_read_per_1k: 1500, cache_creation_per_1k: 18750, output_per_1k: 75000, reasoning_per_1k: 0 });
        }
        if m.contains("haiku") {
            return Some(ModelPricing { input_per_1k: 100, cache_read_per_1k: 10, cache_creation_per_1k: 125, output_per_1k: 500, reasoning_per_1k: 0 });
        }
        return Some(ModelPricing { input_per_1k: 3000, cache_read_per_1k: 300, cache_creation_per_1k: 3750, output_per_1k: 15000, reasoning_per_1k: 0 });
    }
    if k.contains("gemini") || m.contains("gemini") {
        if m.contains("2.5-pro") || m.contains("3") {
            return Some(ModelPricing { input_per_1k: 1250, cache_read_per_1k: 312, cache_creation_per_1k: 0, output_per_1k: 5000, reasoning_per_1k: 5000 });
        }
        if m.contains("flash") {
            return Some(ModelPricing { input_per_1k: 300, cache_read_per_1k: 75, cache_creation_per_1k: 0, output_per_1k: 2500, reasoning_per_1k: 2500 });
        }
        return Some(ModelPricing { input_per_1k: 1250, cache_read_per_1k: 312, cache_creation_per_1k: 0, output_per_1k: 5000, reasoning_per_1k: 5000 });
    }
    if k.contains("openai") || m.contains("gpt") {
        if m.contains("gpt-4o") {
            return Some(ModelPricing { input_per_1k: 2500, cache_read_per_1k: 1250, cache_creation_per_1k: 0, output_per_1k: 10000, reasoning_per_1k: 10000 });
        }
        if m.contains("gpt-4") {
            return Some(ModelPricing { input_per_1k: 30000, cache_read_per_1k: 1500, cache_creation_per_1k: 0, output_per_1k: 60000, reasoning_per_1k: 0 });
        }
        if m.contains("o1") || m.contains("o3") || m.contains("o4") {
            return Some(ModelPricing { input_per_1k: 15000, cache_read_per_1k: 7500, cache_creation_per_1k: 0, output_per_1k: 60000, reasoning_per_1k: 60000 });
        }
    }
    if m.contains("kimi") || m.contains("k3") {
        return Some(ModelPricing { input_per_1k: 600, cache_read_per_1k: 60, cache_creation_per_1k: 0, output_per_1k: 2200, reasoning_per_1k: 0 });
    }
    if m.contains("deepseek") {
        return Some(ModelPricing { input_per_1k: 270, cache_read_per_1k: 27, cache_creation_per_1k: 0, output_per_1k: 1100, reasoning_per_1k: 0 });
    }
    if m.contains("llama") || m.contains("mistral") || m.contains("ministral") {
        return Some(ModelPricing { input_per_1k: 200, cache_read_per_1k: 20, cache_creation_per_1k: 0, output_per_1k: 600, reasoning_per_1k: 0 });
    }
    Some(ModelPricing { input_per_1k: 1000, cache_read_per_1k: 100, cache_creation_per_1k: 0, output_per_1k: 3000, reasoning_per_1k: 0 })
}

#[derive(Clone, Default)]
pub struct PricingTable {
    entries: HashMap<(String, String), ModelPricing>,
}

impl PricingTable {
    pub fn from_configs(configs: &[LlmProviderConfig]) -> Self {
        let mut entries = HashMap::new();
        for c in configs {
            if let Some(pc) = c.pricing() {
                entries.insert((c.kind().to_string(), c.model().to_string()), pc.to_pricing());
            } else if let Some(dp) = default_pricing_for(c.kind(), c.model()) {
                entries.insert((c.kind().to_string(), c.model().to_string()), dp);
            }
        }
        Self { entries }
    }

    pub fn lookup(&self, kind: &str, model: &str) -> Option<&ModelPricing> {
        self.entries.get(&(kind.to_string(), model.to_string()))
    }

    pub fn price_call(&self, usage: &Usage, kind: &str, model: &str) -> i64 {
        let pricing = match self.lookup(kind, model) {
            Some(p) => Some(p.clone()),
            None => default_pricing_for(kind, model),
        };
        price(usage, pricing.as_ref())
    }
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

    fn anthropic_config_with_pricing() -> LlmProviderConfig {
        toml::from_str(r#"
            provider = "anthropic"
            model = "claude-sonnet-4-6"
            api_key = "k"
            pricing = { input_per_1k = 3000, cache_read_per_1k = 300, cache_creation_per_1k = 3750, output_per_1k = 15000, reasoning_per_1k = 15000 }
        "#).unwrap()
    }

    fn gemini_config_no_pricing() -> LlmProviderConfig {
        toml::from_str(r#"
            provider = "gemini"
            model = "gemini-flash"
            api_key = "k"
        "#).unwrap()
    }

    #[test]
    fn pricing_table_builds_from_configs() {
        let configs = vec![anthropic_config_with_pricing(), gemini_config_no_pricing()];
        let table = PricingTable::from_configs(&configs);
        assert!(table.lookup("anthropic", "claude-sonnet-4-6").is_some());
        assert!(table.lookup("gemini", "gemini-flash").is_some(), "gemini-flash should get default pricing");
    }

    #[test]
    fn pricing_table_prices_known_model() {
        let configs = vec![anthropic_config_with_pricing()];
        let table = PricingTable::from_configs(&configs);
        let usage = Usage { input_tokens: 1000, output_tokens: 500, cache_read_tokens: 200, cache_creation_tokens: 100, reasoning_tokens: 50 };
        let cost = table.price_call(&usage, "anthropic", "claude-sonnet-4-6");
        assert!(cost > 0);
        let expected = (1000 * 3000 + 500 * 15000 + 200 * 300 + 100 * 3750 + 50 * 15000) / 1000;
        assert_eq!(cost, expected);
    }

    #[test]
    fn default_pricing_for_gemini_flash() {
        let configs = vec![gemini_config_no_pricing()];
        let table = PricingTable::from_configs(&configs);
        let usage = Usage { input_tokens: 1000, output_tokens: 500, cache_read_tokens: 0, cache_creation_tokens: 0, reasoning_tokens: 0 };
        let cost = table.price_call(&usage, "gemini", "gemini-flash");
        assert!(cost > 0, "gemini-flash should have default pricing");
    }

    #[test]
    fn default_pricing_for_unknown_model() {
        let cfg: LlmProviderConfig = toml::from_str(r#"
            provider = "openai-compatible"
            base_url = "https://example.com"
            model = "some-unknown-model"
            api_key = "k"
        "#).unwrap();
        let table = PricingTable::from_configs(&vec![cfg]);
        let usage = Usage { input_tokens: 1000, output_tokens: 500, cache_read_tokens: 0, cache_creation_tokens: 0, reasoning_tokens: 0 };
        let cost = table.price_call(&usage, "openai-compatible", "some-unknown-model");
        assert!(cost > 0, "unknown models should get fallback pricing");
    }

    #[test]
    fn default_pricing_for_kimi() {
        let cfg: LlmProviderConfig = toml::from_str(r#"
            provider = "openai-compatible"
            base_url = "https://openrouter.ai/api/v1"
            model = "moonshotai/kimi-k3"
            api_key = "k"
        "#).unwrap();
        let table = PricingTable::from_configs(&vec![cfg]);
        let usage = Usage { input_tokens: 1000, output_tokens: 500, cache_read_tokens: 0, cache_creation_tokens: 0, reasoning_tokens: 0 };
        let cost = table.price_call(&usage, "openai-compatible", "moonshotai/kimi-k3");
        assert!(cost > 0, "kimi-k3 should get default pricing");
    }

    #[test]
    fn pricing_table_unknown_model_gets_fallback_pricing() {
        let table = PricingTable::from_configs(&[]);
        let usage = Usage { input_tokens: 1000, output_tokens: 500, cache_read_tokens: 0, cache_creation_tokens: 0, reasoning_tokens: 0 };
        assert!(table.price_call(&usage, "unknown", "unknown-model") > 0);
    }
}
