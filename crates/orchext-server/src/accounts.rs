//! Account records: signup, lookup, and the personal-tenant bootstrap
//! that every new account gets.
//!
//! Uses `sqlx`'s runtime-checked query API rather than the
//! compile-time-validated `query!` macros so the crate builds without a
//! live Postgres. Migrating to `query!` + `cargo sqlx prepare` is a
//! follow-up once CI has a DB available.

use crate::{
    config::DeploymentMode,
    error::ApiError,
    orgs::{self, SignupOutcome},
    password,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgConnection, PgPool};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Account {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct AccountWithPassword {
    id: Uuid,
    email: String,
    display_name: String,
    password: String,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SignupInput {
    pub email: String,
    pub password: String,
    pub display_name: Option<String>,
}

/// Minimum password length. Make configurable once we have a real
/// password policy story.
const MIN_PASSWORD_LEN: usize = 8;

/// Result of a successful signup. Returns the new account plus the
/// org-assignment outcome (became owner of a new org, or landed in
/// pending_signups awaiting approval). Caller (auth handler) issues a
/// session regardless — the awaiting-approval state is reflected in
/// the next `/v1/orgs` call rather than gating session issuance.
pub struct SignupResult {
    pub account: Account,
    pub outcome: SignupOutcome,
}

pub async fn signup(
    db: &PgPool,
    deployment_mode: DeploymentMode,
    input: SignupInput,
) -> Result<SignupResult, ApiError> {
    let email = normalize_email(&input.email)?;
    if input.password.chars().count() < MIN_PASSWORD_LEN {
        return Err(ApiError::InvalidArgument(format!(
            "password must be at least {MIN_PASSWORD_LEN} characters"
        )));
    }
    let display_name = input
        .display_name
        .unwrap_or_else(|| default_display_name(&email))
        .trim()
        .to_string();
    if display_name.is_empty() {
        return Err(ApiError::InvalidArgument(
            "display_name must not be empty".into(),
        ));
    }

    let hash = password::hash(&input.password)
        .map_err(|e| ApiError::Internal(Box::new(e)))?;

    let mut tx = db.begin().await?;

    let account: Account = sqlx::query_as(
        r#"
        INSERT INTO accounts (email, password, display_name)
        VALUES ($1, $2, $3)
        RETURNING id, email, display_name, created_at
        "#,
    )
    .bind(&email)
    .bind(&hash)
    .bind(&display_name)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_account_insert)?;

    // Bootstrap a personal tenant for this account. Workspace endpoints
    // route through `tenant_id`; doing this now keeps the invariant
    // "every account has at least one tenant they own."
    let tenant_row: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO tenants (name, kind)
        VALUES ($1, 'personal')
        RETURNING id
        "#,
    )
    .bind(format!("{}'s personal workspace", display_name))
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO memberships (tenant_id, account_id, role)
        VALUES ($1, $2, 'owner')
        "#,
    )
    .bind(tenant_row.0)
    .bind(account.id)
    .execute(&mut *tx)
    .await?;

    // Pre-approved invitations bypass both the bootstrap rules and
    // the awaiting-approval gate. The admin already said "this email
    // is welcome" — the signup just realizes that.
    let invited_org_ids = orgs::redeem_invitations(&mut tx, &account, &email).await?;
    let outcome = if !invited_org_ids.is_empty() {
        SignupOutcome::InvitedToOrgs {
            org_ids: invited_org_ids,
        }
    } else {
        // Org assignment per D17d. Self-hosted: first user → owner of
        // singleton; subsequent → pending. SaaS: domain-match → pending,
        // new domain → owner of new org claiming that domain.
        match deployment_mode {
            DeploymentMode::SelfHosted => orgs::bootstrap_self_hosted(&mut tx, &account).await?,
            DeploymentMode::Saas => orgs::bootstrap_saas(&mut tx, &account, &email).await?,
        }
    };

    tx.commit().await?;

    Ok(SignupResult { account, outcome })
}

/// Fetch an account by id. Returns `Unauthorized` if missing, matching
/// the enumeration-resistance posture documented in `error.rs`.
pub async fn by_id(db: &PgPool, id: Uuid) -> Result<Account, ApiError> {
    let account: Option<Account> = sqlx::query_as(
        "SELECT id, email, display_name, created_at FROM accounts WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(db)
    .await?;
    account.ok_or(ApiError::Unauthorized)
}

/// Same as [`by_id`] but takes a connection (so it can run inside an
/// in-flight transaction). Used by `orgs::create_org`.
pub async fn by_id_in(conn: &mut PgConnection, id: Uuid) -> Result<Account, ApiError> {
    let account: Option<Account> = sqlx::query_as(
        "SELECT id, email, display_name, created_at FROM accounts WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(conn)
    .await?;
    account.ok_or(ApiError::Unauthorized)
}

/// Verify a password against the account keyed by email. Returns the
/// `Account` on success, `Unauthorized` on any mismatch — including
/// "no such account" — so the error itself cannot be used to probe
/// which emails have accounts.
pub async fn verify_password(
    db: &PgPool,
    email: &str,
    candidate: &str,
) -> Result<Account, ApiError> {
    let email = normalize_email(email)?;
    let row: Option<AccountWithPassword> = sqlx::query_as(
        "SELECT id, email, display_name, password, created_at FROM accounts WHERE email = $1",
    )
    .bind(&email)
    .fetch_optional(db)
    .await?;

    let Some(row) = row else {
        // Run a dummy verify to keep response time roughly constant
        // whether the email exists or not.
        let _ = password::verify(candidate, DUMMY_PHC);
        return Err(ApiError::Unauthorized);
    };

    let ok = password::verify(candidate, &row.password)
        .map_err(|e| ApiError::Internal(Box::new(e)))?;
    if !ok {
        return Err(ApiError::Unauthorized);
    }

    Ok(Account {
        id: row.id,
        email: row.email,
        display_name: row.display_name,
        created_at: row.created_at,
    })
}

/// A valid-shaped Argon2id PHC string over a throwaway password, used
/// when the email doesn't exist so verification time is roughly
/// constant against a timing attacker.
const DUMMY_PHC: &str = "$argon2id$v=19$m=19456,t=2,p=1$ZHVtbXlkdW1teWR1bW15$Yk8vTGFaZ3Brc2FuZG9tSA";

fn normalize_email(raw: &str) -> Result<String, ApiError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ApiError::InvalidArgument("email is required".into()));
    }
    if !trimmed.contains('@') {
        return Err(ApiError::InvalidArgument(
            "email must contain '@'".into(),
        ));
    }
    Ok(trimmed.to_lowercase())
}

fn default_display_name(email: &str) -> String {
    email.split('@').next().unwrap_or(email).to_string()
}

fn map_account_insert(err: sqlx::Error) -> ApiError {
    // 23505 = unique_violation. Any unique collision on this table is
    // the email uniqueness constraint.
    if let sqlx::Error::Database(ref db_err) = err {
        if db_err.code().as_deref() == Some("23505") {
            return ApiError::Conflict("email already registered");
        }
    }
    ApiError::Internal(Box::new(err))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_email_lowercases_and_trims() {
        assert_eq!(
            normalize_email("  User@Example.COM  ").unwrap(),
            "user@example.com"
        );
    }

    #[test]
    fn normalize_email_rejects_empty_and_missing_at() {
        assert!(normalize_email("   ").is_err());
        assert!(normalize_email("no-at-sign").is_err());
    }

    #[test]
    fn dummy_phc_parses() {
        let result = password::verify("anything", DUMMY_PHC);
        assert!(result.is_ok(), "dummy PHC must parse cleanly");
        assert!(
            !result.unwrap(),
            "dummy PHC must never match a real password"
        );
    }
}
