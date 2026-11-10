const STOPWORDS: &[&str] = &[
    "a", "about", "after", "all", "also", "an", "and", "any", "are", "as", "at", "be", "been",
    "before", "but", "by", "can", "could", "did", "do", "does", "doing", "each", "every", "for",
    "from", "get", "gets", "had", "has", "have", "how", "i", "if", "in", "into", "is", "it", "its",
    "list", "me", "my", "of", "on", "or", "our", "should", "show", "so", "some", "such", "tell",
    "than", "that", "the", "their", "them", "then", "there", "these", "they", "this", "those",
    "through", "to", "use", "used", "uses", "using", "via", "was", "we", "were", "what", "when",
    "where", "which", "while", "who", "why", "will", "with", "would", "you", "your",
];

pub fn is_stopword(token: &str) -> bool {
    STOPWORDS.binary_search(&token).is_ok()
}

pub fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for word in words(text) {
        push_word(&word, &mut out);
    }
    out
}

pub fn query_tokens(text: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    tokens(text)
        .into_iter()
        .filter(|t| !is_stopword(t))
        .filter(|t| seen.insert(t.clone()))
        .collect()
}

fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.'))
        .map(|w| w.trim_matches('.'))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
}

fn push_word(word: &str, out: &mut Vec<String>) {
    let whole = word.to_lowercase();
    if word.contains('.') {
        if whole.len() >= 2 {
            out.push(whole);
        }
        for part in word.split('.').filter(|p| !p.is_empty()) {
            push_part(part, out);
        }
    } else {
        push_part(word, out);
    }
}

fn push_part(part: &str, out: &mut Vec<String>) {
    let whole = part.to_lowercase();
    let pieces = split_identifier(part);
    if whole.len() >= 2 {
        out.push(whole.clone());
    }
    if pieces.len() > 1 {
        for piece in pieces {
            if piece.len() >= 2 && piece != whole {
                out.push(piece);
            }
        }
    }
}

pub fn stem(token: &str) -> String {
    if token.len() > 4 && token.ends_with("ies") {
        format!("{}y", &token[..token.len() - 3])
    } else if token.len() > 3 && token.ends_with('s') && !token.ends_with("ss") {
        token[..token.len() - 1].to_string()
    } else {
        token.to_string()
    }
}

pub fn split_identifier(identifier: &str) -> Vec<String> {
    let chars: Vec<char> = identifier.chars().collect();
    let mut pieces = Vec::new();
    let mut current = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c == '_' {
            if !current.is_empty() {
                pieces.push(std::mem::take(&mut current));
            }
            continue;
        }
        let boundary = !current.is_empty() && {
            let prev = chars[i - 1];
            let next = chars.get(i + 1).copied();
            (prev.is_lowercase() && c.is_uppercase())
                || (prev.is_alphabetic() && c.is_ascii_digit())
                || (prev.is_ascii_digit() && c.is_alphabetic())
                || (prev.is_uppercase()
                    && c.is_uppercase()
                    && next.is_some_and(|n| n.is_lowercase()))
        };
        if boundary {
            pieces.push(std::mem::take(&mut current));
        }
        current.push(c);
    }
    if !current.is_empty() {
        pieces.push(current);
    }
    pieces.into_iter().map(|p| p.to_lowercase()).collect()
}

pub fn looks_like_identifier(word: &str) -> bool {
    let trimmed = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
    if trimmed.len() < 3 || !trimmed.starts_with(|c: char| c.is_alphabetic() || c == '_') {
        return false;
    }
    let inner_upper = trimmed.chars().skip(1).any(char::is_uppercase);
    let has_lower = trimmed.chars().any(char::is_lowercase);
    trimmed.contains('_')
        || word.contains('.') && !word.ends_with('.')
        || (inner_upper && has_lower)
        || word.contains("()")
}

pub fn identifiers(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for raw in text.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                ',' | ';' | '?' | '!' | '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}'
            )
    }) {
        let quoted = raw.starts_with('`') || raw.ends_with('`');
        let word = raw.trim_matches(|c: char| c == '`' || c == ':' || c == '.');
        if word.is_empty() {
            continue;
        }
        if (quoted || looks_like_identifier(raw.trim_matches('`')))
            && !found.contains(&word.to_string())
        {
            found.push(word.to_string());
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopwords_are_sorted_for_binary_search() {
        let mut sorted = STOPWORDS.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, STOPWORDS);
    }

    #[test]
    fn camel_and_snake_case_keep_whole_tokens_and_pieces() {
        assert_eq!(
            tokens("VolumeDriver"),
            vec!["volumedriver", "volume", "driver"]
        );
        assert_eq!(
            tokens("supports_multiattach"),
            vec!["supports_multiattach", "supports", "multiattach"]
        );
        assert_eq!(
            split_identifier("HTTPServerError"),
            vec!["http", "server", "error"]
        );
        assert_eq!(split_identifier("v2Api"), vec!["v", "2", "api"]);
    }

    #[test]
    fn dotted_paths_are_searchable_whole_and_in_parts() {
        let t = tokens("cinder.volume.driver");
        assert_eq!(t[0], "cinder.volume.driver");
        assert!(t.contains(&"volume".to_string()));
        assert!(t.contains(&"driver".to_string()));
    }

    #[test]
    fn query_tokens_drop_stopwords_and_duplicates() {
        assert_eq!(
            query_tokens("Which drivers support the multiattach option, and which do not?"),
            vec!["drivers", "support", "multiattach", "option", "not"]
        );
    }

    #[test]
    fn stems_fold_simple_plurals() {
        assert_eq!(stem("drivers"), "driver");
        assert_eq!(stem("policies"), "policy");
        assert_eq!(stem("class"), "class");
        assert_eq!(stem("is"), "is");
    }

    #[test]
    fn identifiers_are_found_in_prose() {
        assert_eq!(
            identifiers("Where is `retry_limit` read, and what does VolumeDriver.initialize_connection() do?"),
            vec!["retry_limit", "VolumeDriver.initialize_connection"]
        );
        assert!(identifiers("How does the scheduler work?").is_empty());
    }
}
