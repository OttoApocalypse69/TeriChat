//! Auth HTTP adapters: account register/login/logout plus device registration.
//!
//! Thin adapters over the auth domain: request/response shapes live here,
//! domain logic lives in `crate::auth` (plus wallet bootstrap in
//! `crate::ledger`). No behavior changes from the former `routes.rs`.

use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth;
use crate::errors::AppError;
use crate::state::{AppState, Bearer};

/// Public account view. Never carries credential material.
#[derive(Debug, Serialize)]
struct UserBody {
    id: Uuid,
    handle: String,
    email: String,
    display_name: String,
    created_at: DateTime<Utc>,
}

impl From<auth::User> for UserBody {
    fn from(user: auth::User) -> Self {
        Self {
            id: user.id,
            handle: user.handle,
            email: user.email,
            display_name: user.display_name,
            created_at: user.created_at,
        }
    }
}

/// Issued session: the token is shown exactly once, here.
#[derive(Debug, Serialize)]
struct LoginResponse {
    token: String,
    session_id: Uuid,
    user_id: Uuid,
    expires_at: DateTime<Utc>,
    user_handle: String,
}

/// Registered device view, with both public keys as hex (`None` agreement
/// key for rows written before the agreement-key migration).
#[derive(Debug, Serialize)]
struct DeviceBody {
    id: Uuid,
    user_id: Uuid,
    label: String,
    identity_pubkey: String,
    agreement_pubkey: Option<String>,
    created_at: DateTime<Utc>,
}

impl From<auth::Device> for DeviceBody {
    fn from(device: auth::Device) -> Self {
        Self {
            id: device.id,
            user_id: device.user_id,
            label: device.label,
            identity_pubkey: hex::encode(&device.identity_pubkey),
            agreement_pubkey: device.agreement_pubkey.as_deref().map(hex::encode),
            created_at: device.created_at,
        }
    }
}

/// `POST /v1/auth/register` request.
#[derive(Debug, Deserialize)]
struct RegisterBody {
    handle: String,
    email: String,
    display_name: String,
    password: String,
}

/// `POST /v1/auth/login` request.
#[derive(Debug, Deserialize)]
struct LoginBody {
    handle: String,
    password: String,
}

/// `POST /v1/auth/devices` request. Both keys are lowercase or uppercase hex
/// (`64` chars, `32` bytes each); rejected generically otherwise.
#[derive(Debug, Deserialize)]
struct RegisterDeviceBody {
    label: String,
    identity_pubkey: String,
    agreement_pubkey: String,
}

async fn register(
    State(state): State<AppState>,
    Json(body): Json<RegisterBody>,
) -> Result<(StatusCode, Json<UserBody>), AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    // Never log `body.password`.
    let user = auth::create_user(
        pool,
        &body.handle,
        &body.email,
        &body.display_name,
        &body.password,
    )
    .await?;
    // New users receive a wallet account in the same request. A ledger failure
    // here is an internal error (the account already exists at this point).
    crate::ledger::ensure_wallet(pool, user.id).await?;
    Ok((StatusCode::CREATED, Json(UserBody::from(user))))
}

async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginBody>,
) -> Result<Json<LoginResponse>, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    // Never log `body.password`.
    let issued = auth::login(pool, &body.handle, &body.password).await?;
    let handle = auth::normalize_handle(&body.handle).map_err(AppError::from)?;
    Ok(Json(LoginResponse {
        token: issued.token,
        session_id: issued.session.id,
        user_id: issued.session.user_id,
        expires_at: issued.session.expires_at,
        user_handle: handle,
    }))
}

async fn logout(
    State(state): State<AppState>,
    bearer: Bearer,
) -> Result<Json<serde_json::Value>, AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    // The raw token never leaves this call except into its `SHA-256` hash.
    auth::logout(pool, bearer.token()).await?;
    Ok(Json(
        serde_json::json!({ "status": "ok", "session_id": bearer.session_id() }),
    ))
}

async fn register_device(
    State(state): State<AppState>,
    bearer: Bearer,
    Json(body): Json<RegisterDeviceBody>,
) -> Result<(StatusCode, Json<DeviceBody>), AppError> {
    let pool = state.pool.as_ref().ok_or(AppError::NoDatabase)?;
    let decode_key = |raw: &str| {
        hex::decode(raw.trim())
            .ok()
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or(AppError::BadRequest(
                "identity_pubkey and agreement_pubkey must each be 64 hex characters".to_owned(),
            ))
    };
    let identity_pubkey: [u8; 32] = decode_key(&body.identity_pubkey)?;
    let agreement_pubkey: [u8; 32] = decode_key(&body.agreement_pubkey)?;
    let device = auth::register_device(
        pool,
        bearer.user_id(),
        &body.label,
        identity_pubkey,
        agreement_pubkey,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(DeviceBody::from(device))))
}

/// Auth routes under `/v1/auth/*`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/auth/register", post(register))
        .route("/v1/auth/login", post(login))
        .route("/v1/auth/logout", post(logout))
        .route("/v1/auth/devices", post(register_device))
}
