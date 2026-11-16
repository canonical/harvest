#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Ssh,
    Https,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteUrl {
    pub transport: Transport,
    pub host: String,
    pub path: String,
}

impl RemoteUrl {
    pub fn parse(url: &str) -> Option<Self> {
        let url = url.trim();
        if let Some(rest) = url
            .strip_prefix("https://")
            .or_else(|| url.strip_prefix("http://"))
        {
            return Self::from_authority_path(Transport::Https, rest);
        }
        if let Some(rest) = url
            .strip_prefix("ssh://")
            .or_else(|| url.strip_prefix("git+ssh://"))
        {
            return Self::from_authority_path(Transport::Ssh, rest);
        }
        if let Some(rest) = url.strip_prefix("git://") {
            return Self::from_authority_path(Transport::Https, rest);
        }
        let (authority, path) = url.split_once(':')?;
        if authority.contains('/') || path.starts_with("//") {
            return None;
        }
        let host = authority.rsplit('@').next()?;
        Some(Self {
            transport: Transport::Ssh,
            host: host.to_ascii_lowercase(),
            path: path.trim_matches('/').trim_end_matches(".git").to_string(),
        })
    }

    fn from_authority_path(transport: Transport, rest: &str) -> Option<Self> {
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = authority.rsplit('@').next()?.split(':').next()?;
        if host.is_empty() {
            return None;
        }
        Some(Self {
            transport,
            host: host.to_ascii_lowercase(),
            path: path.trim_matches('/').trim_end_matches(".git").to_string(),
        })
    }

    pub fn name(&self) -> String {
        self.path
            .rsplit('/')
            .next()
            .unwrap_or(&self.path)
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_and_ssh_forms_resolve_to_the_same_host_and_path() {
        let https = RemoteUrl::parse("https://github.com/canonical/amafu.git").unwrap();
        let scp = RemoteUrl::parse("git@github.com:canonical/amafu.git").unwrap();
        let ssh = RemoteUrl::parse("ssh://git@github.com:22/canonical/amafu").unwrap();
        assert_eq!(https.transport, Transport::Https);
        assert_eq!(scp.transport, Transport::Ssh);
        assert_eq!(ssh.transport, Transport::Ssh);
        for remote in [&https, &scp, &ssh] {
            assert_eq!(remote.host, "github.com");
            assert_eq!(remote.path, "canonical/amafu");
        }
        assert_eq!(https.name(), "amafu");
    }

    #[test]
    fn launchpad_and_opendev_urls_are_understood() {
        let lp = RemoteUrl::parse("git+ssh://user@git.launchpad.net/~team/project").unwrap();
        assert_eq!(
            (lp.transport, lp.host.as_str()),
            (Transport::Ssh, "git.launchpad.net")
        );
        let od = RemoteUrl::parse("https://opendev.org/openstack/cinder").unwrap();
        assert_eq!(od.name(), "cinder");
    }

    #[test]
    fn plain_paths_are_not_remotes() {
        assert_eq!(RemoteUrl::parse("/srv/repos/x"), None);
        assert_eq!(RemoteUrl::parse("https://"), None);
    }
}
