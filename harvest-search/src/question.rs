use serde::{Deserialize, Serialize};

use crate::index::corpus::Relation;
use crate::index::graph::{Direction, EdgeWeights};
use crate::index::tokenizer::{identifiers, query_tokens, split_identifier, stem, tokens};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionKind {
    Lookup,
    Hierarchy,
    Flow,
    Config,
    Errors,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Question {
    pub kind: QuestionKind,
    pub wants_list: bool,
    #[serde(default)]
    pub about_tests: bool,
    pub identifiers: Vec<String>,
    pub words: Vec<String>,
}

const LIST_CUES: &[&str] = &[
    "which ",
    "all ",
    "every ",
    "each ",
    "list ",
    "enumerate",
    "what are the",
    "how many",
];
const HIERARCHY_CUES: &[&str] = &[
    "implement",
    "subclass",
    "inherit",
    "derive",
    "extends",
    "override",
    "drivers",
    "backends",
    "plugins",
    "providers",
    "base class",
    "concrete",
];
const CONFIG_CUES: &[&str] = &[
    "config",
    "option",
    "setting",
    "flag",
    "environment variable",
    "parameter",
    "yaml",
    "toml",
    ".conf",
    "configured",
];
const ERROR_CUES: &[&str] = &[
    "error",
    "exception",
    "raise",
    "fail",
    "traceback",
    "panic",
    "crash",
];
const FLOW_CUES: &[&str] = &[
    "what happens",
    "how does",
    "how do",
    "flow",
    "call",
    "sequence",
    "trigger",
    "invoked",
    "lifecycle",
    "step by step",
    "when ",
];

const TEST_CUES: &[&str] = &[
    " test", "tests", "testing", " spec", "fixture", "mock", "assert", "coverage",
];

fn mentions(text: &str, cues: &[&str]) -> bool {
    cues.iter().any(|cue| text.contains(cue))
}

impl Question {
    pub fn analyze(text: &str) -> Self {
        let lower = format!("{} ", text.to_lowercase());
        let wants_list = mentions(&lower, LIST_CUES);
        let kind = if mentions(&lower, HIERARCHY_CUES) && wants_list {
            QuestionKind::Hierarchy
        } else if mentions(&lower, CONFIG_CUES) {
            QuestionKind::Config
        } else if mentions(&lower, ERROR_CUES) {
            QuestionKind::Errors
        } else if mentions(&lower, FLOW_CUES) {
            QuestionKind::Flow
        } else if mentions(&lower, HIERARCHY_CUES) {
            QuestionKind::Hierarchy
        } else {
            QuestionKind::Lookup
        };
        let about_tests = mentions(&lower, TEST_CUES);
        Self {
            kind,
            wants_list,
            about_tests,
            identifiers: identifiers(text),
            words: query_tokens(text),
        }
    }

    pub fn vocabulary(&self) -> Vec<String> {
        let mut vocabulary = Vec::new();
        let pieces = self.identifiers.iter().flat_map(|i| {
            crate::index::tokenizer::name_parts(i)
                .iter()
                .flat_map(|p| split_identifier(p))
                .collect::<Vec<_>>()
        });
        for word in self.words.iter().cloned().chain(pieces) {
            for form in [stem(&word), word] {
                if form.len() >= 3 && !vocabulary.contains(&form) {
                    vocabulary.push(form);
                }
            }
        }
        vocabulary
    }

    pub fn relatedness(&self, name: &str) -> usize {
        let target = crate::index::tokenizer::name_key(name);
        if self.identifiers.iter().any(|i| {
            let key = crate::index::tokenizer::name_key(i);
            key == target
                || crate::index::tokenizer::name_parts(i)
                    .last()
                    .is_some_and(|l| *l == target)
        }) {
            return usize::MAX;
        }
        let vocabulary = self.vocabulary();
        let mut shared: Vec<String> = tokens(name)
            .into_iter()
            .map(|t| stem(&t))
            .filter(|t| vocabulary.contains(t))
            .collect();
        shared.sort();
        shared.dedup();
        shared.len()
    }

    pub fn relates_to(&self, name: &str) -> bool {
        let needed = self.vocabulary().len().clamp(1, 2);
        self.relatedness(name) >= needed
    }

    pub fn edge_weights(&self) -> EdgeWeights {
        use Direction::{In, Out};
        use Relation::*;
        let entries: &[(Relation, Direction, f32)] = match self.kind {
            QuestionKind::Lookup => &[
                (Calls, Out, 0.5),
                (Calls, In, 0.3),
                (Contains, Out, 0.6),
                (Contains, In, 0.6),
                (Inherits, Out, 0.4),
                (Implements, Out, 0.4),
                (Uses, Out, 0.3),
                (Documents, In, 0.3),
            ],
            QuestionKind::Hierarchy => &[
                (Inherits, In, 1.0),
                (Implements, In, 1.0),
                (Registers, Out, 0.8),
                (Registers, In, 0.8),
                (Overrides, In, 0.7),
                (Overrides, Out, 0.5),
                (Inherits, Out, 0.5),
                (Contains, Out, 0.4),
            ],
            QuestionKind::Flow => &[
                (Calls, Out, 1.0),
                (Calls, In, 0.5),
                (Overrides, In, 0.8),
                (Overrides, Out, 0.5),
                (Instantiates, Out, 0.7),
                (Contains, Out, 0.3),
                (Contains, In, 0.3),
                (Inherits, Out, 0.3),
            ],
            QuestionKind::Config => &[
                (ReadsConfig, In, 1.0),
                (ReadsConfig, Out, 1.0),
                (Documents, In, 0.5),
                (Calls, In, 0.4),
                (Contains, In, 0.3),
            ],
            QuestionKind::Errors => &[
                (Raises, Out, 1.0),
                (Raises, In, 1.0),
                (Calls, In, 0.5),
                (Calls, Out, 0.5),
                (Contains, In, 0.3),
            ],
        };
        EdgeWeights::new(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questions_are_classified_by_their_cues() {
        let q = Question::analyze("Which volume drivers support multiattach?");
        assert_eq!(q.kind, QuestionKind::Hierarchy);
        assert!(q.wants_list);
        assert_eq!(
            Question::analyze("What happens when a volume is detached?").kind,
            QuestionKind::Flow
        );
        assert_eq!(
            Question::analyze("Where is the retry_limit option read?").kind,
            QuestionKind::Config
        );
        assert_eq!(
            Question::analyze("Why does attach raise VolumeBusy?").kind,
            QuestionKind::Errors
        );
        assert_eq!(
            Question::analyze("Show me VolumeManager").kind,
            QuestionKind::Lookup
        );
    }

    #[test]
    fn vocabulary_includes_identifier_pieces() {
        let q = Question::analyze("What does `initialize_connection` return?");
        assert!(q.vocabulary().contains(&"initialize".to_string()));
        assert!(q.vocabulary().contains(&"connection".to_string()));
    }

    #[test]
    fn relatedness_needs_two_shared_words_or_a_named_identifier() {
        let q = Question::analyze("Which volume drivers support multiattach?");
        assert!(q.relates_to("VolumeDriver"));
        assert!(!q.relates_to("VolumeManager"));
        let named = Question::analyze("What does `retry_limit` control?");
        assert!(named.relates_to("retry_limit"));
    }

    #[test]
    fn hierarchy_questions_walk_down_the_inheritance_tree() {
        let weights = Question::analyze("which classes implement Driver").edge_weights();
        assert!(weights.weight(Relation::Inherits, Direction::In) > 0.9);
        assert_eq!(weights.weight(Relation::Calls, Direction::Out), 0.0);
    }
}
