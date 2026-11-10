use serde::{Deserialize, Serialize};

use crate::index::graph;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StepRef {
    Index(usize),
    Name(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Depth {
    Direct,
    #[default]
    Transitive,
}

impl From<Depth> for graph::Depth {
    fn from(depth: Depth) -> Self {
        match depth {
            Depth::Direct => graph::Depth::Direct,
            Depth::Transitive => graph::Depth::Transitive,
        }
    }
}

fn default_budget() -> u32 {
    2_000
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Step {
    Search {
        terms: String,
    },
    SubclassesOf {
        class: String,
        #[serde(default)]
        depth: Depth,
    },
    Capability {
        of: StepRef,
        name: String,
    },
    Method {
        of: StepRef,
        name: String,
    },
    Callers {
        of: StepRef,
    },
    Callees {
        of: StepRef,
    },
    ReadsConfig {
        key: String,
    },
    Read {
        of: StepRef,
        #[serde(default = "default_budget")]
        budget_tokens: u32,
    },
}

pub const STEP_GUIDE: &str = r#"Operations (each step is one JSON object with an "op" field):
- {"op": "search", "terms": "words or identifiers"} finds code by name, text and meaning.
- {"op": "subclasses_of", "class": "ClassName", "depth": "direct" | "transitive"} lists implementations of a class or interface.
- {"op": "capability", "of": REF, "name": "attribute"} resolves a class-level attribute for each class, following inheritance.
- {"op": "method", "of": REF, "name": "method_name"} finds that method on each class, or the ancestor it inherits it from.
- {"op": "callers", "of": REF} and {"op": "callees", "of": REF} follow the call graph.
- {"op": "reads_config", "key": "option_name"} finds code that reads a configuration key.
- {"op": "read", "of": REF, "budget_tokens": 2000} reads the full source of the referenced symbols.
REF is either the 0-based index of an earlier step in the same plan or a symbol name such as "VolumeDriver"."#;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn steps_parse_from_model_json() {
        let steps: Vec<Step> = serde_json::from_value(json!([
            {"op": "subclasses_of", "class": "VolumeDriver"},
            {"op": "capability", "of": 0, "name": "supports_multiattach"},
            {"op": "method", "of": "LVMVolumeDriver", "name": "initialize_connection"},
            {"op": "read", "of": 2}
        ]))
        .unwrap();
        assert_eq!(
            steps[0],
            Step::SubclassesOf {
                class: "VolumeDriver".into(),
                depth: Depth::Transitive
            }
        );
        assert_eq!(
            steps[1],
            Step::Capability {
                of: StepRef::Index(0),
                name: "supports_multiattach".into()
            }
        );
        assert_eq!(
            steps[2],
            Step::Method {
                of: StepRef::Name("LVMVolumeDriver".into()),
                name: "initialize_connection".into()
            }
        );
        assert_eq!(
            steps[3],
            Step::Read {
                of: StepRef::Index(2),
                budget_tokens: 2_000
            }
        );
    }

    #[test]
    fn unknown_operations_are_rejected() {
        assert!(serde_json::from_value::<Step>(json!({"op": "delete_everything"})).is_err());
    }
}
