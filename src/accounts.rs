//! Admin-Konten mit Rollen und Änderungsprotokoll (F-63).
//!
//! - Das Konto aus der Konfiguration (`PINECONE_ADMIN_USER`/`…_PASSWORD_HASH`) wird beim Start
//!   als Admin angelegt bzw. aktualisiert. Es bleibt der Notzugang und lässt sich im Admin
//!   weder löschen noch herabstufen.
//! - Redakteure dürfen Neuigkeiten, die TwoGG-Auswahl und Bilder ändern, alles andere nur lesen;
//!   Konten und Protokoll sehen nur Admins. Durchgesetzt in [`guard`] für die ganze Admin-API.
//! - Jede erfolgreiche Änderung (POST/PUT/DELETE) landet im Protokoll. Einträge älter als ein
//!   Jahr löscht das tägliche Aufräumen. Im Admin lässt sich das Protokoll nur lesen.

use axum::body::Body;
use axum::extract::{Path, Query, Request, State as AxumState};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::admin::{ListQuery, content_range};
use crate::auth::{Admin, Role, hash_password, verify_password};
use crate::db::now;
use crate::{ApiError, ApiResult, Config, JsonBody, State};

/// Protokolleinträge so lange behalten.
pub const AUDIT_RETENTION_DAYS: i64 = 365;

pub fn routes() -> Router<State> {
    Router::new()
        .route("/accounts", get(list).post(create))
        .route("/accounts/{id}", get(get_one).put(update).delete(remove))
        .route("/audit", get(audit_list))
}

/// Konto aus der Konfiguration als Admin übernehmen (AK-63.1).
pub async fn bootstrap(db: &sqlx::SqlitePool, config: &Config) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO admin_accounts (username, password_hash, role, created_at) VALUES (?, ?, 'admin', ?)
         ON CONFLICT(username) DO UPDATE SET password_hash = excluded.password_hash, role = 'admin'",
    )
    .bind(&config.admin_user)
    .bind(&config.admin_password_hash)
    .bind(now())
    .execute(db)
    .await?;
    Ok(())
}

/// Anmeldung prüfen; liefert die Rolle.
pub async fn verify(state: &State, username: &str, password: &str) -> ApiResult<Option<Role>> {
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT password_hash, role FROM admin_accounts WHERE username = ?")
            .bind(username)
            .fetch_optional(&state.db)
            .await?;
    let Some((hash, role)) = row else {
        // Gleich viel Zeit wie bei einem echten Konto (keine Rückschlüsse auf Namen).
        let pw = password.to_string();
        let dummy = state.config.admin_password_hash.clone();
        let _ = tokio::task::spawn_blocking(move || verify_password(&dummy, &pw)).await;
        return Ok(None);
    };
    let pw = password.to_string();
    let ok = tokio::task::spawn_blocking(move || verify_password(&hash, &pw))
        .await
        .unwrap_or(false);
    if !ok {
        return Ok(None);
    }
    sqlx::query("UPDATE admin_accounts SET last_login = ? WHERE username = ?")
        .bind(now())
        .bind(username)
        .execute(&state.db)
        .await?;
    Ok(Role::parse(&role))
}

// ---------- Rechte und Protokoll für die ganze Admin-API ----------

/// Welche Pfade ein Redakteur ändern darf.
fn editor_may_write(path: &str) -> bool {
    ["/news", "/recommendations", "/media", "/logout"]
        .iter()
        .any(|p| path == *p || path.starts_with(&format!("{p}/")))
}

fn admin_only(path: &str) -> bool {
    path.starts_with("/accounts") || path.starts_with("/audit")
}

/// Middleware: Rollen prüfen und erfolgreiche Änderungen protokollieren.
pub async fn guard(AxumState(state): AxumState<State>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let method = req.method().clone();
    let token = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string);
    let who = token.as_deref().and_then(|t| state.sessions.user(t));
    let writes = matches!(
        method,
        Method::POST | Method::PUT | Method::DELETE | Method::PATCH
    );
    if let Some((_, Role::Editor)) = &who
        && (admin_only(&path) || (writes && !editor_may_write(&path)))
    {
        return ApiError::new(StatusCode::FORBIDDEN, "Dafür braucht es die Rolle Admin")
            .into_response();
    }
    let response = next.run(req).await;
    let Some((user, _)) = who else {
        return response;
    };
    if !writes || path == "/logout" || !response.status().is_success() {
        return response;
    }
    // Bei Neuanlagen steht die ID nur in der Antwort: Body puffern (Admin-Antworten sind klein).
    let (parts, body) = response.into_parts();
    let bytes = axum::body::to_bytes(body, 1024 * 1024)
        .await
        .unwrap_or_default();
    let id_from_body = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|v| match &v["id"] {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            _ => None,
        });
    let mut segments = path.trim_start_matches('/').split('/');
    let object = segments.next().unwrap_or_default().to_string();
    let rest: Vec<&str> = segments.collect();
    let object_id = rest.first().map(|s| s.to_string()).or(id_from_body);
    let action = match (&method, rest.get(1)) {
        (_, Some(sub)) => (*sub).to_string(),
        (&Method::POST, None) => "create".into(),
        (&Method::PUT, None) | (&Method::PATCH, None) => "update".into(),
        (&Method::DELETE, None) => "delete".into(),
        _ => method.as_str().to_lowercase(),
    };
    record(&state, &user, &action, &object, object_id.as_deref()).await;
    Response::from_parts(parts, Body::from(bytes))
}

/// Protokolleintrag schreiben; Fehler werden geloggt und halten die Änderung nicht auf.
pub async fn record(state: &State, user: &str, action: &str, object: &str, id: Option<&str>) {
    let r = sqlx::query(
        "INSERT INTO audit_log (at, username, action, object_type, object_id) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(now())
    .bind(user)
    .bind(action)
    .bind(object)
    .bind(id)
    .execute(&state.db)
    .await;
    if let Err(e) = r {
        tracing::warn!("Protokolleintrag fehlgeschlagen: {e}");
    }
}

pub async fn cleanup(state: &State) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM audit_log WHERE at < ?")
        .bind(now() - AUDIT_RETENTION_DAYS * 86_400)
        .execute(&state.db)
        .await?;
    Ok(())
}

// ---------- Konten ----------

#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
struct AccountDto {
    #[sqlx(rename = "username")]
    id: String,
    role: String,
    created_at: i64,
    last_login: Option<i64>,
    /// Konto aus der Konfiguration (nicht löschbar).
    #[sqlx(default)]
    #[serde(default)]
    protected: bool,
}

fn mark(state: &State, mut a: AccountDto) -> AccountDto {
    a.protected = a.id == state.config.admin_user;
    a
}

async fn list(
    AxumState(state): AxumState<State>,
    admin: Admin,
    Query(q): Query<ListQuery>,
) -> ApiResult<(HeaderMap, Json<Vec<AccountDto>>)> {
    admin.require_admin()?;
    let p = q.parse(
        &[
            ("id", "username"),
            ("role", "role"),
            ("lastLogin", "last_login"),
        ],
        false,
    )?;
    let rows: Vec<AccountDto> = sqlx::query_as(&format!(
        "SELECT username, role, created_at, last_login FROM admin_accounts ORDER BY {} {} LIMIT ? OFFSET ?",
        p.sort_column(),
        if p.desc() { "DESC" } else { "ASC" }
    ))
    .bind(p.limit())
    .bind(p.offset)
    .fetch_all(&state.db)
    .await?;
    let (total,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM admin_accounts")
        .fetch_one(&state.db)
        .await?;
    let rows: Vec<AccountDto> = rows.into_iter().map(|a| mark(&state, a)).collect();
    Ok((
        content_range("accounts", p.offset, rows.len(), total),
        Json(rows),
    ))
}

async fn find(state: &State, id: &str) -> ApiResult<AccountDto> {
    let row: Option<AccountDto> = sqlx::query_as(
        "SELECT username, role, created_at, last_login FROM admin_accounts WHERE username = ?",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    row.map(|a| mark(state, a)).ok_or_else(ApiError::not_found)
}

async fn get_one(
    AxumState(state): AxumState<State>,
    admin: Admin,
    Path(id): Path<String>,
) -> ApiResult<Json<AccountDto>> {
    admin.require_admin()?;
    Ok(Json(find(&state, &id).await?))
}

#[derive(Deserialize)]
struct AccountInput {
    #[serde(default)]
    id: Option<String>,
    role: String,
    /// Neues Passwort; beim Bearbeiten leer = unverändert.
    #[serde(default)]
    password: Option<String>,
}

fn valid_username(name: &str) -> bool {
    (2..=32).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

async fn hash(password: String) -> ApiResult<String> {
    tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Hash fehlgeschlagen"))?
        .map_err(|e| ApiError::bad_request(e.to_string()))
}

async fn create(
    AxumState(state): AxumState<State>,
    admin: Admin,
    JsonBody(input): JsonBody<AccountInput>,
) -> ApiResult<(StatusCode, Json<AccountDto>)> {
    admin.require_admin()?;
    let id = input.id.unwrap_or_default().trim().to_string();
    if !valid_username(&id) {
        return Err(ApiError::bad_request(
            "Benutzername: 2–32 Zeichen, Buchstaben, Ziffern, . _ -",
        ));
    }
    let role = Role::parse(&input.role).ok_or_else(|| ApiError::bad_request("Ungültige Rolle"))?;
    let password = input
        .password
        .filter(|p| !p.is_empty())
        .ok_or_else(|| ApiError::bad_request("Bitte ein Passwort angeben"))?;
    let password_hash = hash(password).await?;
    let r = sqlx::query(
        "INSERT INTO admin_accounts (username, password_hash, role, created_at) VALUES (?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(password_hash)
    .bind(role.as_str())
    .bind(now())
    .execute(&state.db)
    .await;
    match r {
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            return Err(ApiError::bad_request("Diesen Benutzernamen gibt es schon"));
        }
        other => {
            other?;
        }
    }
    Ok((StatusCode::CREATED, Json(find(&state, &id).await?)))
}

async fn admins(state: &State) -> ApiResult<i64> {
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM admin_accounts WHERE role = 'admin'")
        .fetch_one(&state.db)
        .await?;
    Ok(n)
}

async fn update(
    AxumState(state): AxumState<State>,
    admin: Admin,
    Path(id): Path<String>,
    JsonBody(input): JsonBody<AccountInput>,
) -> ApiResult<Json<AccountDto>> {
    admin.require_admin()?;
    let current = find(&state, &id).await?;
    let role = Role::parse(&input.role).ok_or_else(|| ApiError::bad_request("Ungültige Rolle"))?;
    if role == Role::Editor && current.role == "admin" {
        if current.protected {
            return Err(ApiError::bad_request(
                "Das Konto aus der Server-Konfiguration bleibt Admin",
            ));
        }
        if admins(&state).await? <= 1 {
            return Err(ApiError::bad_request(
                "Es muss mindestens einen Admin geben",
            ));
        }
    }
    let password_hash = match input.password.filter(|p| !p.is_empty()) {
        Some(_) if current.protected => {
            return Err(ApiError::bad_request(
                "Das Passwort dieses Kontos kommt aus der Server-Konfiguration",
            ));
        }
        Some(p) => Some(hash(p).await?),
        None => None,
    };
    sqlx::query(
        "UPDATE admin_accounts SET role = ?, password_hash = COALESCE(?, password_hash) WHERE username = ?",
    )
    .bind(role.as_str())
    .bind(password_hash.as_deref())
    .bind(&id)
    .execute(&state.db)
    .await?;
    // Neue Rolle bzw. neues Passwort gilt sofort: laufende Sitzungen des Kontos beenden.
    if role.as_str() != current.role || password_hash.is_some() {
        state.sessions.revoke_user(&id);
    }
    Ok(Json(find(&state, &id).await?))
}

async fn remove(
    AxumState(state): AxumState<State>,
    admin: Admin,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    admin.require_admin()?;
    let current = find(&state, &id).await?;
    if current.protected {
        return Err(ApiError::bad_request(
            "Das Konto aus der Server-Konfiguration lässt sich nicht löschen",
        ));
    }
    if id == admin.user {
        return Err(ApiError::bad_request("Du kannst dich nicht selbst löschen"));
    }
    if current.role == "admin" && admins(&state).await? <= 1 {
        return Err(ApiError::bad_request(
            "Es muss mindestens einen Admin geben",
        ));
    }
    sqlx::query("DELETE FROM admin_accounts WHERE username = ?")
        .bind(&id)
        .execute(&state.db)
        .await?;
    state.sessions.revoke_user(&id);
    Ok(Json(serde_json::json!({ "id": id })))
}

// ---------- Protokoll ----------

#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
struct AuditDto {
    id: i64,
    at: i64,
    username: String,
    action: String,
    object_type: String,
    object_id: Option<String>,
}

async fn audit_list(
    AxumState(state): AxumState<State>,
    admin: Admin,
    Query(q): Query<ListQuery>,
) -> ApiResult<(HeaderMap, Json<Vec<AuditDto>>)> {
    admin.require_admin()?;
    let p = q.parse(
        &[
            ("at", "at"),
            ("username", "username"),
            ("objectType", "object_type"),
        ],
        true,
    )?;
    let user = p.filter_str("username");
    let object = p.filter_str("objectType");
    let rows: Vec<AuditDto> = sqlx::query_as(&format!(
        "SELECT id, at, username, action, object_type, object_id FROM audit_log
         WHERE (?1 IS NULL OR username = ?1) AND (?2 IS NULL OR object_type = ?2)
         ORDER BY {} {}, id DESC LIMIT ?3 OFFSET ?4",
        p.sort_column(),
        if p.desc() { "DESC" } else { "ASC" }
    ))
    .bind(&user)
    .bind(&object)
    .bind(p.limit())
    .bind(p.offset)
    .fetch_all(&state.db)
    .await?;
    let (total,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM audit_log WHERE (?1 IS NULL OR username = ?1) AND (?2 IS NULL OR object_type = ?2)",
    )
    .bind(&user)
    .bind(&object)
    .fetch_one(&state.db)
    .await?;
    Ok((
        content_range("audit", p.offset, rows.len(), total),
        Json(rows),
    ))
}
