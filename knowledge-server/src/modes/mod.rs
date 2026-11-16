use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    Chat,
    Code,
    Deploy,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Space {
    Personal { user_id: String },
    DeployProject { project_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Owner {
    User(String),
    Project(String),
}

impl Owner {
    pub fn user_id(&self) -> Option<&str> {
        match self {
            Self::User(id) => Some(id),
            Self::Project(_) => None,
        }
    }

    pub fn project_id(&self) -> Option<&str> {
        match self {
            Self::Project(id) => Some(id),
            Self::User(_) => None,
        }
    }
}

const PERSONAL: &str = "personal";
const DEPLOY: &str = "deploy";

impl Space {
    pub fn personal(user_id: impl Into<String>) -> Self {
        Self::Personal {
            user_id: user_id.into(),
        }
    }

    pub fn deploy_project(project_id: impl Into<String>) -> Self {
        Self::DeployProject {
            project_id: project_id.into(),
        }
    }

    pub fn mode(&self) -> Mode {
        match self {
            Self::Personal { .. } => Mode::Chat,
            Self::DeployProject { .. } => Mode::Deploy,
        }
    }

    pub fn owner(&self) -> Owner {
        match self {
            Self::Personal { user_id } => Owner::User(user_id.clone()),
            Self::DeployProject { project_id } => Owner::Project(project_id.clone()),
        }
    }

    pub fn deploy_project_id(&self) -> Option<&str> {
        match self {
            Self::DeployProject { project_id } => Some(project_id),
            _ => None,
        }
    }

    pub fn key(&self) -> String {
        self.to_string()
    }

    pub fn parse(key: &str) -> Option<Self> {
        let (kind, id) = key.split_once(':')?;
        if id.is_empty() {
            return None;
        }
        match kind {
            PERSONAL => Some(Self::personal(id)),
            DEPLOY => Some(Self::deploy_project(id)),
            _ => None,
        }
    }
}

impl fmt::Display for Space {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Personal { user_id } => write!(f, "{PERSONAL}:{user_id}"),
            Self::DeployProject { project_id } => write!(f, "{DEPLOY}:{project_id}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_space_round_trips_through_its_key() {
        for space in [Space::personal("u1"), Space::deploy_project("p1")] {
            assert_eq!(Space::parse(&space.key()), Some(space));
        }
    }

    #[test]
    fn each_space_belongs_to_one_mode() {
        assert_eq!(Space::personal("u").mode(), Mode::Chat);
        assert_eq!(Space::deploy_project("p").mode(), Mode::Deploy);
    }

    #[test]
    fn personal_work_belongs_to_the_user_and_project_work_to_the_project() {
        assert_eq!(Space::personal("u").owner(), Owner::User("u".into()));
        assert_eq!(
            Space::deploy_project("p").owner(),
            Owner::Project("p".into())
        );
    }

    #[test]
    fn malformed_keys_are_rejected() {
        assert_eq!(Space::parse("deploy:"), None);
        assert_eq!(Space::parse("unknown:x"), None);
        assert_eq!(Space::parse("nokind"), None);
    }
}
