use crate::config::CodeConfig;

const ID_LENGTH: usize = 32;
const LABEL_PREFIX: &str = "p-";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewOrigin {
    scheme: String,
    domain: String,
}

impl PreviewOrigin {
    pub fn from_config(config: &CodeConfig) -> Option<Self> {
        let domain = config
            .preview_domain
            .as_deref()
            .map(|d| d.trim().trim_matches('.').to_ascii_lowercase())
            .filter(|d| !d.is_empty())?;
        Some(Self {
            scheme: config.preview_scheme.trim().to_ascii_lowercase(),
            domain,
        })
    }

    pub fn is_secure(&self) -> bool {
        self.scheme == "https"
    }

    pub fn url_for(&self, preview_id: &str, path: &str) -> String {
        format!("{}://{LABEL_PREFIX}{preview_id}.{}{path}", self.scheme, self.domain)
    }

    pub fn id_from_host(&self, host: &str) -> Option<String> {
        let host = host.to_ascii_lowercase();
        let name = host.rsplit_once(':').filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit()));
        let name = name.map_or(host.as_str(), |(name, _)| name);
        let id = name
            .strip_suffix(&self.domain)?
            .strip_suffix('.')?
            .strip_prefix(LABEL_PREFIX)?;
        let is_id = id.len() == ID_LENGTH && id.chars().all(|c| c.is_ascii_hexdigit());
        is_id.then(|| id.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0123456789abcdef0123456789abcdef";

    fn origin(domain: Option<&str>, scheme: &str) -> Option<PreviewOrigin> {
        PreviewOrigin::from_config(&CodeConfig {
            preview_domain: domain.map(String::from),
            preview_scheme: scheme.into(),
            ..CodeConfig::default()
        })
    }

    #[test]
    fn previews_are_off_without_a_domain() {
        assert_eq!(origin(None, "https"), None);
        assert_eq!(origin(Some("  "), "https"), None);
    }

    #[test]
    fn each_preview_gets_its_own_host() {
        let origin = origin(Some("Preview.Example.com."), "https").unwrap();
        assert_eq!(origin.url_for(ID, "/"), format!("https://p-{ID}.preview.example.com/"));
        assert!(origin.is_secure());
    }

    #[test]
    fn hosts_are_matched_strictly() {
        let origin = origin(Some("preview.example.com"), "http").unwrap();
        assert_eq!(origin.id_from_host(&format!("p-{ID}.preview.example.com")).as_deref(), Some(ID));
        assert_eq!(origin.id_from_host(&format!("P-{}.PREVIEW.example.com:8080", ID.to_uppercase())).as_deref(), Some(ID));
        assert_eq!(origin.id_from_host(&format!("{ID}.preview.example.com")), None);
        assert_eq!(origin.id_from_host("preview.example.com"), None);
        assert_eq!(origin.id_from_host(&format!("p-{ID}.preview.example.com.evil.org")), None);
        assert_eq!(origin.id_from_host(&format!("p-{ID}xpreview.example.com")), None);
        assert_eq!(origin.id_from_host(&format!("a.p-{ID}.preview.example.com")), None);
        assert_eq!(origin.id_from_host("p-not-an-id.preview.example.com"), None);
        assert_eq!(origin.id_from_host("harvest.example.com"), None);
    }
}
