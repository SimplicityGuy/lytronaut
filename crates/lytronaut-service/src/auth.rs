//! Single-administrator credentials and durable opaque sessions. No source library access.
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::{
    fmt,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use subtle::ConstantTimeEq;
use tokio::sync::Semaphore;

/// Deliberately excludes SQL, filesystem and credential details from Debug and Display.
#[derive(Debug, Clone, Copy)]
pub struct AuthError;
impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("authentication unavailable")
    }
}
impl std::error::Error for AuthError {}

#[derive(Clone)]
pub struct AuthStore {
    pool: SqlitePool,
    dummy_hash: Arc<String>,
    hashing: Arc<Semaphore>,
}
#[derive(Clone)]
pub(crate) struct Session {
    pub(crate) digest: String,
    pub(crate) csrf: String,
}

pub(crate) fn now() -> Result<i64, AuthError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AuthError)?
        .as_secs()
        .try_into()
        .map_err(|_| AuthError)
}
pub(crate) fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
pub(crate) fn secret() -> Result<String, AuthError> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|_| AuthError)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
pub(crate) fn token_valid(token: &str) -> bool {
    token.len() == 64
        && token
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(crate) fn equal(a: &str, b: &str) -> bool {
    bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}
fn valid_credentials(username: &str, password: &str) -> bool {
    (1..=64).contains(&username.len())
        && username
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        && (12..=1024).contains(&password.len())
}
async fn hash_password(password: String) -> Result<String, AuthError> {
    tokio::task::spawn_blocking(move || {
        let mut salt = [0; 16];
        getrandom::fill(&mut salt).map_err(|_| AuthError)?;
        let salt = SaltString::encode_b64(&salt).map_err(|_| AuthError)?;
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|h| h.to_string())
            .map_err(|_| AuthError)
    })
    .await
    .map_err(|_| AuthError)?
}
impl AuthStore {
    /// `bootstrap` is a 256-bit token obtained from a local mode-0600 file, never HTTP.
    /// Reopening an initialized database ignores bootstrap. Its hash is consumed atomically.
    pub async fn needs_bootstrap(pool: &SqlitePool) -> Result<bool, AuthError> {
        sqlx::raw_sql("CREATE TABLE IF NOT EXISTS auth_account (singleton INTEGER PRIMARY KEY CHECK(singleton=1), username_hash TEXT, password_hash TEXT, bootstrap_hash TEXT); CREATE TABLE IF NOT EXISTS auth_sessions (token_hash TEXT PRIMARY KEY, expires INTEGER NOT NULL, created INTEGER NOT NULL); CREATE TABLE IF NOT EXISTS auth_attempts (kind TEXT PRIMARY KEY, start INTEGER NOT NULL, count INTEGER NOT NULL);")
            .execute(pool).await.map_err(|_| AuthError)?;
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM auth_account WHERE password_hash IS NOT NULL")
                .fetch_one(pool)
                .await
                .map_err(|_| AuthError)?;
        Ok(count == 0)
    }
    pub async fn open(pool: SqlitePool, bootstrap: Option<&str>) -> Result<Self, AuthError> {
        Self::needs_bootstrap(&pool).await?;
        if let Some(token) = bootstrap {
            if !token_valid(token) {
                return Err(AuthError);
            }
            sqlx::query(
                "INSERT OR IGNORE INTO auth_account (singleton,bootstrap_hash) VALUES (1,?)",
            )
            .bind(digest(token))
            .execute(&pool)
            .await
            .map_err(|_| AuthError)?;
        }
        let exists: i64 = sqlx::query_scalar("SELECT count(*) FROM auth_account")
            .fetch_one(&pool)
            .await
            .map_err(|_| AuthError)?;
        if exists != 1 {
            return Err(AuthError);
        }
        let pending: Option<String> =
            sqlx::query_scalar("SELECT bootstrap_hash FROM auth_account WHERE singleton=1")
                .fetch_one(&pool)
                .await
                .map_err(|_| AuthError)?;
        if let Some(pending) = pending
            && !bootstrap.is_some_and(|value| equal(&pending, &digest(value)))
        {
            return Err(AuthError);
        }
        Ok(Self {
            pool,
            dummy_hash: Arc::new(hash_password(secret()?).await?),
            hashing: Arc::new(Semaphore::new(1)),
        })
    }
    pub async fn initialized(pool: &SqlitePool) -> Result<bool, AuthError> {
        // Called after `open`, not against an arbitrary uninitialized catalog.
        let value: i64 =
            sqlx::query_scalar("SELECT count(*) FROM auth_account WHERE password_hash IS NOT NULL")
                .fetch_one(pool)
                .await
                .map_err(|_| AuthError)?;
        Ok(value == 1)
    }
    /// Atomic global budget cannot be reset by changing usernames, spoofing proxy headers,
    /// simultaneous requests, or restarting the server. Reserve before expensive work.
    pub(crate) async fn admit(&self, kind: &str) -> Result<bool, AuthError> {
        let current = now()?;
        let count: i64 = sqlx::query_scalar("INSERT INTO auth_attempts(kind,start,count) VALUES(?,?,1) ON CONFLICT(kind) DO UPDATE SET count=CASE WHEN start <= ? THEN 1 ELSE min(count+1,6) END, start=CASE WHEN start <= ? THEN excluded.start ELSE start END RETURNING count")
            .bind(kind).bind(current).bind(current-60).bind(current-60).fetch_one(&self.pool).await.map_err(|_| AuthError)?;
        Ok(count <= 5)
    }
    pub(crate) async fn setup(
        &self,
        bootstrap: &str,
        username: &str,
        password: &str,
    ) -> Result<bool, AuthError> {
        if !token_valid(bootstrap) || !valid_credentials(username, password) {
            return Ok(false);
        }
        let stored: Option<String> =
            sqlx::query_scalar("SELECT bootstrap_hash FROM auth_account WHERE singleton=1")
                .fetch_one(&self.pool)
                .await
                .map_err(|_| AuthError)?;
        if !stored.is_some_and(|stored| equal(&stored, &digest(bootstrap))) {
            return Ok(false);
        }
        let _permit = self.hashing.acquire().await.map_err(|_| AuthError)?;
        let password_hash = hash_password(password.to_owned()).await?;
        let result = sqlx::query("UPDATE auth_account SET username_hash=?, password_hash=?, bootstrap_hash=NULL WHERE singleton=1 AND password_hash IS NULL AND bootstrap_hash=?")
            .bind(digest(username)).bind(password_hash).bind(digest(bootstrap)).execute(&self.pool).await.map_err(|_| AuthError)?;
        Ok(result.rows_affected() == 1)
    }
    pub(crate) async fn login(
        &self,
        username: &str,
        password: &str,
        ttl: i64,
    ) -> Result<Option<String>, AuthError> {
        let row: (Option<String>, Option<String>) = sqlx::query_as(
            "SELECT username_hash,password_hash FROM auth_account WHERE singleton=1",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|_| AuthError)?;
        let valid_user = row
            .0
            .is_some_and(|stored| equal(&stored, &digest(username)));
        let hash = row.1.unwrap_or_else(|| (*self.dummy_hash).clone());
        // Always run the same verifier, including wrong/missing usernames. No separate credential errors.
        let _permit = self.hashing.acquire().await.map_err(|_| AuthError)?;
        let password = password.to_owned();
        let valid_password = tokio::task::spawn_blocking(move || {
            PasswordHash::new(&hash).is_ok_and(|hash| {
                Argon2::default()
                    .verify_password(password.as_bytes(), &hash)
                    .is_ok()
            })
        })
        .await
        .map_err(|_| AuthError)?;
        if !valid_user || !valid_password {
            return Ok(None);
        }
        let token = secret()?;
        let current = now()?;
        let mut tx = self.pool.begin().await.map_err(|_| AuthError)?;
        sqlx::query("DELETE FROM auth_sessions WHERE expires<=? OR token_hash IN (SELECT token_hash FROM auth_sessions ORDER BY created DESC,token_hash LIMIT -1 OFFSET 63)")
            .bind(current).execute(&mut *tx).await.map_err(|_| AuthError)?;
        sqlx::query("INSERT INTO auth_sessions(token_hash,expires,created) VALUES(?,?,?)")
            .bind(digest(&token))
            .bind(current.checked_add(ttl).ok_or(AuthError)?)
            .bind(current)
            .execute(&mut *tx)
            .await
            .map_err(|_| AuthError)?;
        tx.commit().await.map_err(|_| AuthError)?;
        Ok(Some(token))
    }
    pub(crate) async fn session(&self, token: &str) -> Result<Option<Session>, AuthError> {
        if !token_valid(token) {
            return Ok(None);
        }
        let hash = digest(token);
        let exists: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM auth_sessions WHERE token_hash=? AND expires>?",
        )
        .bind(&hash)
        .bind(now()?)
        .fetch_one(&self.pool)
        .await
        .map_err(|_| AuthError)?;
        Ok((exists == 1).then(|| Session {
            digest: hash,
            csrf: digest(&format!("lytronaut-csrf-v1:{token}")),
        }))
    }
    pub(crate) async fn logout(&self, session: &Session) -> Result<(), AuthError> {
        sqlx::query("DELETE FROM auth_sessions WHERE token_hash=?")
            .bind(&session.digest)
            .execute(&self.pool)
            .await
            .map_err(|_| AuthError)?;
        Ok(())
    }
}
