use chrono::{DateTime, FixedOffset, Local};
use loco_rs::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub use super::_entities::agent_api_keys::{ActiveModel, Column, Entity, Model};

#[async_trait::async_trait]
impl ActiveModelBehavior for ActiveModel {
    async fn before_save<C>(self, _db: &C, insert: bool) -> std::result::Result<Self, DbErr>
    where
        C: ConnectionTrait,
    {
        if !insert && self.updated_at.is_unchanged() {
            let mut this = self;
            this.updated_at = ActiveValue::Set(now_tz());
            Ok(this)
        } else {
            Ok(self)
        }
    }
}

/// Scopes a key can hold. `write` always implies `read`: an editor must be
/// able to fetch the document it is about to edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scopes {
    pub read: bool,
    pub write: bool,
}

impl Default for Scopes {
    fn default() -> Self {
        Self {
            read: true,
            write: false,
        }
    }
}

impl Scopes {
    #[must_use]
    pub fn read_only() -> Self {
        Self {
            read: true,
            write: false,
        }
    }

    #[must_use]
    pub fn read_write() -> Self {
        Self {
            read: true,
            write: true,
        }
    }

    #[must_use]
    pub fn can_read(self) -> bool {
        self.read || self.write
    }

    #[must_use]
    pub fn can_write(self) -> bool {
        self.write
    }

    /// Parse and validate the comma-separated `scopes` column / form value.
    ///
    /// # Errors
    /// When the value contains a scope that is not `read` or `write`.
    pub fn parse(raw: &str) -> ModelResult<Self> {
        let mut scopes = Self {
            read: false,
            write: false,
        };
        for part in raw.split([',', ' ']) {
            match part.trim() {
                "" => {}
                "read" => scopes.read = true,
                "write" => {
                    scopes.read = true;
                    scopes.write = true;
                }
                other => {
                    return Err(ModelError::msg(&format!(
                        "unknown scope `{other}`; expected read or write"
                    )))
                }
            }
        }
        if !scopes.read && !scopes.write {
            return Err(ModelError::msg("at least one scope is required"));
        }
        Ok(scopes)
    }

    /// Canonical storage form.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match (self.read, self.write) {
            (_, true) => "read,write",
            (true, false) => "read",
            (false, false) => "",
        }
    }
}

impl Model {
    /// # Errors
    /// Database failure.
    pub async fn all_for_admin(db: &DatabaseConnection) -> ModelResult<Vec<Self>> {
        Ok(Entity::find()
            .order_by_desc(Column::CreatedAt)
            .order_by_desc(Column::Id)
            .all(db)
            .await?)
    }

    /// # Errors
    /// `EntityNotFound` when the id does not exist.
    pub async fn find_by_id(db: &DatabaseConnection, id: i64) -> ModelResult<Self> {
        Entity::find_by_id(id)
            .one(db)
            .await?
            .ok_or(ModelError::EntityNotFound)
    }

    /// Create a key and return `(row, plaintext)`. The plaintext is the only
    /// copy; only its SHA-256 hash is persisted.
    ///
    /// # Errors
    /// Empty/oversized name, invalid scopes, or database failure.
    pub async fn create_key(
        db: &DatabaseConnection,
        name: &str,
        scopes: Scopes,
    ) -> ModelResult<(Self, String)> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ModelError::msg("key name is required"));
        }
        if name.chars().count() > 120 {
            return Err(ModelError::msg("key name is too long (max 120 characters)"));
        }
        if name.chars().any(char::is_control) {
            return Err(ModelError::msg(
                "key name cannot contain control characters",
            ));
        }
        let prefix = Uuid::new_v4().simple().to_string()[..8].to_string();
        let secret = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let plaintext = format!("weft_{prefix}_{secret}");
        let row = ActiveModel {
            name: ActiveValue::set(name.to_string()),
            prefix: ActiveValue::set(prefix),
            key_hash: ActiveValue::set(hash_key(&plaintext)),
            scopes: ActiveValue::set(scopes.as_str().to_string()),
            last_used_at: ActiveValue::set(None),
            revoked_at: ActiveValue::set(None),
            ..Default::default()
        }
        .insert(db)
        .await?;
        Ok((row, plaintext))
    }

    /// Resolve a bearer token to an active key.
    ///
    /// The lookup uses the public prefix, then compares hashes in constant
    /// time. A wrong token never reveals whether a prefix exists beyond the
    /// same 401 response.
    ///
    /// # Errors
    /// Database failure.
    pub async fn authenticate(db: &DatabaseConnection, token: &str) -> ModelResult<Option<Self>> {
        let Some(prefix) = token_prefix(token) else {
            return Ok(None);
        };
        let Some(key) = Entity::find()
            .filter(Column::Prefix.eq(prefix))
            .filter(Column::RevokedAt.is_null())
            .one(db)
            .await?
        else {
            return Ok(None);
        };
        if !crate::util::secure_eq(&hash_key(token), &key.key_hash) {
            return Ok(None);
        }
        Ok(Some(key))
    }

    #[must_use]
    pub fn scopes(&self) -> Scopes {
        Scopes::parse(&self.scopes).unwrap_or_else(|_| Scopes::read_only())
    }

    #[must_use]
    pub fn is_revoked(&self) -> bool {
        self.revoked_at.is_some()
    }

    /// Record usage at most once a minute. Every API request already costs a
    /// database read; this keeps the bookkeeping write off the hot path.
    ///
    /// # Errors
    /// Database failure.
    pub async fn touch(&self, db: &DatabaseConnection) -> ModelResult<()> {
        let now = now_tz();
        if self
            .last_used_at
            .is_some_and(|last| (now - last).num_seconds() < 60)
        {
            return Ok(());
        }
        let mut active: ActiveModel = self.clone().into();
        active.last_used_at = ActiveValue::set(Some(now));
        active.updated_at = ActiveValue::set(now);
        active.update(db).await?;
        Ok(())
    }

    /// Revoke a key. Idempotent; the row is kept for an audit trail.
    ///
    /// # Errors
    /// Database failure.
    pub async fn revoke(&self, db: &DatabaseConnection) -> ModelResult<Self> {
        if self.revoked_at.is_some() {
            return Ok(self.clone());
        }
        let now = now_tz();
        let mut active: ActiveModel = self.clone().into();
        active.revoked_at = ActiveValue::set(Some(now));
        active.updated_at = ActiveValue::set(now);
        Ok(active.update(db).await?)
    }
}

#[must_use]
pub fn hash_key(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    crate::util::hex(&hasher.finalize())
}

#[must_use]
pub fn token_prefix(token: &str) -> Option<&str> {
    let rest = token.trim().strip_prefix("weft_")?;
    let (prefix, secret) = rest.split_once('_')?;
    (prefix.len() == 8 && !secret.is_empty()).then_some(prefix)
}

fn now_tz() -> DateTime<FixedOffset> {
    Local::now().fixed_offset()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_parse_and_imply_read() {
        assert_eq!(Scopes::parse("write").unwrap().as_str(), "read,write");
        assert!(Scopes::parse("write").unwrap().can_read());
        assert!(Scopes::parse("read").unwrap().can_read());
        assert!(!Scopes::parse("read").unwrap().can_write());
        assert!(Scopes::parse("bogus").is_err());
        assert!(Scopes::parse("").is_err());
    }

    #[test]
    fn hashes_are_sha256_and_never_the_plaintext() {
        let key = "weft_1234abcd_secret";
        let hash = hash_key(key);
        assert_eq!(hash.len(), 64);
        assert_ne!(hash, key);
        assert_eq!(hash, hash_key(key));
    }

    #[test]
    fn token_prefix_requires_the_weft_shape() {
        assert_eq!(token_prefix("weft_1234abcd_secret").unwrap(), "1234abcd");
        assert!(token_prefix("weft_short_secret").is_none());
        assert!(token_prefix("nope").is_none());
    }
}
