use std::fmt;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Platform {
    Github,
    Gitlab,
    Gitea,
    Launchpad,
    Gerrit,
}

impl Platform {
    pub const ALL: [Platform; 5] = [
        Platform::Github,
        Platform::Gitlab,
        Platform::Gitea,
        Platform::Launchpad,
        Platform::Gerrit,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::Gitlab => "gitlab",
            Self::Gitea => "gitea",
            Self::Launchpad => "launchpad",
            Self::Gerrit => "gerrit",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Github => "GitHub",
            Self::Gitlab => "GitLab",
            Self::Gitea => "Gitea",
            Self::Launchpad => "Launchpad",
            Self::Gerrit => "OpenDev (Gerrit)",
        }
    }

    pub fn default_host(self) -> Option<&'static str> {
        match self {
            Self::Github => Some("github.com"),
            Self::Gitlab => Some("gitlab.com"),
            Self::Launchpad => Some("launchpad.net"),
            Self::Gerrit => Some("review.opendev.org"),
            Self::Gitea => None,
        }
    }

    pub fn needs_username(self) -> bool {
        matches!(self, Self::Gitea | Self::Gerrit)
    }

    pub fn https_username(self, account_username: &str) -> Option<String> {
        match self {
            Self::Github => Some("x-access-token".into()),
            Self::Gitlab => Some("oauth2".into()),
            Self::Gitea | Self::Gerrit => Some(account_username.to_string()),
            Self::Launchpad => None,
        }
    }

    pub fn for_host(host: &str) -> Option<Self> {
        let host = host.to_ascii_lowercase();
        match host.as_str() {
            "github.com" => Some(Self::Github),
            "gitlab.com" => Some(Self::Gitlab),
            "launchpad.net" | "git.launchpad.net" => Some(Self::Launchpad),
            "opendev.org" | "review.opendev.org" => Some(Self::Gerrit),
            _ => None,
        }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Platform {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|p| p.as_str() == s)
            .ok_or_else(|| format!("unknown platform '{s}'"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_platform_round_trips_through_its_name() {
        for platform in Platform::ALL {
            assert_eq!(platform.as_str().parse::<Platform>(), Ok(platform));
        }
        assert!("svn".parse::<Platform>().is_err());
    }

    #[test]
    fn well_known_hosts_map_to_their_platform() {
        assert_eq!(Platform::for_host("GitHub.com"), Some(Platform::Github));
        assert_eq!(
            Platform::for_host("git.launchpad.net"),
            Some(Platform::Launchpad)
        );
        assert_eq!(Platform::for_host("opendev.org"), Some(Platform::Gerrit));
        assert_eq!(Platform::for_host("git.example.org"), None);
    }
}
