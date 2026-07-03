pub mod language;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crate::graph::model::ParsedFile;

pub trait LanguageParser: Send + Sync {
    fn language_name(&self) -> &str;
    fn extensions(&self) -> &[&'static str];

    fn parse(&self, source: &str, path: &Path, repo: &str, version: &str) -> ParsedFile;
}

pub struct ParserRegistry {
    by_extension: HashMap<String, Arc<dyn LanguageParser>>,
}

impl ParserRegistry {
    pub fn with_defaults() -> Self {
        use language::*;

        let entries: Vec<Arc<dyn LanguageParser>> = vec![
            Arc::new(RustParser),
            Arc::new(PythonParser),
            Arc::new(TypeScriptParser),
            Arc::new(JavaScriptParser),
            Arc::new(GoParser),
            Arc::new(CParser),
            Arc::new(CppParser),
        ];

        let mut by_extension = HashMap::new();
        for parser in entries {
            for ext in parser.extensions() {
                by_extension.insert(ext.to_string(), Arc::clone(&parser));
            }
        }
        Self { by_extension }
    }

    pub fn get(&self, extension: &str) -> Option<&dyn LanguageParser> {
        self.by_extension.get(extension).map(|p| p.as_ref())
    }
}
