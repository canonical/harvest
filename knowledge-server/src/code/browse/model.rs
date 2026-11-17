use serde::Serialize;

pub const MAX_TEXT_BYTES: u64 = 1024 * 1024;
pub const MAX_RAW_BYTES: u64 = 20 * 1024 * 1024;
const BINARY_SNIFF_BYTES: usize = 8000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoPath(String);

impl RepoPath {
    pub fn root() -> Self {
        Self(String::new())
    }

    pub fn parse(raw: &str) -> Option<Self> {
        let path = raw.trim_matches('/');
        let segments_are_valid = path
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..");
        let is_valid = path.is_empty() || (segments_are_valid && !path.contains('\0'));
        is_valid.then(|| Self(path.to_string()))
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn child(&self, name: &str) -> String {
        if self.is_root() {
            name.to_string()
        } else {
            format!("{}/{name}", self.0)
        }
    }

    pub fn at(&self, commit: &str) -> String {
        format!("{commit}:{}", self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    Directory,
    Submodule,
    File,
    Symlink,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TreeEntry {
    pub name: String,
    pub path: String,
    pub kind: EntryKind,
}

const SYMLINK_MODE: &str = "120000";

pub fn parse_tree(listing: &[u8], directory: &RepoPath) -> Vec<TreeEntry> {
    let mut entries: Vec<TreeEntry> = listing
        .split(|byte| *byte == 0)
        .filter_map(|record| parse_tree_record(&String::from_utf8_lossy(record), directory))
        .collect();
    entries.sort_by(|a, b| directories_first(a, b).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    entries
}

fn parse_tree_record(record: &str, directory: &RepoPath) -> Option<TreeEntry> {
    let (header, name) = record.split_once('\t')?;
    let mut fields = header.split(' ');
    let mode = fields.next()?;
    let object_type = fields.next()?;
    let kind = match (object_type, mode) {
        ("tree", _) => EntryKind::Directory,
        ("commit", _) => EntryKind::Submodule,
        ("blob", SYMLINK_MODE) => EntryKind::Symlink,
        ("blob", _) => EntryKind::File,
        _ => return None,
    };
    Some(TreeEntry {
        name: name.to_string(),
        path: directory.child(name),
        kind,
    })
}

fn directories_first(a: &TreeEntry, b: &TreeEntry) -> std::cmp::Ordering {
    let is_file = |entry: &TreeEntry| entry.kind >= EntryKind::File;
    is_file(a).cmp(&is_file(b))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FileContent {
    Text { content: String },
    Binary,
    TooLarge,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FileView {
    pub path: String,
    pub size: u64,
    #[serde(flatten)]
    pub content: FileContent,
}

pub fn is_too_large_to_show(size: u64) -> bool {
    size > MAX_TEXT_BYTES
}

pub fn classify(bytes: &[u8]) -> FileContent {
    let sniffed = &bytes[..bytes.len().min(BINARY_SNIFF_BYTES)];
    if sniffed.contains(&0) {
        FileContent::Binary
    } else {
        FileContent::Text {
            content: String::from_utf8_lossy(bytes).into_owned(),
        }
    }
}

pub fn image_type(path: &str) -> Option<&'static str> {
    let extension = path.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "svg" => "image/svg+xml",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(raw: &str) -> RepoPath {
        RepoPath::parse(raw).unwrap()
    }

    #[test]
    fn paths_stay_inside_the_repository() {
        assert_eq!(path("src/main.rs").as_str(), "src/main.rs");
        assert_eq!(path("/src/").as_str(), "src");
        assert!(path("").is_root());
        for escaping in ["../etc/passwd", "src/../../x", "a//b", "./a", "a/\0b"] {
            assert_eq!(RepoPath::parse(escaping), None, "{escaping}");
        }
    }

    #[test]
    fn paths_name_children_and_objects() {
        assert_eq!(path("").child("README.md"), "README.md");
        assert_eq!(path("src").child("lib.rs"), "src/lib.rs");
        assert_eq!(path("src").at("abc123"), "abc123:src");
        assert_eq!(path("").at("abc123"), "abc123:");
    }

    #[test]
    fn trees_list_directories_first_then_names_without_regard_to_case() {
        let listing = [
            "100644 blob 1111\tzeta.txt",
            "040000 tree 2222\tsrc",
            "120000 blob 3333\tlink",
            "160000 commit 4444\tvendor",
            "100644 blob 5555\tAlpha.md",
            "040000 tree 6666\tdocs",
        ]
        .join("\0");
        let entries = parse_tree(listing.as_bytes(), &path("pkg"));
        let summary: Vec<(&str, EntryKind)> = entries.iter().map(|e| (e.path.as_str(), e.kind)).collect();
        assert_eq!(
            summary,
            [
                ("pkg/docs", EntryKind::Directory),
                ("pkg/src", EntryKind::Directory),
                ("pkg/vendor", EntryKind::Submodule),
                ("pkg/Alpha.md", EntryKind::File),
                ("pkg/link", EntryKind::Symlink),
                ("pkg/zeta.txt", EntryKind::File),
            ]
        );
    }

    #[test]
    fn tree_names_keep_spaces_and_skip_malformed_records() {
        let entries = parse_tree(b"100644 blob 1\tmy notes.txt\0garbage\0", &path(""));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "my notes.txt");
    }

    #[test]
    fn content_with_a_nul_byte_is_binary() {
        assert_eq!(classify(b"\x89PNG\r\n\x1a\n\0\0"), FileContent::Binary);
        assert_eq!(
            classify("fn main() {}\n".as_bytes()),
            FileContent::Text { content: "fn main() {}\n".into() }
        );
    }

    #[test]
    fn only_files_over_the_limit_are_too_large_to_show() {
        assert!(!is_too_large_to_show(MAX_TEXT_BYTES));
        assert!(is_too_large_to_show(MAX_TEXT_BYTES + 1));
    }

    #[test]
    fn file_views_serialise_their_kind_beside_the_path() {
        let view = FileView { path: "a.bin".into(), size: 3, content: FileContent::Binary };
        assert_eq!(
            serde_json::to_value(view).unwrap(),
            serde_json::json!({ "path": "a.bin", "size": 3, "kind": "binary" })
        );
    }

    #[test]
    fn images_are_recognised_by_extension() {
        assert_eq!(image_type("docs/Logo.PNG"), Some("image/png"));
        assert_eq!(image_type("icon.svg"), Some("image/svg+xml"));
        assert_eq!(image_type("main.rs"), None);
        assert_eq!(image_type("Makefile"), None);
    }
}
