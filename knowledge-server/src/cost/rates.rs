use crate::llm::pricing::ModelPricing;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rate {
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
}

impl Rate {
    const fn new(input: i64, output: i64, cache_read: i64, cache_write: i64) -> Self {
        Self {
            input,
            output,
            cache_read,
            cache_write,
        }
    }

    fn pricing(self) -> ModelPricing {
        ModelPricing {
            input_per_1k: self.input,
            output_per_1k: self.output,
            cache_read_per_1k: self.cache_read,
            cache_creation_per_1k: self.cache_write,
            reasoning_per_1k: self.output,
        }
    }
}

const FABLE_5_2: Rate = Rate::new(8_000, 40_000, 200, 10_000);
const FABLE_5_1: Rate = Rate::new(10_000, 50_000, 250, 12_500);
const FABLE_5: Rate = Rate::new(10_000, 50_000, 1_000, 12_500);
const OPUS_5_5: Rate = Rate::new(4_000, 20_000, 200, 5_000);
const OPUS: Rate = Rate::new(5_000, 25_000, 500, 6_250);
const SONNET_5_5: Rate = Rate::new(2_000, 10_000, 100, 2_500);
const SONNET_5: Rate = Rate::new(2_000, 10_000, 200, 2_500);
const SONNET_4: Rate = Rate::new(3_000, 15_000, 300, 3_750);
const HAIKU_5_5: Rate = Rate::new(100, 500, 10, 125);
const HAIKU_4_5: Rate = Rate::new(1_000, 5_000, 100, 1_250);

const CLAUDE_MODELS: &[(&str, Rate)] = &[
    ("claude-fable-5-2", FABLE_5_2),
    ("claude-mythos-5-2", FABLE_5_2),
    ("claude-fable-5-1", FABLE_5_1),
    ("claude-mythos-5-1", FABLE_5_1),
    ("claude-fable-5", FABLE_5),
    ("claude-mythos-5", FABLE_5),
    ("claude-opus-5-5", OPUS_5_5),
    ("claude-opus-5", OPUS),
    ("claude-opus-4-8", OPUS),
    ("claude-opus-4-7", OPUS),
    ("claude-opus-4-6", OPUS),
    ("claude-sonnet-5-5", SONNET_5_5),
    ("claude-sonnet-5", SONNET_5),
    ("claude-sonnet-4", SONNET_4),
    ("claude-haiku-5-5", HAIKU_5_5),
    ("claude-haiku-4-5", HAIKU_4_5),
];

const CLAUDE_FAMILIES: &[(&str, Rate)] = &[
    ("fable", FABLE_5_2),
    ("mythos", FABLE_5_2),
    ("opus", OPUS),
    ("haiku", HAIKU_4_5),
];

const CLAUDE_FALLBACK: Rate = SONNET_4;

struct VendorRule {
    vendor: &'static [&'static str],
    model: &'static [&'static str],
    rate: Rate,
}

const VENDOR_RULES: &[VendorRule] = &[
    VendorRule { vendor: &["gemini"], model: &["2-5-pro", "gemini-3"], rate: Rate::new(1_250, 5_000, 312, 0) },
    VendorRule { vendor: &["gemini"], model: &["flash"], rate: Rate::new(300, 2_500, 75, 0) },
    VendorRule { vendor: &["gemini"], model: &[], rate: Rate::new(1_250, 5_000, 312, 0) },
    VendorRule { vendor: &["openai", "gpt"], model: &["gpt-4o"], rate: Rate::new(2_500, 10_000, 1_250, 0) },
    VendorRule { vendor: &["openai", "gpt"], model: &["gpt-4"], rate: Rate::new(30_000, 60_000, 1_500, 0) },
    VendorRule { vendor: &["openai", "gpt", "o1", "o3", "o4"], model: &["o1", "o3", "o4"], rate: Rate::new(15_000, 60_000, 7_500, 0) },
    VendorRule { vendor: &["kimi", "k3"], model: &[], rate: Rate::new(600, 2_200, 60, 0) },
    VendorRule { vendor: &["deepseek"], model: &[], rate: Rate::new(270, 1_100, 27, 0) },
    VendorRule { vendor: &["llama", "mistral", "ministral"], model: &[], rate: Rate::new(200, 600, 20, 0) },
    VendorRule { vendor: &["jev", "kev", "systemone", "system-one"], model: &[], rate: Rate::new(42, 0, 0, 0) },
];

const UNKNOWN_MODEL: Rate = Rate::new(1_000, 3_000, 100, 0);

pub fn default_pricing_for(kind: &str, model: &str) -> Option<ModelPricing> {
    let id = normalized_model_id(model);
    let kind = kind.to_lowercase();
    let rate = if kind.contains("anthropic") || id.starts_with("claude") {
        claude_rate(&id)
    } else {
        vendor_rate(&kind, &id)
    };
    Some(rate.pricing())
}

fn normalized_model_id(model: &str) -> String {
    let lower = model.to_lowercase();
    let after_vendor = lower.rsplit('/').next().unwrap_or(&lower);
    let bare = after_vendor.strip_prefix("anthropic.").unwrap_or(after_vendor);
    bare.replace('.', "-")
}

fn claude_rate(id: &str) -> Rate {
    CLAUDE_MODELS
        .iter()
        .find(|(prefix, _)| id.starts_with(prefix))
        .or_else(|| CLAUDE_FAMILIES.iter().find(|(family, _)| id.contains(family)))
        .map(|(_, rate)| *rate)
        .unwrap_or(CLAUDE_FALLBACK)
}

fn vendor_rate(kind: &str, id: &str) -> Rate {
    VENDOR_RULES
        .iter()
        .find(|rule| rule.applies_to(kind, id))
        .map(|rule| rule.rate)
        .unwrap_or(UNKNOWN_MODEL)
}

impl VendorRule {
    fn applies_to(&self, kind: &str, id: &str) -> bool {
        let vendor = self.vendor.iter().any(|v| kind.contains(v) || id.contains(v));
        let model = self.model.is_empty() || self.model.iter().any(|m| id.contains(m));
        vendor && model
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rate_of(kind: &str, model: &str) -> (i64, i64, i64, i64) {
        let p = default_pricing_for(kind, model).unwrap();
        (p.input_per_1k, p.output_per_1k, p.cache_read_per_1k, p.cache_creation_per_1k)
    }

    #[test]
    fn current_claude_models_have_their_list_prices() {
        assert_eq!(rate_of("anthropic", "claude-fable-5-2"), (8_000, 40_000, 200, 10_000));
        assert_eq!(rate_of("anthropic", "claude-opus-5-5"), (4_000, 20_000, 200, 5_000));
        assert_eq!(rate_of("anthropic", "claude-sonnet-5-5"), (2_000, 10_000, 100, 2_500));
        assert_eq!(rate_of("anthropic", "claude-haiku-5-5"), (100, 500, 10, 125));
    }

    #[test]
    fn longer_model_ids_win_over_their_prefixes() {
        assert_eq!(rate_of("anthropic", "claude-opus-5"), (5_000, 25_000, 500, 6_250));
        assert_eq!(rate_of("anthropic", "claude-sonnet-5"), (2_000, 10_000, 200, 2_500));
        assert_eq!(rate_of("anthropic", "claude-fable-5"), (10_000, 50_000, 1_000, 12_500));
    }

    #[test]
    fn router_and_cloud_model_ids_resolve_to_the_same_model() {
        let first_party = rate_of("anthropic", "claude-sonnet-5-5");
        assert_eq!(rate_of("openai-compatible", "anthropic/claude-sonnet-5.5"), first_party);
        assert_eq!(rate_of("anthropic", "anthropic.claude-sonnet-5-5"), first_party);
        assert_eq!(rate_of("openai-compatible", "Anthropic/Claude-Sonnet-5.5"), first_party);
    }

    #[test]
    fn older_claude_models_fall_back_to_their_family() {
        assert_eq!(rate_of("anthropic", "claude-sonnet-4-6"), (3_000, 15_000, 300, 3_750));
        assert_eq!(rate_of("anthropic", "claude-opus-4-1"), (5_000, 25_000, 500, 6_250));
        assert_eq!(rate_of("anthropic", "claude-3-haiku"), (1_000, 5_000, 100, 1_250));
    }

    #[test]
    fn reasoning_is_priced_as_output() {
        for model in ["claude-opus-5-5", "gemini-2.5-flash", "o3-mini", "kimi-k3"] {
            let p = default_pricing_for("openai-compatible", model).unwrap();
            assert_eq!(p.reasoning_per_1k, p.output_per_1k, "{model}");
        }
    }

    #[test]
    fn other_vendors_keep_their_rates() {
        assert_eq!(rate_of("gemini", "gemini-2.5-flash"), (300, 2_500, 75, 0));
        assert_eq!(rate_of("gemini", "gemini-2.5-pro"), (1_250, 5_000, 312, 0));
        assert_eq!(rate_of("openai-compatible", "openai/gpt-4o-mini"), (2_500, 10_000, 1_250, 0));
        assert_eq!(rate_of("openai-compatible", "deepseek-chat"), (270, 1_100, 27, 0));
        assert_eq!(rate_of("openai-compatible", "something-new"), (1_000, 3_000, 100, 0));
    }
}
