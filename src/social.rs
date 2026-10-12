//! Freunde-Erweiterungen (Plan v2-B): Freundescode (F-57), Einladungen auf einen Server (F-56)
//! und geteilte Sync-Packs (F-58). Alles nur für angemeldete Spieler; Daten hängen am Spieler
//! und verschwinden mit „Meine Freunde-Daten löschen“ (`ON DELETE CASCADE`).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{Path, State as AxumState};
use axum::routing::{get, post};
use axum::{Json, Router};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::now;
use crate::friends::Player;
use crate::{ApiError, ApiResult, JsonBody, State};

/// Einladungen gelten 15 Minuten.
pub const INVITE_TTL: i64 = 15 * 60;
/// Höchstens so viele Einladungen je Spieler und Minute.
const INVITES_PER_MINUTE: i64 = 10;
/// Geteilte Sync-Packs gelten 30 Tage.
pub const SHARE_TTL: i64 = 30 * 86_400;
const MAX_SHARE_BYTES: usize = 64 * 1024;
const MAX_SHARES: i64 = 20;
/// Falsche Freundescodes je Spieler und Stunde, bevor gesperrt wird.
const CODE_FAILURES_PER_HOUR: u32 = 10;

/// Ohne verwechselbare Zeichen (0/O, 1/I/L, U): 30 Zeichen, also knapp 4,9 Bit je Stelle.
const ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTVWXYZ";

pub fn routes() -> Router<State> {
    Router::new()
        .route("/friends/code", get(get_code).post(new_code))
        .route("/invites", post(invite))
        .route("/invites/{id}/answer", post(answer))
        .route("/sync-shares", post(share))
        .route("/sync-shares/{code}", get(get_share))
}

fn random_code(len: usize) -> String {
    let mut rng = rand::rngs::OsRng;
    (0..len)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect()
}

/// Freundescode im Format `XXXX-XXXX-XX` (10 Zeichen ≈ 49 Bit).
fn friend_code() -> String {
    let raw = random_code(10);
    format!("{}-{}-{}", &raw[..4], &raw[4..8], &raw[8..])
}

/// Eingabe vereinheitlichen: Groß, ohne Leerzeichen/Bindestriche, `pinecone://friend/` erlaubt.
pub fn normalize_code(input: &str) -> Option<String> {
    let s = input.trim();
    let s = s.strip_prefix("pinecone://friend/").unwrap_or(s);
    let raw: String = s
        .chars()
        .filter(|c| !matches!(c, '-' | ' '))
        .map(|c| c.to_ascii_uppercase())
        .collect();
    (raw.len() == 10 && raw.bytes().all(|b| ALPHABET.contains(&b)))
        .then(|| format!("{}-{}-{}", &raw[..4], &raw[4..8], &raw[8..]))
}

/// Fehlversuche beim Freundescode je Spieler (gegen Durchprobieren).
#[derive(Default)]
pub struct CodeFailures(Mutex<HashMap<String, (u32, Instant)>>);

impl CodeFailures {
    const WINDOW: Duration = Duration::from_secs(3600);

    pub fn blocked(&self, uuid: &str) -> bool {
        let map = self.0.lock().unwrap();
        map.get(uuid).is_some_and(|(n, start)| {
            start.elapsed() < Self::WINDOW && *n >= CODE_FAILURES_PER_HOUR
        })
    }

    pub fn fail(&self, uuid: &str) {
        let mut map = self.0.lock().unwrap();
        if map.len() > 10_000 {
            map.retain(|_, (_, start)| start.elapsed() < Self::WINDOW);
        }
        let entry = map.entry(uuid.to_string()).or_insert((0, Instant::now()));
        if entry.1.elapsed() >= Self::WINDOW {
            *entry = (0, Instant::now());
        }
        entry.0 += 1;
    }
}

// ---------- Freundescode (F-57) ----------

async fn get_code(AxumState(state): AxumState<State>, player: Player) -> ApiResult<Json<Value>> {
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT friend_code FROM players WHERE uuid = ?")
            .bind(&player.uuid)
            .fetch_optional(&state.db)
            .await?;
    match row.and_then(|(c,)| c) {
        Some(code) => Ok(Json(serde_json::json!({ "code": code }))),
        None => set_new_code(&state, &player.uuid).await,
    }
}

async fn new_code(AxumState(state): AxumState<State>, player: Player) -> ApiResult<Json<Value>> {
    set_new_code(&state, &player.uuid).await
}

/// Neuer Code; der alte ist sofort ungültig (AK-57.1).
async fn set_new_code(state: &State, uuid: &str) -> ApiResult<Json<Value>> {
    for _ in 0..5 {
        let code = friend_code();
        let r = sqlx::query("UPDATE players SET friend_code = ? WHERE uuid = ?")
            .bind(&code)
            .bind(uuid)
            .execute(&state.db)
            .await;
        match r {
            Ok(_) => return Ok(Json(serde_json::json!({ "code": code }))),
            // Kollision mit dem eindeutigen Index: neu würfeln.
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(ApiError::new(
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        "Kein freier Freundescode gefunden",
    ))
}

/// Spieler zu einem Freundescode (für Anfragen per Code). Fehlversuche werden gezählt.
pub async fn player_by_code(
    state: &State,
    player: &Player,
    code: &str,
) -> ApiResult<(String, String)> {
    if state.code_failures.blocked(&player.uuid) {
        return Err(ApiError::new(
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "Zu viele falsche Codes. Versuch es in einer Stunde wieder.",
        ));
    }
    let found = match normalize_code(code) {
        Some(c) => {
            sqlx::query_as::<_, (String, String)>(
                "SELECT uuid, name FROM players WHERE friend_code = ?",
            )
            .bind(c)
            .fetch_optional(&state.db)
            .await?
        }
        None => None,
    };
    found.ok_or_else(|| {
        state.code_failures.fail(&player.uuid);
        ApiError::new(
            axum::http::StatusCode::NOT_FOUND,
            "Diesen Freundescode gibt es nicht",
        )
    })
}

// ---------- Einladungen (F-56) ----------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InviteRequest {
    to: String,
    /// Serveradresse zum Beitreten.
    server: String,
    /// Teilen-Link des Packs (`pinecone://…`), falls die Instanz aus einer Quelle stammt.
    #[serde(default)]
    pack: Option<String>,
    /// Name der Instanz des Einladenden (nur zur Anzeige).
    #[serde(default)]
    instance: Option<String>,
}

fn clip(s: &str, max: usize) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect::<String>()
        .trim()
        .to_string()
}

async fn invite(
    AxumState(state): AxumState<State>,
    player: Player,
    JsonBody(req): JsonBody<InviteRequest>,
) -> ApiResult<(axum::http::StatusCode, Json<Value>)> {
    let to = crate::friends::normalize_uuid(&req.to)
        .ok_or_else(|| ApiError::bad_request("Ungültiger Spieler"))?;
    let server = clip(&req.server, 255);
    if server.is_empty() {
        return Err(ApiError::bad_request("Kein Server zum Beitreten"));
    }
    let pack = req
        .pack
        .map(|p| clip(&p, 255))
        .filter(|p| p.starts_with("pinecone://"));
    let instance = req.instance.map(|i| clip(&i, 64)).filter(|i| !i.is_empty());
    if !crate::friends::are_friends(&state, &player.uuid, &to).await? {
        return Err(ApiError::new(
            axum::http::StatusCode::FORBIDDEN,
            "Einladen geht nur bei Freunden",
        ));
    }
    let t = now();
    let (recent,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM invites WHERE from_uuid = ? AND created_at > ?")
            .bind(&player.uuid)
            .bind(t - 60)
            .fetch_one(&state.db)
            .await?;
    if recent >= INVITES_PER_MINUTE {
        return Err(ApiError::new(
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "Zu viele Einladungen. Warte kurz.",
        ));
    }
    let id = random_code(16).to_lowercase();
    let mut tx = state.db.begin().await?;
    // Nur die neueste Einladung je Absender und Empfänger zählt. Die alte bleibt als „ersetzt“
    // stehen, damit sie für die Ratenbegrenzung mitzählt.
    sqlx::query(
        "UPDATE invites SET status = 'replaced' WHERE from_uuid = ? AND to_uuid = ? AND status = 'open'",
    )
        .bind(&player.uuid)
        .bind(&to)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO invites (id, from_uuid, to_uuid, server, pack, instance, created_at, expires_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&player.uuid)
    .bind(&to)
    .bind(&server)
    .bind(&pack)
    .bind(&instance)
    .bind(t)
    .bind(t + INVITE_TTL)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((
        axum::http::StatusCode::CREATED,
        Json(serde_json::json!({ "id": id, "expiresAt": t + INVITE_TTL })),
    ))
}

#[derive(Deserialize)]
struct AnswerRequest {
    accept: bool,
}

async fn answer(
    AxumState(state): AxumState<State>,
    player: Player,
    Path(id): Path<String>,
    JsonBody(req): JsonBody<AnswerRequest>,
) -> ApiResult<Json<Value>> {
    let r = sqlx::query(
        "UPDATE invites SET status = ? WHERE id = ? AND to_uuid = ? AND status = 'open' AND expires_at > ?",
    )
    .bind(if req.accept { "accepted" } else { "declined" })
    .bind(&id)
    .bind(&player.uuid)
    .bind(now())
    .execute(&state.db)
    .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::new(
            axum::http::StatusCode::NOT_FOUND,
            "Die Einladung ist abgelaufen",
        ));
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Einladung in `GET /friends`.
#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct InviteDto {
    pub id: String,
    /// Absender (eingehend) bzw. Empfänger (ausgehend).
    pub uuid: String,
    pub name: String,
    pub server: String,
    pub pack: Option<String>,
    pub instance: Option<String>,
    pub status: String,
    pub expires_at: i64,
}

/// Offene eingehende Einladungen und eigene der letzten 15 Minuten (mit Antwort).
pub async fn invites_for(
    state: &State,
    uuid: &str,
) -> sqlx::Result<(Vec<InviteDto>, Vec<InviteDto>)> {
    let t = now();
    let incoming = sqlx::query_as::<_, InviteDto>(
        "SELECT i.id, p.uuid, p.name, i.server, i.pack, i.instance, i.status, i.expires_at
         FROM invites i JOIN players p ON p.uuid = i.from_uuid
         WHERE i.to_uuid = ? AND i.status = 'open' AND i.expires_at > ? ORDER BY i.created_at DESC",
    )
    .bind(uuid)
    .bind(t)
    .fetch_all(&state.db)
    .await?;
    let sent = sqlx::query_as::<_, InviteDto>(
        "SELECT i.id, p.uuid, p.name, i.server, i.pack, i.instance, i.status, i.expires_at
         FROM invites i JOIN players p ON p.uuid = i.to_uuid
         WHERE i.from_uuid = ? AND i.expires_at > ? AND i.status != 'replaced'
         ORDER BY i.created_at DESC",
    )
    .bind(uuid)
    .bind(t)
    .fetch_all(&state.db)
    .await?;
    Ok((incoming, sent))
}

// ---------- Geteilte Sync-Packs (F-58) ----------

#[derive(Deserialize)]
struct ShareRequest {
    /// Sync-Pack ohne eigene Dateien (Projekte, Optionen, Tasten). Der Launcher prüft beim
    /// Öffnen selbst, was er übernimmt.
    data: Value,
}

async fn share(
    AxumState(state): AxumState<State>,
    player: Player,
    JsonBody(req): JsonBody<ShareRequest>,
) -> ApiResult<(axum::http::StatusCode, Json<Value>)> {
    let data = serde_json::to_string(&req.data).unwrap_or_default();
    if !req.data.is_object() || data.len() > MAX_SHARE_BYTES {
        return Err(ApiError::bad_request(
            "Sync-Pack ungültig oder zu groß (höchstens 64 KB)",
        ));
    }
    let t = now();
    sqlx::query("DELETE FROM sync_shares WHERE expires_at <= ?")
        .bind(t)
        .execute(&state.db)
        .await?;
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM sync_shares WHERE owner_uuid = ?")
        .bind(&player.uuid)
        .fetch_one(&state.db)
        .await?;
    if count >= MAX_SHARES {
        return Err(ApiError::bad_request(
            "Du hast schon 20 Sync-Packs geteilt. Ältere laufen nach 30 Tagen ab.",
        ));
    }
    let code = random_code(10);
    sqlx::query(
        "INSERT INTO sync_shares (code, owner_uuid, data, created_at, expires_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&code)
    .bind(&player.uuid)
    .bind(&data)
    .bind(t)
    .bind(t + SHARE_TTL)
    .execute(&state.db)
    .await?;
    Ok((
        axum::http::StatusCode::CREATED,
        Json(serde_json::json!({ "code": code, "expiresAt": t + SHARE_TTL })),
    ))
}

async fn get_share(
    AxumState(state): AxumState<State>,
    _player: Player,
    Path(code): Path<String>,
) -> ApiResult<Json<Value>> {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT s.data, p.name FROM sync_shares s JOIN players p ON p.uuid = s.owner_uuid
         WHERE s.code = ? AND s.expires_at > ?",
    )
    .bind(code.trim().to_ascii_uppercase())
    .bind(now())
    .fetch_optional(&state.db)
    .await?;
    let (data, owner) = row.ok_or_else(|| {
        ApiError::new(
            axum::http::StatusCode::NOT_FOUND,
            "Dieser Link ist abgelaufen oder ungültig",
        )
    })?;
    let data: Value = serde_json::from_str(&data).unwrap_or(Value::Null);
    Ok(Json(serde_json::json!({ "data": data, "owner": owner })))
}

/// Abgelaufene Einladungen und Freigaben entfernen (läuft mit dem täglichen Aufräumen).
pub async fn cleanup(state: &State) -> sqlx::Result<()> {
    let t = now();
    sqlx::query("DELETE FROM invites WHERE expires_at <= ?")
        .bind(t)
        .execute(&state.db)
        .await?;
    sqlx::query("DELETE FROM sync_shares WHERE expires_at <= ?")
        .bind(t)
        .execute(&state.db)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_normalized() {
        let c = friend_code();
        assert_eq!(c.len(), 12);
        assert_eq!(normalize_code(&c).as_deref(), Some(c.as_str()));
        assert_eq!(
            normalize_code(&c.to_lowercase().replace('-', " ")).as_deref(),
            Some(c.as_str())
        );
        assert_eq!(
            normalize_code(&format!("pinecone://friend/{c}")).as_deref(),
            Some(c.as_str())
        );
        assert_eq!(
            normalize_code("ABCD-EFGH-I0"),
            None,
            "0 und I gibt es nicht"
        );
        assert_eq!(normalize_code("zu-kurz"), None);
    }

    #[test]
    fn failures_block_after_limit() {
        let f = CodeFailures::default();
        for _ in 0..CODE_FAILURES_PER_HOUR {
            assert!(!f.blocked("a"));
            f.fail("a");
        }
        assert!(f.blocked("a"));
        assert!(!f.blocked("b"));
    }
}
