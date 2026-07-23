use axum::{
    Json,
    extract::State,
    http::{Request, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};

pub struct AuthState {
    pub required: bool,
    store: PathBuf,
    approved: Mutex<HashSet<String>>,
    pairing: Mutex<Option<PairingCode>>,
}
struct PairingCode {
    hash: String,
    expires: Instant,
}

impl AuthState {
    pub fn new(required: bool, state_dir: PathBuf) -> Self {
        let store = state_dir.join("approved-devices");
        let approved = std::fs::read_to_string(&store)
            .ok()
            .map(|s| s.lines().map(str::to_owned).collect())
            .unwrap_or_default();
        Self {
            required,
            store,
            approved: Mutex::new(approved),
            pairing: Mutex::new(None),
        }
    }
    pub fn open_pairing(&self) -> Result<PairingInfo, String> {
        let code = format!("{:06}", rand::rng().random_range(0..1_000_000));
        *self.pairing.lock().unwrap() = Some(PairingCode {
            hash: hash(&code),
            expires: Instant::now() + Duration::from_secs(300),
        });
        Ok(PairingInfo {
            code,
            expires_in: 300,
        })
    }
    fn approve(&self, code: &str) -> Result<String, &'static str> {
        let mut window = self.pairing.lock().unwrap();
        let valid = window
            .as_ref()
            .is_some_and(|p| p.expires > Instant::now() && p.hash == hash(code));
        if !valid {
            return Err("pairing code is invalid or expired");
        }
        *window = None;
        let token: String = (0..32)
            .map(|_| format!("{:02x}", rand::random::<u8>()))
            .collect();
        self.approved.lock().unwrap().insert(hash(&token));
        self.persist();
        Ok(token)
    }
    fn valid(&self, token: &str) -> bool {
        self.approved.lock().unwrap().contains(&hash(token))
    }
    fn revoke(&self, token: &str) {
        self.approved.lock().unwrap().remove(&hash(token));
        self.persist();
    }
    fn persist(&self) {
        if let Some(parent) = self.store.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let data = self
            .approved
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        let _ = std::fs::write(&self.store, data);
    }
}
fn hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
fn cookie(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .map(str::trim)
        .find_map(|c| c.strip_prefix("rustytune_session="))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingInfo {
    pub code: String,
    pub expires_in: u64,
}
#[derive(Deserialize)]
pub struct PairRequest {
    pub code: String,
}
pub async fn pair(
    State(state): State<crate::api::SharedState>,
    Json(req): Json<PairRequest>,
) -> Response {
    let auth = &state.auth;
    match auth.approve(&req.code) {
        Ok(token) => (
            [(
                header::SET_COOKIE,
                format!("rustytune_session={token}; Path=/; HttpOnly; SameSite=Strict"),
            )],
            Json(serde_json::json!({"paired":true})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error":e})),
        )
            .into_response(),
    }
}
pub async fn logout(
    State(state): State<crate::api::SharedState>,
    headers: axum::http::HeaderMap,
) -> Response {
    let auth = &state.auth;
    if let Some(t) = cookie(&headers) {
        auth.revoke(t)
    }
    (
        [(
            header::SET_COOKIE,
            "rustytune_session=; Path=/; Max-Age=0; HttpOnly; SameSite=Strict",
        )],
        StatusCode::NO_CONTENT,
    )
        .into_response()
}

pub async fn require_auth(
    State(auth): State<std::sync::Arc<AuthState>>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    if !auth.required {
        return next.run(req).await;
    }
    let path = req.uri().path();
    if path == "/api/health" || path == "/api/pair" || !path.starts_with("/api/") {
        return next.run(req).await;
    }
    if !cookie(req.headers()).is_some_and(|t| auth.valid(t)) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error":"pairing required"})),
        )
            .into_response();
    }
    if !matches!(
        *req.method(),
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    ) {
        let origin = req
            .headers()
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok());
        let host = req
            .headers()
            .get(header::HOST)
            .and_then(|v| v.to_str().ok());
        if origin.is_some()
            && !matches!((origin,host),(Some(o),Some(h)) if o==format!("http://{h}") || o==format!("https://{h}"))
        {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error":"cross-origin request rejected"})),
            )
                .into_response();
        }
    }
    next.run(req).await
}
