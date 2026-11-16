use serde_json::{json, Value};

use harvest_db::Db;

use super::Role;

#[derive(Debug)]
pub enum MemberError {
    UnknownUser,
    LastOwner,
    NotMember,
    Storage(anyhow::Error),
}

impl From<anyhow::Error> for MemberError {
    fn from(e: anyhow::Error) -> Self {
        Self::Storage(e)
    }
}

pub struct Members<'a> {
    db: &'a Db,
    project_id: &'a str,
}

impl<'a> Members<'a> {
    pub fn new(db: &'a Db, project_id: &'a str) -> Self {
        Self { db, project_id }
    }

    pub async fn list(&self) -> Result<Vec<Value>, MemberError> {
        Ok(self
            .db
            .query(
                "SELECT m.user_id, u.email, u.name, m.role, m.added_at
                 FROM deploy_project_members m JOIN users u ON u.id = m.user_id
                 WHERE m.project_id = $pid
                 ORDER BY CASE m.role WHEN 'owner' THEN 0 WHEN 'editor' THEN 1 ELSE 2 END, u.email",
                json!({ "pid": self.project_id }),
            )
            .await?)
    }

    pub async fn add(&self, email: &str, role: Role, added_by: &str) -> Result<(), MemberError> {
        let users = self
            .db
            .query(
                "SELECT id FROM users WHERE lower(email) = lower($email)",
                json!({ "email": email.trim() }),
            )
            .await?;
        let user_id = users
            .first()
            .and_then(|u| u["id"].as_str())
            .ok_or(MemberError::UnknownUser)?;
        let current = self.role_of(user_id).await?;
        if current == Some(Role::Owner) && role != Role::Owner {
            self.ensure_another_owner(user_id).await?;
        }
        self.db
            .execute(
                "INSERT INTO deploy_project_members (project_id, user_id, role, added_by)
                 VALUES ($pid, $uid, $role, $added_by)
                 ON CONFLICT (project_id, user_id) DO UPDATE SET role = EXCLUDED.role",
                json!({ "pid": self.project_id, "uid": user_id, "role": role.as_str(), "added_by": added_by }),
            )
            .await?;
        Ok(())
    }

    pub async fn add_owner(&self, user_id: &str) -> Result<(), MemberError> {
        self.db
            .execute(
                "INSERT INTO deploy_project_members (project_id, user_id, role, added_by)
                 VALUES ($pid, $uid, 'owner', $uid)
                 ON CONFLICT (project_id, user_id) DO UPDATE SET role = 'owner'",
                json!({ "pid": self.project_id, "uid": user_id }),
            )
            .await?;
        Ok(())
    }

    pub async fn set_role(&self, user_id: &str, role: Role) -> Result<(), MemberError> {
        let current = self.role_of(user_id).await?.ok_or(MemberError::NotMember)?;
        if current == Role::Owner && role != Role::Owner {
            self.ensure_another_owner(user_id).await?;
        }
        self.db
            .execute(
                "UPDATE deploy_project_members SET role = $role WHERE project_id = $pid AND user_id = $uid",
                json!({ "pid": self.project_id, "uid": user_id, "role": role.as_str() }),
            )
            .await?;
        Ok(())
    }

    pub async fn remove(&self, user_id: &str) -> Result<(), MemberError> {
        let current = self.role_of(user_id).await?.ok_or(MemberError::NotMember)?;
        if current == Role::Owner {
            self.ensure_another_owner(user_id).await?;
        }
        self.db
            .execute(
                "DELETE FROM deploy_project_members WHERE project_id = $pid AND user_id = $uid",
                json!({ "pid": self.project_id, "uid": user_id }),
            )
            .await?;
        Ok(())
    }

    async fn role_of(&self, user_id: &str) -> Result<Option<Role>, MemberError> {
        let rows = self
            .db
            .query(
                "SELECT role FROM deploy_project_members WHERE project_id = $pid AND user_id = $uid",
                json!({ "pid": self.project_id, "uid": user_id }),
            )
            .await?;
        Ok(rows
            .first()
            .and_then(|r| r["role"].as_str())
            .and_then(|r| r.parse().ok()))
    }

    async fn ensure_another_owner(&self, user_id: &str) -> Result<(), MemberError> {
        let rows = self
            .db
            .query(
                "SELECT count(*) AS owners FROM deploy_project_members
                 WHERE project_id = $pid AND role = 'owner' AND user_id <> $uid",
                json!({ "pid": self.project_id, "uid": user_id }),
            )
            .await?;
        if rows.first().and_then(|r| r["owners"].as_i64()).unwrap_or(0) == 0 {
            return Err(MemberError::LastOwner);
        }
        Ok(())
    }
}
