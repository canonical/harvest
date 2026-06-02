use anyhow::Result;
use chrono::Utc;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,
    pub email: String,
    pub name: String,
    pub role: String,
    pub exp: i64,
}

pub fn issue(secret: &str, sub: &str, email: &str, name: &str, role: &str) -> Result<String> {
    let claims = Claims {
        sub: sub.to_string(),
        email: email.to_string(),
        name: name.to_string(),
        role: role.to_string(),
        exp: (Utc::now() + chrono::Duration::days(30)).timestamp(),
    };
    Ok(encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )?)
}

pub fn validate(secret: &str, token: &str) -> Result<Claims> {
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )?;
    Ok(data.claims)
}
