use std::io::{BufRead, Write};
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

fn essential_by_default() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GoldEvidence {
    pub file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    #[serde(default = "essential_by_default")]
    pub essential: bool,
}

impl GoldEvidence {
    pub fn symbol(file: &str, name: &str, lines: (u32, u32), essential: bool) -> Self {
        Self {
            file: file.into(),
            name: Some(name.into()),
            start_line: Some(lines.0),
            end_line: Some(lines.1),
            essential,
        }
    }

    pub fn span(file: &str, lines: (u32, u32), essential: bool) -> Self {
        Self {
            file: file.into(),
            name: None,
            start_line: Some(lines.0),
            end_line: Some(lines.1),
            essential,
        }
    }

    pub fn file(file: &str, essential: bool) -> Self {
        Self {
            file: file.into(),
            name: None,
            start_line: None,
            end_line: None,
            essential,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GoldQuery {
    pub id: String,
    pub question: String,
    pub repo: String,
    pub version: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub source: String,
    pub evidence: Vec<GoldEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_answer: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Split {
    Train,
    Dev,
    Heldout,
    All,
}

fn fnv(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

pub fn split_of(id: &str) -> Split {
    match fnv(id) % 10 {
        0..=2 => Split::Heldout,
        3..=5 => Split::Dev,
        _ => Split::Train,
    }
}

impl GoldQuery {
    pub fn in_split(&self, split: Split) -> bool {
        split == Split::All || split_of(&self.id) == split
    }

    pub fn essential(&self) -> impl Iterator<Item = &GoldEvidence> {
        self.evidence.iter().filter(|e| e.essential)
    }
}

pub fn load(path: &Path) -> Result<Vec<GoldQuery>> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut queries = Vec::new();
    for (number, line) in std::io::BufReader::new(file).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        queries.push(
            serde_json::from_str(&line)
                .with_context(|| format!("{}:{}", path.display(), number + 1))?,
        );
    }
    Ok(queries)
}

pub fn save(path: &Path, queries: &[GoldQuery]) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::File::create(path)?;
    for query in queries {
        writeln!(file, "{}", serde_json::to_string(query)?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_are_stable_and_roughly_proportional() {
        assert_eq!(split_of("q-1"), split_of("q-1"));
        let mut counts = [0usize; 3];
        for i in 0..3000 {
            match split_of(&format!("q-{i}")) {
                Split::Heldout => counts[0] += 1,
                Split::Dev => counts[1] += 1,
                Split::Train => counts[2] += 1,
                Split::All => unreachable!(),
            }
        }
        assert!((800..1000).contains(&counts[0]), "{counts:?}");
        assert!((800..1000).contains(&counts[1]), "{counts:?}");
        assert!((1100..1300).contains(&counts[2]), "{counts:?}");
    }

    #[test]
    fn gold_sets_round_trip_and_default_to_essential() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("g/gold.jsonl");
        let query = GoldQuery {
            id: "a".into(),
            question: "q".into(),
            repo: "r".into(),
            version: "v".into(),
            kind: "lookup".into(),
            source: "synthetic".into(),
            evidence: vec![GoldEvidence::symbol("a.py", "f", (1, 2), true)],
            reference_answer: None,
        };
        save(&path, std::slice::from_ref(&query)).unwrap();
        assert_eq!(load(&path).unwrap(), vec![query]);
        let parsed: GoldEvidence = serde_json::from_str(r#"{"file": "x.py"}"#).unwrap();
        assert!(parsed.essential);
    }
}
