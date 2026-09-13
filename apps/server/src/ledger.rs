//! Issue #8 Economy ledger baseline: double-entry wallets, atomic transfers,
//! idempotent transactions, derived balances, tenant-isolated history.
//!
//! Single default currency (`CREDITS`). Balances are always derived from
//! `ledger_postings`; no cached column. Every committed transaction nets to
//! zero (`sum(postings) = 0`), enforced in the application transaction and by
//! a deferred database constraint trigger. Transaction ids are idempotency
//! keys: a duplicate id returns the original without double-spending.
//! No real-money concepts, no shops.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    errors::AppError,
    state::{AppState, Bearer},
};

/// Single supported currency for the baseline (referenced by tests and the
/// wallet invariant check; the SQL schema pins the same value).
pub const DEFAULT_CURRENCY: &str = "CREDITS";

/// Typed ledger error.
#[derive(Debug)]
pub enum LedgerError {
    /// Caller-supplied value rejected (bad amount, self-transfer, unknown id reuse).
    BadInput(String),
    /// Caller tried to move or read another user's wallet.
    Forbidden,
    /// Ledger invariant broken (never from user input; always a bug).
    Invariant(String),
    /// Database failure (logged by the HTTP layer, never shown verbatim).
    Database(sqlx::Error),
}

impl std::fmt::Display for LedgerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadInput(detail) => write!(f, "{detail}"),
            Self::Forbidden => write!(f, "not your wallet"),
            Self::Invariant(_) => write!(f, "ledger invariant violated"),
            Self::Database(_) => write!(f, "database error"),
        }
    }
}

impl std::error::Error for LedgerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(err) => Some(err),
            _ => None,
        }
    }
}

/// Wallet row.
#[derive(Debug, Clone)]
pub struct LedgerAccount {
    /// Account id (`UUIDv7`).
    pub id: Uuid,
    /// Owning user (`None` for the system/treasury account).
    #[allow(dead_code)]
    pub owner_user_id: Option<Uuid>,
    /// `user` or `system`.
    #[allow(dead_code)]
    pub kind: String,
    /// Always `CREDITS` in the baseline.
    pub currency: String,
    /// Creation time.
    #[allow(dead_code)]
    pub created_at: DateTime<Utc>,
}

/// Committed ledger transaction.
#[derive(Debug, Clone)]
pub struct LedgerTransaction {
    /// Client-supplied idempotency key.
    pub id: Uuid,
    /// `transfer` or `mint`.
    pub kind: String,
    /// Free-form memo.
    #[allow(dead_code)]
    pub memo: String,
    /// Commit time.
    #[allow(dead_code)]
    pub created_at: DateTime<Utc>,
}

/// One history line from the caller's perspective.
#[derive(Debug, Clone)]
pub struct HistoryItem {
    /// Transaction id.
    pub transaction_id: Uuid,
    /// `transfer` or `mint`.
    pub kind: String,
    /// Net change for the queried wallet in this transaction.
    pub amount: i64,
    /// Transaction time.
    pub created_at: DateTime<Utc>,
}

type AccountRow = (Uuid, Option<Uuid>, String, String, DateTime<Utc>);

fn to_account(row: AccountRow) -> LedgerAccount {
    LedgerAccount {
        id: row.0,
        owner_user_id: row.1,
        kind: row.2,
        currency: row.3,
        created_at: row.4,
    }
}

/// Ensure the user's wallet exists, creating it when missing. Race-safe:
/// concurrent creators collide on the partial unique index and exactly one
/// insert wins (`ON CONFLICT DO NOTHING`), then both read the same row.
///
/// # Errors
///
/// Returns [`LedgerError::Database`] on database failure.
pub async fn ensure_wallet(
    pool: &sqlx::PgPool,
    user_id: Uuid,
) -> Result<LedgerAccount, LedgerError> {
    sqlx::query(
        "INSERT INTO ledger_accounts (id, owner_user_id, kind, currency)
         VALUES ($1, $2, 'user', $3) ON CONFLICT DO NOTHING",
    )
    .bind(Uuid::now_v7())
    .bind(user_id)
    .bind(DEFAULT_CURRENCY)
    .execute(pool)
    .await
    .map_err(map_unknown_owner)?;
    let row: Option<AccountRow> = sqlx::query_as(
        "SELECT id, owner_user_id, kind, currency, created_at
         FROM ledger_accounts WHERE owner_user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(LedgerError::Database)?;
    row.map(to_account)
        .ok_or_else(|| LedgerError::Database(sqlx::Error::RowNotFound))
}

/// Derived balance for the owner's wallet. Tenant-isolated: callers may only
/// read their own wallet.
///
/// # Errors
///
/// Returns [`LedgerError::Forbidden`] when `caller_id != owner_id`, or
/// [`LedgerError::Database`] on database failure.
pub async fn balance(
    pool: &sqlx::PgPool,
    caller_id: Uuid,
    owner_id: Uuid,
) -> Result<i64, LedgerError> {
    if caller_id != owner_id {
        return Err(LedgerError::Forbidden);
    }
    let account = ensure_wallet(pool, owner_id).await?;
    let total: Option<i64> =
        sqlx::query_scalar("SELECT SUM(amount)::BIGINT FROM ledger_postings WHERE account_id = $1")
            .bind(account.id)
            .fetch_optional(pool)
            .await
            .map_err(LedgerError::Database)?
            .flatten();
    Ok(total.unwrap_or(0))
}

// --- HTTP adapter (merged into the main router) ---

/// `GET /v1/wallet` view: the caller's own wallet and derived balance.
#[derive(Debug, Serialize)]
struct WalletBody {
    account_id: Uuid,
    currency: String,
    balance: i64,
}

/// `POST /v1/wallet/transfers` request.
#[derive(Debug, Deserialize)]
struct TransferBody {
    to_user_id: Uuid,
    amount: i64,
    transaction_id: Uuid,
}

/// `POST /v1/wallet/transfers` response. `deduped` reports an idempotent retry.
#[derive(Debug, Serialize)]
struct TransferResponse {
    transaction_id: Uuid,
    from_user_id: Uuid,
    to_user_id: Uuid,
    amount: i64,
    deduped: bool,
}

/// `GET /v1/wallet/history` query.
#[derive(Debug, Deserialize)]
struct HistoryQuery {
    limit: Option<i64>,
    before: Option<Uuid>,
}

/// `GET /v1/wallet/history` entry.
#[derive(Debug, Serialize)]
struct HistoryBody {
    transaction_id: Uuid,
    kind: String,
    amount: i64,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct HistoryPage {
    entries: Vec<HistoryBody>,
}

/// Wallet routes: own balance, caller-sourced transfer, own history.
/// Tenant isolation holds by construction (the source/owner is always the
/// bearer); cross-user reads/moves are rejected in the domain layer.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/wallet", get(get_wallet))
        .route("/v1/wallet/transfers", post(post_transfer))
        .route("/v1/wallet/history", get(get_history))
}

async fn get_wallet(
    State(state): State<AppState>,
    bearer: Bearer,
) -> Result<Json<WalletBody>, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    let account = ensure_wallet(pool, bearer.user_id()).await?;
    let total = balance(pool, bearer.user_id(), bearer.user_id()).await?;
    Ok(Json(WalletBody {
        account_id: account.id,
        currency: account.currency,
        balance: total,
    }))
}

async fn post_transfer(
    State(state): State<AppState>,
    bearer: Bearer,
    Json(body): Json<TransferBody>,
) -> Result<(StatusCode, Json<TransferResponse>), AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    let (tx, created) = transfer(
        pool,
        bearer.user_id(),
        bearer.user_id(),
        body.to_user_id,
        body.amount,
        body.transaction_id,
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(TransferResponse {
            transaction_id: tx.id,
            from_user_id: bearer.user_id(),
            to_user_id: body.to_user_id,
            amount: body.amount,
            deduped: !created,
        }),
    ))
}

async fn get_history(
    State(state): State<AppState>,
    bearer: Bearer,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<HistoryPage>, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    let limit = query.limit.unwrap_or(50);
    if limit <= 0 {
        return Err(AppError::BadRequest("limit must be positive".to_owned()));
    }
    let user_id = bearer.user_id();
    let items = if query.before.is_some() {
        history_before(pool, user_id, user_id, limit.min(100), query.before).await?
    } else {
        history(pool, user_id, user_id, limit.min(100)).await?
    };
    Ok(Json(HistoryPage {
        entries: items
            .into_iter()
            .map(|item| HistoryBody {
                transaction_id: item.transaction_id,
                kind: item.kind,
                amount: item.amount,
                created_at: item.created_at,
            })
            .collect(),
    }))
}

// --- Double-entry transfer/mint core ---

type TxRow = (Uuid, String, String, DateTime<Utc>);

fn to_transaction(row: TxRow) -> LedgerTransaction {
    LedgerTransaction {
        id: row.0,
        kind: row.1,
        memo: row.2,
        created_at: row.3,
    }
}

/// Ensure the singleton system/treasury account inside the caller's
/// transaction. Race-safe via `ON CONFLICT DO NOTHING` + re-read.
async fn ensure_treasury_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<LedgerAccount, LedgerError> {
    sqlx::query(
        "INSERT INTO ledger_accounts (id, owner_user_id, kind, currency)
         VALUES ($1, NULL, 'system', $2) ON CONFLICT DO NOTHING",
    )
    .bind(Uuid::now_v7())
    .bind(DEFAULT_CURRENCY)
    .execute(&mut **tx)
    .await
    .map_err(LedgerError::Database)?;
    let row: Option<AccountRow> = sqlx::query_as(
        "SELECT id, owner_user_id, kind, currency, created_at
         FROM ledger_accounts WHERE kind = 'system'",
    )
    .fetch_optional(&mut **tx)
    .await
    .map_err(LedgerError::Database)?;
    row.map(to_account)
        .ok_or_else(|| LedgerError::Database(sqlx::Error::RowNotFound))
}

/// Ensure a user wallet inside the caller's transaction (same race-safe
/// pattern as [`ensure_wallet`], but joins the ambient transaction so the
/// account creation and the postings commit atomically).
async fn ensure_wallet_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
) -> Result<LedgerAccount, LedgerError> {
    sqlx::query(
        "INSERT INTO ledger_accounts (id, owner_user_id, kind, currency)
         VALUES ($1, $2, 'user', $3) ON CONFLICT DO NOTHING",
    )
    .bind(Uuid::now_v7())
    .bind(user_id)
    .bind(DEFAULT_CURRENCY)
    .execute(&mut **tx)
    .await
    .map_err(map_unknown_owner)?;
    let row: Option<AccountRow> = sqlx::query_as(
        "SELECT id, owner_user_id, kind, currency, created_at
         FROM ledger_accounts WHERE owner_user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(LedgerError::Database)?;
    row.map(to_account)
        .ok_or_else(|| LedgerError::Database(sqlx::Error::RowNotFound))
}

/// Ensure both wallets inside `tx`, creating them in deterministic user-id
/// order. Two concurrent first-time transfers in opposite directions would
/// otherwise deadlock waiting on each other's partial-unique-index insert
/// (40P01). Returns `(from_account, to_account)` regardless of creation
/// order; row locks are separately sorted by account id in
/// [`lock_accounts_tx`].
async fn ensure_wallets_sorted_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    from_user_id: Uuid,
    to_user_id: Uuid,
) -> Result<(LedgerAccount, LedgerAccount), LedgerError> {
    if from_user_id <= to_user_id {
        let from_account = ensure_wallet_tx(tx, from_user_id).await?;
        let to_account = ensure_wallet_tx(tx, to_user_id).await?;
        Ok((from_account, to_account))
    } else {
        let to_account = ensure_wallet_tx(tx, to_user_id).await?;
        let from_account = ensure_wallet_tx(tx, from_user_id).await?;
        Ok((from_account, to_account))
    }
}
async fn lock_accounts_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    first: Uuid,
    second: Uuid,
) -> Result<(), LedgerError> {
    // Fixed ordering keeps concurrent transfers from deadlocking on
    // reciprocal locks.
    let (lo, hi) = if first <= second {
        (first, second)
    } else {
        (second, first)
    };
    sqlx::query("SELECT id FROM ledger_accounts WHERE id IN ($1, $2) ORDER BY id FOR UPDATE")
        .bind(lo)
        .bind(hi)
        .fetch_all(&mut **tx)
        .await
        .map_err(LedgerError::Database)?;
    Ok(())
}

async fn fetch_transaction_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    transaction_id: Uuid,
) -> Result<Option<LedgerTransaction>, LedgerError> {
    let row: Option<TxRow> =
        sqlx::query_as("SELECT id, kind, memo, created_at FROM ledger_transactions WHERE id = $1")
            .bind(transaction_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(LedgerError::Database)?;
    Ok(row.map(to_transaction))
}

async fn fetch_postings_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    transaction_id: Uuid,
) -> Result<Vec<(Uuid, i64)>, LedgerError> {
    sqlx::query_as(
        "SELECT account_id, amount FROM ledger_postings WHERE transaction_id = $1 ORDER BY account_id",
    )
    .bind(transaction_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(LedgerError::Database)
}

fn is_unique_violation(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(db) if db.code().as_deref() == Some("23505"))
}

/// Map a foreign-key violation on wallet creation (owner references a user
/// the database does not know) to [`LedgerError::BadInput`]. This keeps an
/// unknown recipient a uniform 400 instead of a 500 that would oracle user
/// existence (500 unknown vs 400 known-but-unfunded). Anything else stays a
/// database error.
fn map_unknown_owner(err: sqlx::Error) -> LedgerError {
    if let sqlx::Error::Database(db_err) = &err {
        if db_err.code().as_deref() == Some("23503") {
            return LedgerError::BadInput("unknown user".to_owned());
        }
    }
    LedgerError::Database(err)
}

/// Atomic peer-to-peer transfer of `amount` credits from `from_user_id` to
/// `to_user_id`. The caller must own the source wallet (`caller_id ==
/// from_user_id`), otherwise [`LedgerError::Forbidden`]. `transaction_id` is
/// the idempotency key: a retry with identical parameters returns the
/// original `(transaction, false)` without moving funds twice; reuse of the
/// id with different parameters is [`LedgerError::BadInput`]. Amounts must
/// be positive and the sender must cover the debit (no overdraft); both
/// wallets are created when missing and locked for the transaction so
/// concurrent transfers serialize on the account rows.
///
/// # Errors
///
/// Returns [`LedgerError::Forbidden`] for cross-user moves,
/// [`LedgerError::BadInput`] for bad amounts, self-transfers, insufficient
/// funds, or conflicting id reuse, or [`LedgerError::Database`] on failure.
pub async fn transfer(
    pool: &sqlx::PgPool,
    caller_id: Uuid,
    from_user_id: Uuid,
    to_user_id: Uuid,
    amount: i64,
    transaction_id: Uuid,
) -> Result<(LedgerTransaction, bool), LedgerError> {
    if caller_id != from_user_id {
        return Err(LedgerError::Forbidden);
    }
    if amount <= 0 {
        return Err(LedgerError::BadInput("amount must be positive".to_owned()));
    }
    if from_user_id == to_user_id {
        return Err(LedgerError::BadInput(
            "cannot transfer to yourself".to_owned(),
        ));
    }
    let mut tx = pool.begin().await.map_err(LedgerError::Database)?;
    let (from_account, to_account) =
        ensure_wallets_sorted_tx(&mut tx, from_user_id, to_user_id).await?;
    lock_accounts_tx(&mut tx, from_account.id, to_account.id).await?;
    if let Some(existing) = fetch_transaction_tx(&mut tx, transaction_id).await? {
        let postings = fetch_postings_tx(&mut tx, transaction_id).await?;
        let mut expected = vec![(from_account.id, -amount), (to_account.id, amount)];
        expected.sort_unstable();
        let mut actual = postings;
        actual.sort_unstable();
        if existing.kind != "transfer" || actual != expected {
            return Err(LedgerError::BadInput(
                "transaction id already used with different parameters".to_owned(),
            ));
        }
        tx.commit().await.map_err(LedgerError::Database)?;
        return Ok((existing, false));
    }
    let insert_tx =
        sqlx::query("INSERT INTO ledger_transactions (id, kind, memo) VALUES ($1, 'transfer', '')")
            .bind(transaction_id)
            .execute(&mut *tx)
            .await;
    if let Err(err) = insert_tx {
        if is_unique_violation(&err) {
            // Lost a concurrent insert race with the same id: fall back to
            // the idempotent path so exactly one posting set survives.
            let existing = fetch_transaction_tx(&mut tx, transaction_id)
                .await?
                .ok_or(LedgerError::Database(sqlx::Error::RowNotFound))?;
            let postings = fetch_postings_tx(&mut tx, transaction_id).await?;
            let mut expected = vec![(from_account.id, -amount), (to_account.id, amount)];
            expected.sort_unstable();
            let mut actual = postings;
            actual.sort_unstable();
            if existing.kind != "transfer" || actual != expected {
                return Err(LedgerError::BadInput(
                    "transaction id already used with different parameters".to_owned(),
                ));
            }
            tx.commit().await.map_err(LedgerError::Database)?;
            return Ok((existing, false));
        }
        return Err(LedgerError::Database(err));
    }
    for (account_id, leg) in [(from_account.id, -amount), (to_account.id, amount)] {
        sqlx::query(
            "INSERT INTO ledger_postings (id, transaction_id, account_id, amount)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(Uuid::now_v7())
        .bind(transaction_id)
        .bind(account_id)
        .bind(leg)
        .execute(&mut *tx)
        .await
        .map_err(LedgerError::Database)?;
    }
    let tx_sum: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(amount), 0)::BIGINT FROM ledger_postings WHERE transaction_id = $1",
    )
    .bind(transaction_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(LedgerError::Database)?;
    if tx_sum != 0 {
        return Err(LedgerError::Invariant(format!(
            "new transaction {transaction_id} nets to {tx_sum}"
        )));
    }
    let sender_balance: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(amount), 0)::BIGINT FROM ledger_postings WHERE account_id = $1",
    )
    .bind(from_account.id)
    .fetch_one(&mut *tx)
    .await
    .map_err(LedgerError::Database)?;
    if sender_balance < 0 {
        return Err(LedgerError::BadInput("insufficient funds".to_owned()));
    }
    let row: TxRow =
        sqlx::query_as("SELECT id, kind, memo, created_at FROM ledger_transactions WHERE id = $1")
            .bind(transaction_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(LedgerError::Database)?;
    tx.commit().await.map_err(LedgerError::Database)?;
    Ok((to_transaction(row), true))
}

/// Synthetic-credit funding from the system treasury to `to_user_id`.
/// Test/fixture path only (no HTTP route): the treasury leg may go negative,
/// which is how issuance is represented while keeping `sum(postings) = 0`.
/// Idempotency mirrors [`transfer`].
///
/// Kept available outside tests for a future faucet; the binary currently
/// wires no minting route.
///
/// # Errors
///
/// Returns [`LedgerError::BadInput`] for non-positive amounts or conflicting
/// id reuse, or [`LedgerError::Database`] on failure.
#[allow(dead_code)]
pub async fn mint(
    pool: &sqlx::PgPool,
    to_user_id: Uuid,
    amount: i64,
    transaction_id: Uuid,
) -> Result<(LedgerTransaction, bool), LedgerError> {
    if amount <= 0 {
        return Err(LedgerError::BadInput("amount must be positive".to_owned()));
    }
    let mut tx = pool.begin().await.map_err(LedgerError::Database)?;
    let treasury = ensure_treasury_tx(&mut tx).await?;
    let to_account = ensure_wallet_tx(&mut tx, to_user_id).await?;
    lock_accounts_tx(&mut tx, treasury.id, to_account.id).await?;
    if let Some(existing) = fetch_transaction_tx(&mut tx, transaction_id).await? {
        let postings = fetch_postings_tx(&mut tx, transaction_id).await?;
        let mut expected = vec![(treasury.id, -amount), (to_account.id, amount)];
        expected.sort_unstable();
        let mut actual = postings;
        actual.sort_unstable();
        if existing.kind != "mint" || actual != expected {
            return Err(LedgerError::BadInput(
                "transaction id already used with different parameters".to_owned(),
            ));
        }
        tx.commit().await.map_err(LedgerError::Database)?;
        return Ok((existing, false));
    }
    if let Err(err) =
        sqlx::query("INSERT INTO ledger_transactions (id, kind, memo) VALUES ($1, 'mint', '')")
            .bind(transaction_id)
            .execute(&mut *tx)
            .await
    {
        if is_unique_violation(&err) {
            // Lost a concurrent insert race with the same id: fall back to
            // the idempotent path so exactly one posting set survives.
            // Parameters are verified like in `transfer`; a conflicting reuse
            // is rejected rather than reported as a successful dedupe.
            let existing = fetch_transaction_tx(&mut tx, transaction_id)
                .await?
                .ok_or(LedgerError::Database(sqlx::Error::RowNotFound))?;
            let postings = fetch_postings_tx(&mut tx, transaction_id).await?;
            let mut expected = vec![(treasury.id, -amount), (to_account.id, amount)];
            expected.sort_unstable();
            let mut actual = postings;
            actual.sort_unstable();
            if existing.kind != "mint" || actual != expected {
                return Err(LedgerError::BadInput(
                    "transaction id already used with different parameters".to_owned(),
                ));
            }
            tx.commit().await.map_err(LedgerError::Database)?;
            return Ok((existing, false));
        }
        return Err(LedgerError::Database(err));
    }
    for (account_id, leg) in [(treasury.id, -amount), (to_account.id, amount)] {
        sqlx::query(
            "INSERT INTO ledger_postings (id, transaction_id, account_id, amount)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(Uuid::now_v7())
        .bind(transaction_id)
        .bind(account_id)
        .bind(leg)
        .execute(&mut *tx)
        .await
        .map_err(LedgerError::Database)?;
    }
    // Same application-level zero-sum layer as `transfer`: the deferred
    // trigger remains the backstop, not the only check.
    let tx_sum: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(amount), 0)::BIGINT FROM ledger_postings WHERE transaction_id = $1",
    )
    .bind(transaction_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(LedgerError::Database)?;
    if tx_sum != 0 {
        return Err(LedgerError::Invariant(format!(
            "new transaction {transaction_id} nets to {tx_sum}"
        )));
    }
    let row: TxRow =
        sqlx::query_as("SELECT id, kind, memo, created_at FROM ledger_transactions WHERE id = $1")
            .bind(transaction_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(LedgerError::Database)?;
    tx.commit().await.map_err(LedgerError::Database)?;
    Ok((to_transaction(row), true))
}

/// Tenant-isolated transaction history: the caller's own wallet lines only,
/// newest first. `limit` is clamped to 1..=100 by the caller-facing query.
///
/// # Errors
///
/// Returns [`LedgerError::Forbidden`] when reading another wallet, or
/// [`LedgerError::Database`] when the query fails.
pub async fn history(
    pool: &sqlx::PgPool,
    caller_id: Uuid,
    owner_id: Uuid,
    limit: i64,
) -> Result<Vec<HistoryItem>, LedgerError> {
    history_before(pool, caller_id, owner_id, limit, None).await
}

/// Live keyset page strictly older in `(created_at, transaction_id)` order
/// than an optional transaction in the caller's own history. Resolve the
/// timestamp in `PostgreSQL` so timestamp ties lose no precision. Unknown and
/// foreign anchors have the same error; shared transfers are valid for both
/// participants and still return only the caller's posting.
/// Later commits ahead of the boundary require refreshing the first page;
/// later commits behind it may appear during traversal (this is no snapshot).
async fn history_before(
    pool: &sqlx::PgPool,
    caller_id: Uuid,
    owner_id: Uuid,
    limit: i64,
    before: Option<Uuid>,
) -> Result<Vec<HistoryItem>, LedgerError> {
    if caller_id != owner_id {
        return Err(LedgerError::Forbidden);
    }
    let account = ensure_wallet(pool, owner_id).await?;
    let boundary: Option<DateTime<Utc>> = if let Some(id) = before {
        Some(
            sqlx::query_scalar(
                "SELECT t.created_at FROM ledger_transactions t
                 JOIN ledger_postings p ON p.transaction_id = t.id
                 WHERE p.account_id = $1 AND t.id = $2",
            )
            .bind(account.id)
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(LedgerError::Database)?
            .ok_or_else(|| LedgerError::BadInput("invalid history cursor".to_owned()))?,
        )
    } else {
        None
    };
    let rows: Vec<(Uuid, String, i64, DateTime<Utc>)> = sqlx::query_as(
        "SELECT t.id, t.kind, p.amount, t.created_at
         FROM ledger_postings p JOIN ledger_transactions t ON t.id = p.transaction_id
         WHERE p.account_id = $1
           AND ($3::TIMESTAMPTZ IS NULL OR (t.created_at, t.id) < ($3, $4::UUID))
         ORDER BY t.created_at DESC, t.id DESC LIMIT $2",
    )
    .bind(account.id)
    .bind(limit.clamp(1, 100))
    .bind(boundary)
    .bind(before)
    .fetch_all(pool)
    .await
    .map_err(LedgerError::Database)?;
    Ok(rows
        .into_iter()
        .map(|(id, kind, amount, created_at)| HistoryItem {
            transaction_id: id,
            kind,
            amount,
            created_at,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn isolated_pool() -> sqlx::PgPool {
        let url = std::env::var("DATABASE_URL")
            .expect("service-enabled regression requires DATABASE_URL");
        let admin = sqlx::PgPool::connect(&url).await.unwrap();
        let schema = format!("ledger_{}", Uuid::now_v7().simple());
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin)
            .await
            .unwrap();
        admin.close().await;
        let options: sqlx::postgres::PgConnectOptions = url.parse().unwrap();
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(8)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .unwrap();
        crate::MIGRATOR.run(&pool).await.unwrap();
        pool
    }

    async fn ledger_user(pool: &sqlx::PgPool, stamp: i64, name: &str) -> Uuid {
        let handle = format!("{name}{stamp}");
        crate::auth::create_user(
            pool,
            &handle,
            &format!("{handle}@example.invalid"),
            name,
            "synthetic-ledger-password",
        )
        .await
        .expect("synthetic account")
        .id
    }

    async fn http_call(
        app: axum::Router,
        method: &str,
        uri: &str,
        token: &str,
        body: Option<serde_json::Value>,
    ) -> (axum::http::StatusCode, Vec<u8>) {
        use axum::http::Request;
        use http_body_util::BodyExt;
        use tower::ServiceExt;
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {token}"));
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let body = body.map_or_else(axum::body::Body::empty, |json| {
            axum::body::Body::from(json.to_string())
        });
        let response = app.oneshot(builder.body(body).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec();
        (status, bytes)
    }

    #[tokio::test]
    async fn wallet_creation_gives_zero_balance() {
        let pool = isolated_pool().await;
        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let user = ledger_user(&pool, stamp, "ledgerred").await;
        let account = ensure_wallet(&pool, user).await.unwrap();
        assert_eq!(account.owner_user_id, Some(user));
        assert_eq!(account.kind, "user");
        assert_eq!(account.currency, DEFAULT_CURRENCY);
        assert!(account.created_at <= Utc::now());
        let again = ensure_wallet(&pool, user).await.unwrap();
        assert_eq!(account.id, again.id, "wallet creation is idempotent");
        assert_eq!(balance(&pool, user, user).await.unwrap(), 0);
        pool.close().await;
    }

    #[tokio::test]
    async fn atomic_transfer_moves_50_credits() {
        let pool = isolated_pool().await;
        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let alice = ledger_user(&pool, stamp, "ledgera").await;
        let bob = ledger_user(&pool, stamp, "ledgerb").await;
        mint(&pool, alice, 100, Uuid::now_v7()).await.unwrap();
        assert_eq!(balance(&pool, alice, alice).await.unwrap(), 100);
        let tx_id = Uuid::now_v7();
        let (tx, created) = transfer(&pool, alice, alice, bob, 50, tx_id).await.unwrap();
        assert!(created);
        assert_eq!(tx.id, tx_id);
        assert_eq!(tx.kind, "transfer");
        assert_eq!(tx.memo, "");
        assert!(tx.created_at <= Utc::now());
        assert_eq!(balance(&pool, alice, alice).await.unwrap(), 50);
        assert_eq!(balance(&pool, bob, bob).await.unwrap(), 50);
        // Every committed transaction nets to zero.
        let total: i64 =
            sqlx::query_scalar("SELECT COALESCE(SUM(amount),0)::BIGINT FROM ledger_postings")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(total, 0, "conservation: sum of all postings is zero");
        let tx_sum: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(amount),0)::BIGINT FROM ledger_postings WHERE transaction_id = $1",
        )
        .bind(tx_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(tx_sum, 0, "sum(postings) = 0 for the transfer");
        pool.close().await;
    }

    #[tokio::test]
    async fn duplicate_transaction_id_is_idempotent() {
        let pool = isolated_pool().await;
        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let alice = ledger_user(&pool, stamp, "ledgeridemA").await;
        let bob = ledger_user(&pool, stamp, "ledgeridemB").await;
        mint(&pool, alice, 100, Uuid::now_v7()).await.unwrap();
        let tx_id = Uuid::now_v7();
        let (_, first_created) = transfer(&pool, alice, alice, bob, 50, tx_id).await.unwrap();
        assert!(first_created);
        let (retry, second_created) = transfer(&pool, alice, alice, bob, 50, tx_id).await.unwrap();
        assert!(!second_created, "duplicate id must report deduped");
        assert_eq!(retry.id, tx_id);
        assert_eq!(balance(&pool, alice, alice).await.unwrap(), 50);
        assert_eq!(balance(&pool, bob, bob).await.unwrap(), 50);
        let legs: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::BIGINT FROM ledger_postings WHERE transaction_id = $1",
        )
        .bind(tx_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(legs, 2, "duplicate must not create a second posting set");
        // Conflicting reuse of the same id is rejected, not applied.
        let conflict = transfer(&pool, alice, alice, bob, 10, tx_id).await;
        assert!(
            matches!(conflict, Err(LedgerError::BadInput(_))),
            "conflicting id reuse must fail, got {conflict:?}"
        );
        assert_eq!(balance(&pool, alice, alice).await.unwrap(), 50);
        assert_eq!(balance(&pool, bob, bob).await.unwrap(), 50);
        pool.close().await;
    }

    #[tokio::test]
    async fn conflicting_mint_id_reuse_is_rejected() {
        let pool = isolated_pool().await;
        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let alice = ledger_user(&pool, stamp, "ledgermintA").await;
        let tx_id = Uuid::now_v7();
        let (_, created) = mint(&pool, alice, 100, tx_id).await.unwrap();
        assert!(created);
        // Identical retry dedupes.
        let (_, redone) = mint(&pool, alice, 100, tx_id).await.unwrap();
        assert!(!redone, "identical mint retry must report deduped");
        // Same id, different amount: rejected, balance untouched.
        let conflict = mint(&pool, alice, 25, tx_id).await;
        assert!(
            matches!(conflict, Err(LedgerError::BadInput(_))),
            "conflicting mint id reuse must fail, got {conflict:?}"
        );
        assert_eq!(balance(&pool, alice, alice).await.unwrap(), 100);
        pool.close().await;
    }

    /// Unknown recipient is a uniform 400, not a 500: the FK violation on
    /// wallet creation must not oracle user existence (500 unknown vs 400
    /// known-but-unfunded).
    #[tokio::test]
    async fn unknown_recipient_is_bad_input_not_internal() {
        let pool = isolated_pool().await;
        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let alice = ledger_user(&pool, stamp, "ledgerunkA").await;
        mint(&pool, alice, 100, Uuid::now_v7()).await.unwrap();
        let ghost = Uuid::now_v7();
        let moved = transfer(&pool, alice, alice, ghost, 10, Uuid::now_v7()).await;
        assert!(
            matches!(moved, Err(LedgerError::BadInput(_))),
            "unknown recipient must be BadInput, got {moved:?}"
        );
        assert_eq!(balance(&pool, alice, alice).await.unwrap(), 100);
        pool.close().await;
    }

    /// Both wallets cold, concurrent transfers in opposite directions: wallet
    /// creation runs in deterministic user order, so no two transactions can
    /// deadlock on the partial-unique-index inserts (40P01). Every attempt
    /// here fails overdraft (zero funds) after creation; none may surface a
    /// database/deadlock error.
    #[tokio::test]
    async fn cold_wallet_bidirectional_transfers_never_deadlock() {
        let pool = isolated_pool().await;
        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let alice = ledger_user(&pool, stamp, "ledgercoldA").await;
        let bob = ledger_user(&pool, stamp, "ledgercoldB").await;
        let mut handles = Vec::new();
        for i in 0..20 {
            let pool = pool.clone();
            handles.push(tokio::spawn(async move {
                let id = Uuid::now_v7();
                if i % 2 == 0 {
                    transfer(&pool, alice, alice, bob, 1, id).await
                } else {
                    transfer(&pool, bob, bob, alice, 1, id).await
                }
            }));
        }
        for handle in handles {
            match handle.await.expect("task panicked") {
                Ok(_) => panic!("cold wallets hold no funds; success is impossible"),
                Err(LedgerError::BadInput(_)) => {}
                Err(other) => panic!("deadlock or db failure surfaced: {other:?}"),
            }
        }
        assert_eq!(balance(&pool, alice, alice).await.unwrap(), 0);
        assert_eq!(balance(&pool, bob, bob).await.unwrap(), 0);
        pool.close().await;
    }

    /// The deferred trigger rejects direct-SQL tampering: deleting one leg,
    /// or a whole transaction, violates the two-legs invariant at COMMIT.
    #[tokio::test]
    async fn trigger_rejects_posting_deletes() {
        let pool = isolated_pool().await;
        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let alice = ledger_user(&pool, stamp, "ledgertamperA").await;
        let tx_id = Uuid::now_v7();
        mint(&pool, alice, 100, tx_id).await.unwrap();
        let one_leg: Uuid =
            sqlx::query_scalar("SELECT id FROM ledger_postings WHERE transaction_id = $1 LIMIT 1")
                .bind(tx_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        let removed = sqlx::query("DELETE FROM ledger_postings WHERE id = $1")
            .bind(one_leg)
            .execute(&pool)
            .await;
        assert!(removed.is_err(), "single-leg delete must trip the trigger");
        let wiped = sqlx::query("DELETE FROM ledger_transactions WHERE id = $1")
            .bind(tx_id)
            .execute(&pool)
            .await;
        assert!(
            wiped.is_err(),
            "whole-transaction delete must trip the trigger"
        );
        assert_eq!(balance(&pool, alice, alice).await.unwrap(), 100);
        pool.close().await;
    }

    #[tokio::test]
    async fn tenant_isolation_blocks_cross_wallet_moves_and_reads() {
        let pool = isolated_pool().await;
        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let alice = ledger_user(&pool, stamp, "ledgerisoA").await;
        let bob = ledger_user(&pool, stamp, "ledgerisoB").await;
        mint(&pool, bob, 100, Uuid::now_v7()).await.unwrap();
        // Alice cannot move Bob's funds.
        let moved = transfer(&pool, alice, bob, alice, 50, Uuid::now_v7()).await;
        assert!(
            matches!(moved, Err(LedgerError::Forbidden)),
            "cross-user move must be forbidden, got {moved:?}"
        );
        assert_eq!(balance(&pool, bob, bob).await.unwrap(), 100);
        assert_eq!(balance(&pool, alice, alice).await.unwrap(), 0);
        // Alice cannot read Bob's wallet.
        let read = balance(&pool, alice, bob).await;
        assert!(
            matches!(read, Err(LedgerError::Forbidden)),
            "cross-user read must be forbidden, got {read:?}"
        );
        let hist = history(&pool, alice, bob, 10).await;
        assert!(
            matches!(hist, Err(LedgerError::Forbidden)),
            "cross-user history must be forbidden, got {hist:?}"
        );
        pool.close().await;
    }

    #[tokio::test]
    async fn concurrent_transfers_preserve_conservation() {
        let pool = isolated_pool().await;
        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let alice = ledger_user(&pool, stamp, "ledgerconA").await;
        let bob = ledger_user(&pool, stamp, "ledgerconB").await;
        mint(&pool, alice, 1000, Uuid::now_v7()).await.unwrap();
        mint(&pool, bob, 1000, Uuid::now_v7()).await.unwrap();
        // Twenty concurrent 10-credit moves each way; net effect must be zero
        // and no overdraft may appear from the race.
        let mut handles = Vec::new();
        for i in 0..40 {
            let pool = pool.clone();
            let (from, to) = if i % 2 == 0 {
                (alice, bob)
            } else {
                (bob, alice)
            };
            let caller = from;
            handles.push(tokio::spawn(async move {
                transfer(&pool, caller, from, to, 10, Uuid::now_v7())
                    .await
                    .expect("concurrent transfer")
            }));
        }
        for handle in handles {
            handle.await.unwrap();
        }
        assert_eq!(balance(&pool, alice, alice).await.unwrap(), 1000);
        assert_eq!(balance(&pool, bob, bob).await.unwrap(), 1000);
        let total: i64 =
            sqlx::query_scalar("SELECT COALESCE(SUM(amount),0)::BIGINT FROM ledger_postings")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(total, 0, "concurrent transfers must conserve funds");
        let unbalanced: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::BIGINT FROM (SELECT transaction_id FROM ledger_postings \
             GROUP BY transaction_id HAVING COALESCE(SUM(amount),0) <> 0) unbalanced",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(unbalanced, 0, "every transaction must net to zero");
        pool.close().await;
    }

    #[tokio::test]
    async fn balanced_postings_and_replay_invariants_hold() {
        let pool = isolated_pool().await;
        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let alice = ledger_user(&pool, stamp, "ledgerpropA").await;
        let bob = ledger_user(&pool, stamp, "ledgerpropB").await;
        let carol = ledger_user(&pool, stamp, "ledgerpropC").await;
        // Property-style sweep: varied mint/transfer amounts across users.
        let mut funded: i64 = 0;
        for (i, user) in [alice, bob, carol].iter().enumerate() {
            let amount = 100 + i64::try_from(i).unwrap_or(0) * 37;
            mint(&pool, *user, amount, Uuid::now_v7()).await.unwrap();
            funded += amount;
        }
        let legs = [
            (alice, bob, 25),
            (bob, carol, 40),
            (carol, alice, 15),
            (alice, carol, 50),
        ];
        let mut tx_ids = Vec::new();
        for (from, to, amount) in legs {
            let tx_id = Uuid::now_v7();
            tx_ids.push((tx_id, from, to, amount));
            transfer(&pool, from, from, to, amount, tx_id)
                .await
                .unwrap();
        }
        // Invariant 1: every transaction nets to zero.
        let unbalanced: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::BIGINT FROM (SELECT transaction_id FROM ledger_postings \
             GROUP BY transaction_id HAVING COALESCE(SUM(amount),0) <> 0) unbalanced",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(unbalanced, 0, "balanced-postings invariant");
        // Invariant 2: replaying the same ids changes nothing.
        let before_a = balance(&pool, alice, alice).await.unwrap();
        let before_b = balance(&pool, bob, bob).await.unwrap();
        let before_c = balance(&pool, carol, carol).await.unwrap();
        for (tx_id, from, to, amount) in &tx_ids {
            let (_, created) = transfer(&pool, *from, *from, *to, *amount, *tx_id)
                .await
                .unwrap();
            assert!(!created, "replay must dedupe");
        }
        assert_eq!(balance(&pool, alice, alice).await.unwrap(), before_a);
        assert_eq!(balance(&pool, bob, bob).await.unwrap(), before_b);
        assert_eq!(balance(&pool, carol, carol).await.unwrap(), before_c);
        // Invariant 3: user balances plus treasury balance equal zero
        // (issuance is a treasury debit, so conservation holds globally).
        let user_total = before_a + before_b + before_c;
        let treasury_id: Uuid =
            sqlx::query_scalar("SELECT id FROM ledger_accounts WHERE kind = 'system'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let treasury_balance: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(amount),0)::BIGINT FROM ledger_postings WHERE account_id = $1",
        )
        .bind(treasury_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(treasury_balance, -funded);
        assert_eq!(user_total + treasury_balance, 0, "global conservation");
        pool.close().await;
    }

    #[tokio::test]
    async fn history_lists_own_entries_and_rejects_bad_money() {
        let pool = isolated_pool().await;
        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let alice = ledger_user(&pool, stamp, "ledgerhistA").await;
        let bob = ledger_user(&pool, stamp, "ledgerhistB").await;
        assert!(history(&pool, alice, alice, 10).await.unwrap().is_empty());
        mint(&pool, alice, 200, Uuid::now_v7()).await.unwrap();
        let first = Uuid::now_v7();
        transfer(&pool, alice, alice, bob, 50, first).await.unwrap();
        let second = Uuid::now_v7();
        transfer(&pool, alice, alice, bob, 30, second)
            .await
            .unwrap();
        let alice_hist = history(&pool, alice, alice, 10).await.unwrap();
        // Mint (+200) plus two debits (-50, -30).
        assert_eq!(alice_hist.len(), 3);
        let amounts: Vec<i64> = alice_hist.iter().map(|item| item.amount).collect();
        assert!(amounts.contains(&200));
        assert!(amounts.contains(&-50));
        assert!(amounts.contains(&-30));
        let bob_hist = history(&pool, bob, bob, 10).await.unwrap();
        assert_eq!(bob_hist.len(), 2);
        assert!(bob_hist.iter().all(|item| item.amount > 0));
        // Validation: bad amounts, self-transfer, and overdraft all fail
        // without moving funds.
        for (from, to, amount) in [(alice, bob, 0), (alice, bob, -5), (alice, alice, 10)] {
            let bad = transfer(&pool, from, from, to, amount, Uuid::now_v7()).await;
            assert!(
                matches!(bad, Err(LedgerError::BadInput(_))),
                "amount validation must fail, got {bad:?}"
            );
        }
        let overdraft = transfer(&pool, alice, alice, bob, 500, Uuid::now_v7()).await;
        assert!(
            matches!(overdraft, Err(LedgerError::BadInput(_))),
            "overdraft must fail, got {overdraft:?}"
        );
        assert_eq!(balance(&pool, alice, alice).await.unwrap(), 120);
        assert_eq!(balance(&pool, bob, bob).await.unwrap(), 80);
        pool.close().await;
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn wallet_http_reports_balance_transfer_and_history() {
        use axum::http::StatusCode;

        let pool = isolated_pool().await;
        let stamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let alice_handle = format!("ledgerhttpA{stamp}");
        let bob_handle = format!("ledgerhttpB{stamp}");
        for handle in [&alice_handle, &bob_handle] {
            crate::auth::create_user(
                &pool,
                handle,
                &format!("{handle}@example.invalid"),
                "http",
                "synthetic-ledger-password",
            )
            .await
            .unwrap();
        }
        let alice_token = crate::auth::login(&pool, &alice_handle, "synthetic-ledger-password")
            .await
            .unwrap()
            .token;
        let bob_token = crate::auth::login(&pool, &bob_handle, "synthetic-ledger-password")
            .await
            .unwrap()
            .token;
        let alice_id = crate::auth::user_id_by_handle(&pool, &alice_handle)
            .await
            .unwrap();
        let bob_id = crate::auth::user_id_by_handle(&pool, &bob_handle)
            .await
            .unwrap();
        mint(&pool, alice_id, 100, Uuid::now_v7()).await.unwrap();
        let (hub, _) = tokio::sync::broadcast::channel(crate::HUB_CAPACITY);
        let app = crate::build_router(crate::AppState {
            pool: Some(pool.clone()),
            hub,
        });
        // `/wallet`-equivalent API returns the correct derived balance.
        let (status, bytes) = http_call(app.clone(), "GET", "/v1/wallet", &alice_token, None).await;
        assert_eq!(status, StatusCode::OK);
        let wallet: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(wallet["balance"], 100);
        assert_eq!(wallet["currency"], DEFAULT_CURRENCY);
        // Atomic peer transfer of 50 through the HTTP layer.
        let tx_id = Uuid::now_v7();
        let (status, bytes) = http_call(
            app.clone(),
            "POST",
            "/v1/wallet/transfers",
            &alice_token,
            Some(serde_json::json!({
                "to_user_id": bob_id,
                "amount": 50,
                "transaction_id": tx_id,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let created: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(created["deduped"], false);
        // Duplicate POST with the same id dedupes instead of double-spending.
        let (status, bytes) = http_call(
            app.clone(),
            "POST",
            "/v1/wallet/transfers",
            &alice_token,
            Some(serde_json::json!({
                "to_user_id": bob_id,
                "amount": 50,
                "transaction_id": tx_id,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let deduped: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(deduped["deduped"], true);
        let (status, bytes) = http_call(app.clone(), "GET", "/v1/wallet", &alice_token, None).await;
        assert_eq!(status, StatusCode::OK);
        let wallet: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(wallet["balance"], 50);
        let (status, bytes) = http_call(app.clone(), "GET", "/v1/wallet", &bob_token, None).await;
        assert_eq!(status, StatusCode::OK);
        let wallet: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(wallet["balance"], 50, "bob sees only his own wallet");
        // History query surfaces the transfer from each side's perspective.
        let (status, bytes) = http_call(
            app.clone(),
            "GET",
            "/v1/wallet/history?limit=10",
            &alice_token,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let page: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let amounts: Vec<i64> = page["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["amount"].as_i64().unwrap())
            .collect();
        assert!(amounts.contains(&-50), "alice history shows the debit");
        // Bob cannot move Alice's funds: the source is always the bearer.
        let (status, _) = http_call(
            app.clone(),
            "POST",
            "/v1/wallet/transfers",
            &bob_token,
            Some(serde_json::json!({
                "to_user_id": alice_id,
                "amount": 500,
                "transaction_id": Uuid::now_v7(),
            })),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "overdraft must not move funds"
        );
        pool.close().await;
    }

    async fn history_http(app: &Router, token: &str, query: &str) -> serde_json::Value {
        let (status, bytes) = http_call(
            app.clone(),
            "GET",
            &format!("/v1/wallet/history{query}"),
            token,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        serde_json::from_slice(&bytes).unwrap()
    }

    fn page_ids(page: &serde_json::Value) -> Vec<Uuid> {
        page["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| Uuid::parse_str(entry["transaction_id"].as_str().unwrap()).unwrap())
            .collect()
    }

    async fn history_fixture() -> (sqlx::PgPool, Router, Uuid, String, Uuid, String) {
        let pool = isolated_pool().await;
        let alice = ledger_user(&pool, 1, "pageAlice").await;
        let bob = ledger_user(&pool, 1, "pageBob").await;
        let alice_token = crate::auth::login(&pool, "pageAlice1", "synthetic-ledger-password")
            .await
            .unwrap()
            .token;
        let bob_token = crate::auth::login(&pool, "pageBob1", "synthetic-ledger-password")
            .await
            .unwrap()
            .token;
        let (hub, _) = tokio::sync::broadcast::channel(crate::HUB_CAPACITY);
        let app = crate::build_router(crate::AppState {
            pool: Some(pool.clone()),
            hub,
        });
        (pool, app, alice, alice_token, bob, bob_token)
    }

    #[tokio::test]
    async fn history_http_traverses_timestamp_ties_beyond_cap() {
        let (pool, app, alice, token, bob, _) = history_fixture().await;
        assert!(page_ids(&history_http(&app, &token, "").await).is_empty());
        let mut expected = Vec::new();
        for n in 1..=205_u128 {
            let id = Uuid::from_u128(n);
            mint(&pool, alice, 1, id).await.unwrap();
            expected.push(id);
        }
        // All 205 timestamps tie at microsecond precision, across every page.
        sqlx::query("UPDATE ledger_transactions SET created_at = '2026-01-01 00:00:00.123456+00'")
            .execute(&pool)
            .await
            .unwrap();
        expected.reverse();
        mint(&pool, bob, 900, Uuid::now_v7()).await.unwrap();
        let default = history_http(&app, &token, "").await;
        assert_eq!(
            default.as_object().unwrap().len(),
            1,
            "preserve response envelope"
        );
        assert_eq!(page_ids(&default), expected[..50]);
        assert_eq!(
            page_ids(&history_http(&app, &token, "?limit=9223372036854775807").await),
            expected[..100]
        );
        for limit in [1, 37, 100] {
            let mut actual = Vec::new();
            let mut before = None;
            // Bound the test even if a regression endlessly repeats page one.
            for _ in 0..=205 {
                let query = before.map_or_else(
                    || format!("?limit={limit}"),
                    |id| format!("?limit={limit}&before={id}"),
                );
                let page = history_http(&app, &token, &query).await;
                let ids = page_ids(&page);
                assert!(page["entries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|e| e["amount"] == 1));
                if ids.is_empty() {
                    break;
                }
                assert!(
                    ids.iter().all(|id| !actual.contains(id)),
                    "exclusive cursor must not repeat entries"
                );
                before = ids.last().copied();
                actual.extend(ids);
            }
            assert_eq!(actual, expected);
        }
        assert_eq!(balance(&pool, alice, alice).await.unwrap(), 205);
        assert_eq!(balance(&pool, bob, bob).await.unwrap(), 900);
        pool.close().await;
    }

    #[tokio::test]
    async fn history_http_rejects_invalid_and_foreign_cursors() {
        let (pool, app, alice, token, bob, bob_token) = history_fixture().await;
        let foreign = Uuid::now_v7();
        mint(&pool, bob, 100, foreign).await.unwrap();
        let mut invalid_responses = Vec::new();
        for id in [foreign, Uuid::now_v7(), Uuid::nil()] {
            let response = http_call(
                app.clone(),
                "GET",
                &format!("/v1/wallet/history?before={id}"),
                &token,
                None,
            )
            .await;
            assert_eq!(response.0, StatusCode::BAD_REQUEST);
            invalid_responses.push(response);
        }
        assert!(
            invalid_responses.windows(2).all(|pair| pair[0] == pair[1]),
            "foreign and unknown anchors must be indistinguishable"
        );
        for query in [
            "?before=",
            "?before=bad",
            "?before=1",
            "?limit=0",
            "?limit=-1",
            "?limit=9223372036854775808",
            "?before=bad&before=bad",
        ] {
            let (status, _) = http_call(
                app.clone(),
                "GET",
                &format!("/v1/wallet/history{query}"),
                &token,
                None,
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{query}");
        }
        let (status, _) =
            http_call(app.clone(), "GET", "/v1/wallet/history?limit=1", "", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let shared = Uuid::now_v7();
        transfer(&pool, bob, bob, alice, 20, shared).await.unwrap();
        assert!(
            !transfer(&pool, bob, bob, alice, 20, shared)
                .await
                .unwrap()
                .1
        );
        let query = format!("?before={shared}");
        assert!(page_ids(&history_http(&app, &token, &query).await).is_empty());
        assert_eq!(
            page_ids(&history_http(&app, &bob_token, &query).await),
            [foreign]
        );
        assert_eq!(balance(&pool, alice, alice).await.unwrap(), 20);
        assert_eq!(balance(&pool, bob, bob).await.unwrap(), 80);
        pool.close().await;
    }

    #[tokio::test]
    async fn history_query_enforces_owner_and_exclusive_live_boundary() {
        let (pool, app, alice, token, bob, _) = history_fixture().await;
        let older = Uuid::from_u128(10);
        let anchor = Uuid::from_u128(20);
        let newer = Uuid::from_u128(30);
        for id in [older, anchor, newer] {
            mint(&pool, alice, 1, id).await.unwrap();
        }
        sqlx::query("UPDATE ledger_transactions SET created_at = '2026-01-01 00:00:00.123456+00'")
            .execute(&pool)
            .await
            .unwrap();
        let initial = history_before(&pool, alice, alice, 2, None).await.unwrap();
        assert_eq!(
            initial.iter().map(|i| i.transaction_id).collect::<Vec<_>>(),
            [newer, anchor]
        );
        // Another task adds rows after page one: a normal current-time commit
        // lies ahead; a synthetic delayed/backdated commit lies behind.
        let ahead = Uuid::from_u128(5); // Timestamp takes precedence over UUID.
        let behind = Uuid::from_u128(40);
        let writer_pool = pool.clone();
        tokio::spawn(async move {
            mint(&writer_pool, alice, 1, ahead).await.unwrap();
            mint(&writer_pool, alice, 1, behind).await.unwrap();
            sqlx::query("UPDATE ledger_transactions SET created_at = '2025-12-31 23:59:59+00' WHERE id = $1")
                .bind(behind).execute(&writer_pool).await.unwrap();
        }).await.unwrap();
        let expected = [older, behind];
        for _ in 0..2 {
            let items = history_before(&pool, alice, alice, 100, Some(anchor))
                .await
                .unwrap();
            assert_eq!(
                items.iter().map(|i| i.transaction_id).collect::<Vec<_>>(),
                expected
            );
            assert_eq!(
                page_ids(&history_http(&app, &token, &format!("?before={anchor}")).await),
                expected
            );
        }
        assert_eq!(
            history(&pool, alice, alice, 100).await.unwrap()[0].transaction_id,
            ahead
        );
        assert!(history_before(&pool, alice, alice, 100, Some(behind))
            .await
            .unwrap()
            .is_empty());
        let foreign = Uuid::now_v7();
        mint(&pool, bob, 50, foreign).await.unwrap();
        for id in [foreign, Uuid::nil()] {
            let error = history_before(&pool, alice, alice, 1, Some(id))
                .await
                .unwrap_err();
            assert!(matches!(&error, LedgerError::BadInput(_)));
            assert_eq!(error.to_string(), "invalid history cursor");
            assert!(matches!(
                history_before(&pool, alice, bob, 1, Some(id)).await,
                Err(LedgerError::Forbidden)
            ));
        }
        // Domain bounds retain the pre-existing clamp behavior.
        assert_eq!(
            history_before(&pool, alice, alice, 0, None)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(balance(&pool, alice, alice).await.unwrap(), 5);
        let total: i64 = sqlx::query_scalar("SELECT SUM(amount)::BIGINT FROM ledger_postings")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(total, 0);
        pool.close().await;
    }
}
