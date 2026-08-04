use serde_json::Value;
use std::collections::BTreeMap;

pub type FileBundle = BTreeMap<String, String>;

const MAX_FILES: usize = 200;
const MAX_TOTAL_BYTES: usize = 2 * 1024 * 1024;

pub fn parse_bundle(content: &str) -> Result<FileBundle, String> {
    let value: Value = serde_json::from_str(content)
        .map_err(|_| "content must be valid JSON".to_string())?;
    let object = value.as_object()
        .ok_or_else(|| "content must be a JSON object mapping file paths to file text".to_string())?;

    let mut bundle = FileBundle::new();
    for (path, text) in object {
        let text = text.as_str()
            .ok_or_else(|| format!("file '{path}' must have a string value"))?;
        bundle.insert(path.clone(), text.to_string());
    }
    Ok(bundle)
}

fn is_safe_relative_path(path: &str) -> bool {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') {
        return false;
    }
    path.split('/').all(|segment| !segment.is_empty() && segment != "..")
}

pub fn validate_bundle(bundle: &FileBundle) -> Result<(), String> {
    if bundle.is_empty() {
        return Err("bundle must contain at least one file".to_string());
    }
    if bundle.len() > MAX_FILES {
        return Err(format!("bundle exceeds the maximum of {MAX_FILES} files"));
    }

    let mut total_bytes = 0usize;
    for (path, text) in bundle {
        if !is_safe_relative_path(path) {
            return Err(format!("unsafe file path: '{path}'"));
        }
        total_bytes += text.len();
    }
    if total_bytes > MAX_TOTAL_BYTES {
        return Err(format!("bundle exceeds the maximum total size of {MAX_TOTAL_BYTES} bytes"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bundle_rejects_invalid_json() {
        assert!(parse_bundle("not json").is_err());
    }

    #[test]
    fn parse_bundle_rejects_non_object_json() {
        assert!(parse_bundle("[1,2,3]").is_err());
    }

    #[test]
    fn parse_bundle_rejects_non_string_values() {
        assert!(parse_bundle(r#"{"main.tf": 123}"#).is_err());
    }

    #[test]
    fn parse_bundle_accepts_valid_map() {
        let bundle = parse_bundle(r#"{"main.tf": "resource \"local_file\" \"x\" {}"}"#).unwrap();
        assert_eq!(bundle.get("main.tf").unwrap(), "resource \"local_file\" \"x\" {}");
    }

    #[test]
    fn validate_bundle_rejects_empty() {
        assert!(validate_bundle(&FileBundle::new()).is_err());
    }

    #[test]
    fn validate_bundle_rejects_path_traversal() {
        let mut bundle = FileBundle::new();
        bundle.insert("../etc/passwd".into(), "x".into());
        assert!(validate_bundle(&bundle).is_err());
    }

    #[test]
    fn validate_bundle_rejects_nested_path_traversal() {
        let mut bundle = FileBundle::new();
        bundle.insert("modules/../../etc/passwd".into(), "x".into());
        assert!(validate_bundle(&bundle).is_err());
    }

    #[test]
    fn validate_bundle_rejects_absolute_path() {
        let mut bundle = FileBundle::new();
        bundle.insert("/etc/passwd".into(), "x".into());
        assert!(validate_bundle(&bundle).is_err());
    }

    #[test]
    fn validate_bundle_rejects_backslash_path() {
        let mut bundle = FileBundle::new();
        bundle.insert("modules\\evil".into(), "x".into());
        assert!(validate_bundle(&bundle).is_err());
    }

    #[test]
    fn validate_bundle_rejects_too_many_files() {
        let mut bundle = FileBundle::new();
        for i in 0..(MAX_FILES + 1) {
            bundle.insert(format!("file{i}.tf"), "x".into());
        }
        assert!(validate_bundle(&bundle).is_err());
    }

    #[test]
    fn validate_bundle_rejects_oversized_total() {
        let mut bundle = FileBundle::new();
        bundle.insert("main.tf".into(), "x".repeat(MAX_TOTAL_BYTES + 1));
        assert!(validate_bundle(&bundle).is_err());
    }

    #[test]
    fn validate_bundle_accepts_nested_module_layout() {
        let mut bundle = FileBundle::new();
        bundle.insert("main.tf".into(), "...".into());
        bundle.insert("modules/network/main.tf".into(), "...".into());
        assert!(validate_bundle(&bundle).is_ok());
    }
}
