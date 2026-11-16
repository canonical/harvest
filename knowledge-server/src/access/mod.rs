pub mod recheck;
pub mod repos;

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

#[derive(Clone, Copy, Debug)]
pub struct Caller<'a> {
    pub user_id: &'a str,
    pub role: &'a str,
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
            Space::Personal { user_id } => personal(caller, user_id),
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
        _action: Action,
    ) -> Result<Value, Denied> {
        let rows = self
            .db
            .query(
                "SELECT p.id, p.name, p.description, p.group_id, g.name AS group_name,
                        p.created_by, p.created_at
                 FROM projects p JOIN groups g ON g.id = p.group_id
                 WHERE p.id = $pid
                   AND ($role = 'admin' OR EXISTS (
                         SELECT 1 FROM user_groups ug WHERE ug.user_id = $uid AND ug.group_id = g.id))",
                json!({ "pid": project_id, "uid": caller.user_id, "role": caller.role }),
            )
            .await
            .map_err(|_| Denied::Unavailable)?;
        rows.into_iter().next().ok_or(Denied::NotFound)
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
}
