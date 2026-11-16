pub mod guard;
pub mod members;
pub mod recheck;
pub mod repos;

use std::fmt;
use std::str::FromStr;

use serde_json::{json, Value};

use harvest_db::Db;

use crate::auth::jwt::Claims;
use crate::modes::Space;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Read,
    Write,
    Manage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Viewer,
    Editor,
    Owner,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Editor => "editor",
            Self::Owner => "owner",
        }
    }

    pub fn allows(self, action: Action) -> bool {
        self >= action.minimum_role()
    }
}

impl Action {
    pub fn minimum_role(self) -> Role {
        match self {
            Self::Read => Role::Viewer,
            Self::Write => Role::Editor,
            Self::Manage => Role::Owner,
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Role {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "viewer" => Ok(Self::Viewer),
            "editor" => Ok(Self::Editor),
            "owner" => Ok(Self::Owner),
            other => Err(format!("unknown role '{other}'")),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Caller<'a> {
    pub user_id: &'a str,
    pub role: &'a str,
}

impl Caller<'_> {
    fn is_admin(&self) -> bool {
        self.role == "admin"
    }
}

impl<'a> From<&'a Claims> for Caller<'a> {
    fn from(claims: &'a Claims) -> Self {
        Self {
            user_id: &claims.sub,
            role: &claims.role,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Denied {
    NotFound,
    Forbidden,
    Unavailable,
}

pub struct Authorizer<'a> {
    db: &'a Db,
}

impl<'a> Authorizer<'a> {
    pub fn new(db: &'a Db) -> Self {
        Self { db }
    }

    pub async fn authorize(
        &self,
        caller: Caller<'_>,
        space: &Space,
        action: Action,
    ) -> Result<(), Denied> {
        match space {
            Space::Personal { user_id } | Space::CodeSession { user_id, .. } => {
                personal(caller, user_id)
            }
            Space::DeployProject { project_id } => self
                .deploy_project(caller, project_id, action)
                .await
                .map(|_| ()),
        }
    }

    pub async fn deploy_project(
        &self,
        caller: Caller<'_>,
        project_id: &str,
        action: Action,
    ) -> Result<Value, Denied> {
        let row = self
            .db
            .query(
                "SELECT p.id, p.name, p.description, p.created_by, p.created_at, m.role
                 FROM deploy_projects p
                 LEFT JOIN deploy_project_members m ON m.project_id = p.id AND m.user_id = $uid
                 WHERE p.id = $pid",
                json!({ "pid": project_id, "uid": caller.user_id }),
            )
            .await
            .map_err(|_| Denied::Unavailable)?
            .into_iter()
            .next()
            .ok_or(Denied::NotFound)?;
        let membership = row["role"].as_str().and_then(|r| r.parse::<Role>().ok());
        let effective = if caller.is_admin() {
            Some(Role::Owner)
        } else {
            membership
        };
        match effective {
            None => Err(Denied::NotFound),
            Some(role) if !role.allows(action) => Err(Denied::Forbidden),
            Some(role) => {
                let mut project = row;
                project["role"] = json!(role.as_str());
                Ok(project)
            }
        }
    }
}

fn personal(caller: Caller<'_>, owner: &str) -> Result<(), Denied> {
    if caller.user_id == owner {
        Ok(())
    } else {
        Err(Denied::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caller(user_id: &str) -> Caller<'_> {
        Caller {
            user_id,
            role: "user",
        }
    }

    #[test]
    fn a_personal_space_belongs_to_its_owner_only() {
        assert_eq!(personal(caller("u1"), "u1"), Ok(()));
        assert_eq!(personal(caller("u2"), "u1"), Err(Denied::NotFound));
    }

    #[test]
    fn each_role_can_do_what_the_roles_below_it_can() {
        assert!(Role::Viewer.allows(Action::Read));
        assert!(!Role::Viewer.allows(Action::Write));
        assert!(Role::Editor.allows(Action::Write));
        assert!(!Role::Editor.allows(Action::Manage));
        assert!(Role::Owner.allows(Action::Manage));
    }

    #[test]
    fn roles_round_trip_through_their_names() {
        for role in [Role::Viewer, Role::Editor, Role::Owner] {
            assert_eq!(role.as_str().parse::<Role>(), Ok(role));
        }
        assert!("admin".parse::<Role>().is_err());
    }
}
