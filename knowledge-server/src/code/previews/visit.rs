use anyhow::Result;

use super::super::CodeEnv;
use super::clones::{self, Target};
use super::{may_view, Audience, Preview, Viewer};
use crate::access::repos::RepoAccess;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Identity {
    User(String),
    Anonymous(String),
}

impl Identity {
    pub fn parse(key: &str) -> Option<Self> {
        match key.split_once(':')? {
            ("user", id) if !id.is_empty() => Some(Self::User(id.to_string())),
            ("anon", token) if !token.is_empty() => Some(Self::Anonymous(token.to_string())),
            _ => None,
        }
    }
}

pub struct Visit {
    pub env: CodeEnv,
    pub preview: Preview,
    pub viewer: Viewer,
}

impl Visit {
    pub async fn open(env: &CodeEnv, preview_id: &str, identity: Identity) -> Result<Option<Self>> {
        let Some(preview) = super::get(&env.db, preview_id).await? else {
            return Ok(None);
        };
        let viewer = viewer_for(env, &preview, identity).await;
        if !may_view(&preview, &viewer) {
            return Ok(None);
        }
        Ok(Some(Self { env: env.clone(), preview, viewer }))
    }

    pub async fn target(&self) -> Result<Target> {
        clones::target_for(&self.env, &self.preview, &self.viewer).await
    }

    pub fn is_owner(&self) -> bool {
        self.viewer.is_user(&self.preview.owner_id)
    }
}

async fn viewer_for(env: &CodeEnv, preview: &Preview, identity: Identity) -> Viewer {
    match identity {
        Identity::Anonymous(token) => Viewer::Anonymous(token),
        Identity::User(id) => {
            let needs_access_check = preview.audience == Audience::Repository && id != preview.owner_id;
            let repo_allowed = needs_access_check && repository_allows(env, &id, &preview.project_name).await;
            Viewer::User { id, repo_allowed }
        }
    }
}

async fn repository_allows(env: &CodeEnv, user_id: &str, repo: &str) -> bool {
    RepoAccess::new(&env.db)
        .scope_for(user_id)
        .await
        .map(|scope| scope.allows(repo))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_round_trip_through_viewer_keys() {
        let user = Viewer::User { id: "u1".into(), repo_allowed: false };
        assert_eq!(Identity::parse(&user.key()), Some(Identity::User("u1".into())));
        let anonymous = Viewer::Anonymous("abc".into());
        assert_eq!(Identity::parse(&anonymous.key()), Some(Identity::Anonymous("abc".into())));
    }

    #[test]
    fn malformed_keys_are_rejected() {
        for key in ["", "user:", "anon:", "admin:x", "nocolon"] {
            assert_eq!(Identity::parse(key), None, "{key}");
        }
    }
}
