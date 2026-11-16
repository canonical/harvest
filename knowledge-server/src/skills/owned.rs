use serde_json::{json, Value};
use uuid::Uuid;

use harvest_db::Db;

use crate::modes::Owner;

#[derive(Debug)]
pub enum SkillError {
    Invalid(&'static str),
    NameTaken,
    NotFound,
    Storage(anyhow::Error),
}

impl From<anyhow::Error> for SkillError {
    fn from(e: anyhow::Error) -> Self {
        Self::Storage(e)
    }
}

pub struct NewSkill<'a> {
    pub name: &'a str,
    pub description: &'a str,
    pub content: &'a str,
}

#[derive(Default)]
pub struct SkillChanges<'a> {
    pub name: Option<&'a str>,
    pub description: Option<&'a str>,
    pub content: Option<&'a str>,
}

pub struct OwnedSkills<'a> {
    db: &'a Db,
    owner: &'a Owner,
}

const OWNED: &str = "(project_id = $pid OR user_id = $uid)";

impl<'a> OwnedSkills<'a> {
    pub fn new(db: &'a Db, owner: &'a Owner) -> Self {
        Self { db, owner }
    }

    fn params(&self, extra: Value) -> Value {
        let mut params = json!({ "pid": self.owner.project_id(), "uid": self.owner.user_id() });
        if let (Some(target), Some(source)) = (params.as_object_mut(), extra.as_object()) {
            target.extend(source.clone());
        }
        params
    }

    pub async fn list(&self) -> Result<Vec<Value>, SkillError> {
        Ok(self
            .db
            .query(
                &format!(
                    "SELECT id, name, description, created_at, updated_at, created_by
                     FROM skills WHERE {OWNED} ORDER BY created_at DESC"
                ),
                self.params(json!({})),
            )
            .await?)
    }

    pub async fn get(&self, id: &str) -> Result<Value, SkillError> {
        self.db
            .query(
                &format!(
                    "SELECT id, name, description, content, created_by, created_at, updated_at
                     FROM skills WHERE id = $sid AND {OWNED}"
                ),
                self.params(json!({ "sid": id })),
            )
            .await?
            .into_iter()
            .next()
            .ok_or(SkillError::NotFound)
    }

    async fn name_taken(&self, name: &str, exclude_id: &str) -> Result<bool, SkillError> {
        let rows = self
            .db
            .query(
                &format!(
                    "SELECT id FROM skills
                     WHERE name = $name AND (is_global OR {OWNED}) AND id <> $exclude_id
                     LIMIT 1"
                ),
                self.params(json!({ "name": name, "exclude_id": exclude_id })),
            )
            .await?;
        Ok(!rows.is_empty())
    }

    pub async fn create(&self, skill: NewSkill<'_>, created_by: &str) -> Result<Value, SkillError> {
        let name = skill.name.trim();
        if name.is_empty() {
            return Err(SkillError::Invalid("name is required"));
        }
        if self.name_taken(name, "").await? {
            return Err(SkillError::NameTaken);
        }
        let id = Uuid::new_v4().to_string();
        let now = harvest_db::now_rfc3339();
        self.db
            .execute(
                "INSERT INTO skills (id, project_id, user_id, name, description, content,
                                     created_by, created_at, updated_at)
                 VALUES ($id, $pid, $uid, $name, $description, $content, $created_by, $now, $now)",
                self.params(json!({
                    "id": id, "name": name, "description": skill.description,
                    "content": skill.content, "created_by": created_by, "now": now,
                })),
            )
            .await?;
        Ok(json!({ "id": id, "name": name, "created_at": now }))
    }

    pub async fn update(&self, id: &str, changes: SkillChanges<'_>) -> Result<(), SkillError> {
        let name = changes.name.map(str::trim);
        if name == Some("") {
            return Err(SkillError::Invalid("name cannot be empty"));
        }
        if let Some(name) = name {
            if self.name_taken(name, id).await? {
                return Err(SkillError::NameTaken);
            }
        }
        let updated = self
            .db
            .query(
                &format!(
                    "UPDATE skills SET
                         name        = COALESCE($name::text, name),
                         description = COALESCE($description::text, description),
                         content     = COALESCE($content::text, content),
                         updated_at  = $now
                     WHERE id = $sid AND {OWNED}
                     RETURNING id"
                ),
                self.params(json!({
                    "sid": id, "name": name, "description": changes.description,
                    "content": changes.content, "now": harvest_db::now_rfc3339(),
                })),
            )
            .await?;
        if updated.is_empty() {
            return Err(SkillError::NotFound);
        }
        Ok(())
    }

    pub async fn delete(&self, id: &str) -> Result<(), SkillError> {
        self.db
            .execute(
                &format!("DELETE FROM skills WHERE id = $sid AND {OWNED}"),
                self.params(json!({ "sid": id })),
            )
            .await?;
        Ok(())
    }
}
