//! Pinecone-Freundesliste: Anmeldung mit dem Minecraft-Account (Mojang-Sitzungsserver), Freunde
//! per Spielername, Anfragen und Anwesenheit („spielt gerade …“). Den Status sehen nur Freunde.
//!
//! Anmeldung: `POST /auth/challenge` liefert eine zufällige Server-ID. Der Launcher meldet sie mit
//! seinem Minecraft-Zugang bei Mojang an (`session/minecraft/join`, wie beim Betreten eines
//! Servers). `POST /auth/verify` fragt Mojang (`hasJoined`) und gibt einen Sitzungsschlüssel aus.
//! Den Minecraft-Zugang selbst bekommt der Pinecone-Server nie zu sehen.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{FromRequestParts, Path, State as AxumState};
use axum::http::request::Parts;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::db::now;
use crate::{ApiError, ApiResult, JsonBody, State};

/// Gültigkeit einer Anmeldung.
const SESSION_TTL: i64 = 30 * 24 * 3600;
/// Ohne Lebenszeichen gilt ein Spieler danach als offline.
pub const ONLINE_WINDOW: i64 = 90;
const CHALLENGE_TTL: Duration = Duration::from_secs(120);
const MAX_FRIENDS: i64 = 500;
const MAX_PENDING: i64 = 50;

pub const DEFAULT_SESSION_SERVER: &str = "https://sessionserver.mojang.com";

pub fn routes() -> Router<State> {
    Router::new()
        .route("/auth/challenge", post(challenge))
        .route("/auth/verify", post(verify))
        .route("/me", get(me).delete(delete_me))
        .route("/presence", post(presence))
        .route("/friends", get(friends))
        .route("/friends/requests", post(request))
        .route("/friends/requests/{uuid}/accept", post(accept))
        .route("/friends/requests/{uuid}", delete(decline))
        .route("/friends/{uuid}", delete(remove))
}

/// Offene Herausforderungen: Server-ID → Ablauf.
#[derive(Default)]
pub struct Challenges(Mutex<HashMap<String, Instant>>);

impl Challenges {
    fn issue(&self) -> String {
        let mut bytes = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let mut map = self.0.lock().unwrap();
        let now = Instant::now();
        map.retain(|_, exp| *exp > now);
        // Schutz vor Fluten: höchstens so viele gleichzeitig offene Anmeldungen.
        if map.len() >= 10_000 {
            return id;
        }
        map.insert(id.clone(), now + CHALLENGE_TTL);
        id
    }

    /// Einmal verwendbar.
    fn take(&self, id: &str) -> bool {
        let mut map = self.0.lock().unwrap();
        matches!(map.remove(id), Some(exp) if exp > Instant::now())
    }
}

fn hash_token(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

/// UUID ohne Bindestriche, klein (wie bei Mojang).
pub(crate) fn normalize_uuid(s: &str) -> Option<String> {
    let u: String = s
        .chars()
        .filter(|c| *c != '-')
        .collect::<String>()
        .to_ascii_lowercase();
    (u.len() == 32 && u.bytes().all(|b| b.is_ascii_hexdigit())).then_some(u)
}

fn valid_name(name: &str) -> bool {
    (1..=16).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Angemeldeter Spieler (Bearer-Token aus `/auth/verify`).
pub struct Player {
    pub uuid: String,
    pub name: String,
}

impl FromRequestParts<State> for Player {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &State) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or_else(ApiError::unauthorized)?;
        let row: Option<(String, String)> = sqlx::query_as(
            "SELECT p.uuid, p.name FROM player_sessions s JOIN players p ON p.uuid = s.uuid
             WHERE s.token_hash = ? AND s.expires_at > ?",
        )
        .bind(hash_token(token))
        .bind(now())
        .fetch_optional(&state.db)
        .await
        .map_err(ApiError::from)?;
        let (uuid, name) = row.ok_or_else(ApiError::unauthorized)?;
        Ok(Player { uuid, name })
    }
}

// ---------- Anmeldung ----------

async fn challenge(AxumState(state): AxumState<State>) -> Json<Value> {
    Json(serde_json::json!({ "serverId": state.challenges.issue() }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VerifyRequest {
    username: String,
    server_id: String,
}

async fn verify(
    AxumState(state): AxumState<State>,
    JsonBody(req): JsonBody<VerifyRequest>,
) -> ApiResult<Json<Value>> {
    if !valid_name(&req.username) || !state.challenges.take(&req.server_id) {
        return Err(ApiError::bad_request(
            "Anmeldung abgelaufen, bitte erneut versuchen",
        ));
    }
    #[derive(Deserialize)]
    struct Profile {
        id: String,
        name: String,
    }
    let url = format!(
        "{}/session/minecraft/hasJoined",
        state.config.session_server.trim_end_matches('/')
    );
    let resp = state
        .http
        .get(&url)
        .query(&[("username", &req.username), ("serverId", &req.server_id)])
        .send()
        .await
        .map_err(|e| {
            ApiError::new(
                axum::http::StatusCode::BAD_GATEWAY,
                format!("Mojang nicht erreichbar: {e}"),
            )
        })?;
    // 204: Mojang kennt die Anmeldung nicht.
    if resp.status() != reqwest::StatusCode::OK {
        return Err(ApiError::new(
            axum::http::StatusCode::FORBIDDEN,
            "Minecraft-Account konnte nicht bestätigt werden",
        ));
    }
    let profile: Profile = resp.json().await.map_err(|_| {
        ApiError::new(
            axum::http::StatusCode::BAD_GATEWAY,
            "Unerwartete Antwort von Mojang",
        )
    })?;
    let uuid =
        normalize_uuid(&profile.id).ok_or_else(|| ApiError::bad_request("Ungültige UUID"))?;
    let t = now();
    let prev: Option<(i64,)> = sqlx::query_as("SELECT last_seen FROM players WHERE uuid = ?")
        .bind(&uuid)
        .fetch_optional(&state.db)
        .await?;
    crate::stats::mark_active(&state, prev.map(|p| p.0)).await;
    sqlx::query(
        "INSERT INTO players (uuid, name, name_lower, created_at, last_seen) VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(uuid) DO UPDATE SET name = excluded.name, name_lower = excluded.name_lower,
         last_seen = excluded.last_seen",
    )
    .bind(&uuid)
    .bind(&profile.name)
    .bind(profile.name.to_ascii_lowercase())
    .bind(t)
    .bind(t)
    .execute(&state.db)
    .await?;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    sqlx::query("DELETE FROM player_sessions WHERE expires_at <= ?")
        .bind(t)
        .execute(&state.db)
        .await?;
    sqlx::query("INSERT INTO player_sessions (token_hash, uuid, expires_at) VALUES (?, ?, ?)")
        .bind(hash_token(&token))
        .bind(&uuid)
        .bind(t + SESSION_TTL)
        .execute(&state.db)
        .await?;
    Ok(Json(serde_json::json!({
        "token": token,
        "expiresAt": t + SESSION_TTL,
        "player": { "uuid": uuid, "name": profile.name },
    })))
}

async fn me(player: Player) -> Json<Value> {
    Json(serde_json::json!({ "uuid": player.uuid, "name": player.name }))
}

// ---------- Anwesenheit ----------

/// Was ein Spieler gerade macht. Alles optional, Längen begrenzt.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    /// Name der Instanz, z. B. „REVIVAL: Avalon“.
    #[serde(default)]
    pub instance: Option<String>,
    /// z. B. „Fabric 1.21.11“.
    #[serde(default)]
    pub version: Option<String>,
    /// Server zum Beitreten, nur wenn der Spieler ihn teilen möchte.
    #[serde(default)]
    pub server: Option<String>,
    /// Im Spiel (sonst nur im Launcher).
    #[serde(default)]
    pub playing: bool,
}

impl Activity {
    fn cleaned(mut self) -> Self {
        let clip = |s: Option<String>, max: usize| {
            s.map(|v| {
                v.chars()
                    .filter(|c| !c.is_control())
                    .take(max)
                    .collect::<String>()
            })
            .filter(|v| !v.trim().is_empty())
        };
        self.instance = clip(self.instance, 64);
        self.version = clip(self.version, 32);
        self.server = clip(self.server, 255);
        self
    }
}

#[derive(Deserialize)]
struct PresenceRequest {
    #[serde(default)]
    activity: Activity,
}

async fn presence(
    AxumState(state): AxumState<State>,
    player: Player,
    JsonBody(req): JsonBody<PresenceRequest>,
) -> ApiResult<Json<Value>> {
    let activity = serde_json::to_string(&req.activity.cleaned()).unwrap_or_default();
    let (last_seen,): (i64,) = sqlx::query_as("SELECT last_seen FROM players WHERE uuid = ?")
        .bind(&player.uuid)
        .fetch_one(&state.db)
        .await?;
    crate::stats::mark_active(&state, Some(last_seen)).await;
    sqlx::query("UPDATE players SET last_seen = ?, activity = ? WHERE uuid = ?")
        .bind(now())
        .bind(activity)
        .bind(&player.uuid)
        .execute(&state.db)
        .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

// ---------- Freunde ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FriendDto {
    uuid: String,
    name: String,
    online: bool,
    last_seen: i64,
    activity: Option<Activity>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlayerDto {
    uuid: String,
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FriendsDto {
    friends: Vec<FriendDto>,
    incoming: Vec<PlayerDto>,
    outgoing: Vec<PlayerDto>,
    /// Offene Einladungen an mich (F-56).
    invites: Vec<crate::social::InviteDto>,
    /// Meine Einladungen der letzten 15 Minuten mit Status (offen, angenommen, abgelehnt).
    sent_invites: Vec<crate::social::InviteDto>,
}

async fn friends(
    AxumState(state): AxumState<State>,
    player: Player,
) -> ApiResult<Json<FriendsDto>> {
    let t = now();
    let rows: Vec<(String, String, i64, Option<String>)> = sqlx::query_as(
        "SELECT p.uuid, p.name, p.last_seen, p.activity FROM friendships f
         JOIN players p ON p.uuid = CASE WHEN f.a_uuid = ?1 THEN f.b_uuid ELSE f.a_uuid END
         WHERE f.a_uuid = ?1 OR f.b_uuid = ?1",
    )
    .bind(&player.uuid)
    .fetch_all(&state.db)
    .await?;
    let mut friends: Vec<FriendDto> = rows
        .into_iter()
        .map(|(uuid, name, last_seen, activity)| {
            let online = t - last_seen <= ONLINE_WINDOW;
            FriendDto {
                uuid,
                name,
                online,
                last_seen,
                // Was jemand macht, nur solange er online ist.
                activity: activity
                    .filter(|_| online)
                    .and_then(|a| serde_json::from_str(&a).ok()),
            }
        })
        .collect();
    friends.sort_by(|a, b| {
        b.online
            .cmp(&a.online)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    let players = |sql: &'static str| {
        let state = state.clone();
        let uuid = player.uuid.clone();
        async move {
            sqlx::query_as::<_, (String, String)>(sql)
                .bind(uuid)
                .fetch_all(&state.db)
                .await
                .map(|rows| {
                    rows.into_iter()
                        .map(|(uuid, name)| PlayerDto { uuid, name })
                        .collect::<Vec<_>>()
                })
        }
    };
    let incoming = players(
        "SELECT p.uuid, p.name FROM friend_requests r JOIN players p ON p.uuid = r.from_uuid
         WHERE r.to_uuid = ? ORDER BY r.created_at DESC",
    )
    .await?;
    let outgoing = players(
        "SELECT p.uuid, p.name FROM friend_requests r JOIN players p ON p.uuid = r.to_uuid
         WHERE r.from_uuid = ? ORDER BY r.created_at DESC",
    )
    .await?;
    let (invites, sent_invites) = crate::social::invites_for(&state, &player.uuid).await?;
    Ok(Json(FriendsDto {
        friends,
        incoming,
        outgoing,
        invites,
        sent_invites,
    }))
}

fn pair(a: &str, b: &str) -> (String, String) {
    if a < b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

pub(crate) async fn are_friends(state: &State, a: &str, b: &str) -> ApiResult<bool> {
    let (x, y) = pair(a, b);
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT 1 FROM friendships WHERE a_uuid = ? AND b_uuid = ?")
            .bind(x)
            .bind(y)
            .fetch_optional(&state.db)
            .await?;
    Ok(row.is_some())
}

async fn befriend(state: &State, a: &str, b: &str) -> ApiResult<()> {
    let (x, y) = pair(a, b);
    let mut tx = state.db.begin().await?;
    sqlx::query("INSERT OR IGNORE INTO friendships (a_uuid, b_uuid, created_at) VALUES (?, ?, ?)")
        .bind(&x)
        .bind(&y)
        .bind(now())
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "DELETE FROM friend_requests WHERE (from_uuid = ?1 AND to_uuid = ?2) OR (from_uuid = ?2 AND to_uuid = ?1)",
    )
    .bind(a)
    .bind(b)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

#[derive(Deserialize)]
struct FriendRequest {
    #[serde(default)]
    name: Option<String>,
    /// Freundescode statt Spielername (F-57).
    #[serde(default)]
    code: Option<String>,
}

/// Anfrage per Spielername oder Freundescode. Der andere muss Pinecone schon einmal benutzt
/// haben. Hat er selbst schon angefragt, seid ihr sofort befreundet.
async fn request(
    AxumState(state): AxumState<State>,
    player: Player,
    JsonBody(req): JsonBody<FriendRequest>,
) -> ApiResult<Json<Value>> {
    let (uuid, target_name) = match (req.code.as_deref(), req.name.as_deref()) {
        (Some(code), _) => crate::social::player_by_code(&state, &player, code).await?,
        (None, Some(name)) => {
            let name = name.trim();
            if !valid_name(name) {
                return Err(ApiError::bad_request("Ungültiger Spielername"));
            }
            let target: Option<(String, String)> =
                sqlx::query_as("SELECT uuid, name FROM players WHERE name_lower = ?")
                    .bind(name.to_ascii_lowercase())
                    .fetch_optional(&state.db)
                    .await?;
            target.ok_or_else(|| {
                ApiError::new(
                    axum::http::StatusCode::NOT_FOUND,
                    format!("{name} nutzt Pinecone noch nicht"),
                )
            })?
        }
        (None, None) => return Err(ApiError::bad_request("Spielername oder Code fehlt")),
    };
    if uuid == player.uuid {
        return Err(ApiError::bad_request("Das bist du selbst"));
    }
    if are_friends(&state, &player.uuid, &uuid).await? {
        return Ok(Json(
            serde_json::json!({ "status": "friends", "name": target_name }),
        ));
    }
    let reverse: Option<(i64,)> =
        sqlx::query_as("SELECT 1 FROM friend_requests WHERE from_uuid = ? AND to_uuid = ?")
            .bind(&uuid)
            .bind(&player.uuid)
            .fetch_optional(&state.db)
            .await?;
    if reverse.is_some() {
        befriend(&state, &player.uuid, &uuid).await?;
        return Ok(Json(
            serde_json::json!({ "status": "friends", "name": target_name }),
        ));
    }
    let (pending,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM friend_requests WHERE from_uuid = ?")
            .bind(&player.uuid)
            .fetch_one(&state.db)
            .await?;
    let (count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM friendships WHERE a_uuid = ?1 OR b_uuid = ?1")
            .bind(&player.uuid)
            .fetch_one(&state.db)
            .await?;
    if pending >= MAX_PENDING || count >= MAX_FRIENDS {
        return Err(ApiError::bad_request(
            "Zu viele offene Anfragen oder Freunde",
        ));
    }
    sqlx::query(
        "INSERT OR IGNORE INTO friend_requests (from_uuid, to_uuid, created_at) VALUES (?, ?, ?)",
    )
    .bind(&player.uuid)
    .bind(&uuid)
    .bind(now())
    .execute(&state.db)
    .await?;
    Ok(Json(
        serde_json::json!({ "status": "requested", "name": target_name }),
    ))
}

async fn accept(
    AxumState(state): AxumState<State>,
    player: Player,
    Path(uuid): Path<String>,
) -> ApiResult<Json<Value>> {
    let uuid = normalize_uuid(&uuid).ok_or_else(ApiError::not_found)?;
    let found: Option<(i64,)> =
        sqlx::query_as("SELECT 1 FROM friend_requests WHERE from_uuid = ? AND to_uuid = ?")
            .bind(&uuid)
            .bind(&player.uuid)
            .fetch_optional(&state.db)
            .await?;
    if found.is_none() {
        return Err(ApiError::not_found());
    }
    befriend(&state, &player.uuid, &uuid).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Eingehende Anfrage ablehnen oder eigene zurückziehen.
async fn decline(
    AxumState(state): AxumState<State>,
    player: Player,
    Path(uuid): Path<String>,
) -> ApiResult<Json<Value>> {
    let uuid = normalize_uuid(&uuid).ok_or_else(ApiError::not_found)?;
    sqlx::query(
        "DELETE FROM friend_requests WHERE (from_uuid = ?1 AND to_uuid = ?2) OR (from_uuid = ?2 AND to_uuid = ?1)",
    )
    .bind(&player.uuid)
    .bind(&uuid)
    .execute(&state.db)
    .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn remove(
    AxumState(state): AxumState<State>,
    player: Player,
    Path(uuid): Path<String>,
) -> ApiResult<Json<Value>> {
    let uuid = normalize_uuid(&uuid).ok_or_else(ApiError::not_found)?;
    let (a, b) = pair(&player.uuid, &uuid);
    sqlx::query("DELETE FROM friendships WHERE a_uuid = ? AND b_uuid = ?")
        .bind(a)
        .bind(b)
        .execute(&state.db)
        .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Eigenes Spielerprofil mit Sitzungen, Anfragen und Freundschaften löschen (Recht auf Löschung).
/// Die Fremdschlüssel löschen abhängige Zeilen mit (`ON DELETE CASCADE`).
async fn delete_me(
    AxumState(state): AxumState<State>,
    player: Player,
) -> ApiResult<axum::http::StatusCode> {
    sqlx::query("DELETE FROM players WHERE uuid = ?")
        .bind(&player.uuid)
        .execute(&state.db)
        .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Standard-Aufbewahrung für inaktive Spielerprofile.
pub const DEFAULT_RETENTION_DAYS: u64 = 365;

/// Abgelaufene Sitzungen und lange inaktive Spielerprofile entfernen.
pub async fn cleanup(state: &State) -> sqlx::Result<u64> {
    let now = now();
    sqlx::query("DELETE FROM player_sessions WHERE expires_at <= ?")
        .bind(now)
        .execute(&state.db)
        .await?;
    let days = state.config.player_retention_days;
    if days == 0 {
        return Ok(0);
    }
    let cutoff = now - (days as i64) * 86_400;
    let r = sqlx::query("DELETE FROM players WHERE last_seen < ?")
        .bind(cutoff)
        .execute(&state.db)
        .await?;
    Ok(r.rows_affected())
}

/// Läuft im Hintergrund: beim Start und dann täglich.
pub async fn cleanup_loop(state: State) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(86_400));
    loop {
        tick.tick().await;
        if let Err(e) = crate::accounts::cleanup(&state).await {
            tracing::warn!("Aufräumen des Protokolls: {e}");
        }
        if let Err(e) = crate::serverpack::cleanup(&state).await {
            tracing::warn!("Aufräumen der Server-Packs: {e}");
        }
        if let Err(e) = crate::stats::cleanup(&state).await {
            tracing::warn!("Aufräumen der Statistik: {e}");
        }
        if let Err(e) = crate::social::cleanup(&state).await {
            tracing::warn!("Aufräumen von Einladungen und Freigaben: {e}");
        }
        match cleanup(&state).await {
            Ok(0) => {}
            Ok(n) => tracing::info!("{n} inaktive Spielerprofile gelöscht"),
            Err(e) => tracing::warn!("Aufräumen der Spielerprofile: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuids_and_names() {
        assert_eq!(
            normalize_uuid("069A79F4-44E9-4726-A5BE-FCA90E38AAF5").as_deref(),
            Some("069a79f444e94726a5befca90e38aaf5")
        );
        assert_eq!(normalize_uuid("kaputt"), None);
        assert!(valid_name("wheatley_2"));
        assert!(!valid_name("zu lang für minecraft"));
        assert!(!valid_name(""));
    }

    #[test]
    fn challenges_are_single_use() {
        let c = Challenges::default();
        let id = c.issue();
        assert_eq!(id.len(), 32);
        assert!(c.take(&id));
        assert!(!c.take(&id));
        assert!(!c.take("erfunden"));
    }

    #[test]
    fn activity_is_clipped() {
        let a = Activity {
            instance: Some("x".repeat(200)),
            version: Some("  ".into()),
            server: Some("mc.twogg.de\n".into()),
            playing: true,
        }
        .cleaned();
        assert_eq!(a.instance.unwrap().len(), 64);
        assert_eq!(a.version, None);
        assert_eq!(a.server.as_deref(), Some("mc.twogg.de"));
    }
}
