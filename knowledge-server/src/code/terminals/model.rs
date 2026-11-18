use serde::Serialize;
use uuid::Uuid;

use crate::lxd::console::ConsoleSize;

const MAX_DIMENSION: u16 = 1_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalId(String);

impl TerminalId {
    pub fn generate() -> Self {
        Self(Uuid::new_v4().simple().to_string())
    }

    pub fn parse(raw: &str) -> Option<Self> {
        let is_valid = raw.len() == 32 && raw.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase());
        is_valid.then(|| Self(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalSize {
    pub cols: u16,
    pub rows: u16,
}

impl TerminalSize {
    pub const DEFAULT: Self = Self { cols: 80, rows: 24 };

    pub fn new(cols: u16, rows: u16) -> Option<Self> {
        let fits = (1..=MAX_DIMENSION).contains(&cols) && (1..=MAX_DIMENSION).contains(&rows);
        fits.then_some(Self { cols, rows })
    }
}

impl From<TerminalSize> for ConsoleSize {
    fn from(size: TerminalSize) -> Self {
        Self { width: size.cols, height: size.rows }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Terminal {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub last_attached_at: Option<String>,
}

pub fn environment() -> Vec<(String, String)> {
    [
        ("TERM", "xterm-256color"),
        ("COLORTERM", "truecolor"),
        ("LANG", "C.UTF-8"),
        ("HOME", "/root"),
        ("USER", "root"),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_string(), value.to_string()))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_ids_are_lowercase_hex_of_a_fixed_length() {
        let id = TerminalId::generate();
        assert_eq!(TerminalId::parse(id.as_str()), Some(id));
        for invalid in ["", "abc", "0123456789ABCDEF0123456789abcdef", "../../../../etc/passwd/xxxxxxxxxxx", "0123456789abcdef0123456789abcde;"] {
            assert_eq!(TerminalId::parse(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn sizes_must_be_positive_and_reasonable() {
        assert_eq!(TerminalSize::new(120, 40), Some(TerminalSize { cols: 120, rows: 40 }));
        assert_eq!(TerminalSize::new(0, 40), None);
        assert_eq!(TerminalSize::new(120, 5_000), None);
        assert_eq!(ConsoleSize::from(TerminalSize::DEFAULT), ConsoleSize { width: 80, height: 24 });
    }

    #[test]
    fn shells_get_full_colour_and_utf8() {
        let environment = environment();
        assert!(environment.contains(&("TERM".into(), "xterm-256color".into())));
        assert!(environment.contains(&("COLORTERM".into(), "truecolor".into())));
    }
}
