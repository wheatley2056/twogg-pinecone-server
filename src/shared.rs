//! Von Spielern geteilte Modpacks zum Abonnieren (`pinecone://pack/<id>`).
//!
//! Ein Spieler veröffentlicht eine Instanz als `.mrpack` (wie beim Datei-Export, also auch mit
//! eigenen Dateien). Wer den Link hat und angemeldet ist, kann das Pack installieren; neue
//! Versionen sieht der Launcher als „Update verfügbar“. Nur der Ersteller darf neue Versionen
//! hochladen und das Pack löschen. Alles hängt am Spieler und verschwindet mit „Meine
//! Freunde-Daten löschen“ (`ON DELETE CASCADE`, Dateien beim täglichen Aufräumen).

use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::PathBuf;

use axum::extract::{ConnectInfo, DefaultBodyLimit, Multipart, Path, State as AxumState};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use sqlx::FromRow;

use crate::api::{DownloadLink, client_ip, is_launcher, sanitize};
use crate::db::now;
use crate::files;
use crate::friends::Player;
use crate::social::random_code;
use crate::{ApiError, ApiResult, State};

/// Größte `.mrpack`-Datei je Version.
pub const MAX_PACK_BYTES: usize = 100 * 1024 * 1024;
/// Höchstens so viele geteilte Packs je Spieler.
pub const MAX_PACKS: i64 = 5;
/// Ältere Versionen über diese Zahl hinaus werden gelöscht.
pub const KEEP_VERSIONS: usize = 3;
/// Ohne neue Version läuft ein Pack nach so vielen Tagen ab.
pub const PACK_TTL: i64 = 180 * 86_400;
/// Präfix in Download-Tokens, damit sie nicht für TwoGG-Packs gelten.
const TOKEN_PREFIX: &str = "shared-";

pub fn routes() -> Router<State> {
    Router::new()
        .route("/shared-packs", get(mine).post(create))
        .route("/shared-packs/{id}", get(get_pack).delete(delete_pack))
        .route("/shared-packs/{id}/versions", post(add_version))
        .route(
            "/shared-packs/{id}/versions/{version}/download",
            post(download_link),
        )
        .layer(DefaultBodyLimit::max(MAX_PACK_BYTES + 64 * 1024))
}

pub fn dir(state: &State) -> PathBuf {
    state.config.data_dir.join("shared-packs")
}

#[derive(FromRow)]
struct PackRow {
    id: String,
    owner_uuid: String,
    owner_name: String,
    name: String,
    created_at: i64,
    updated_at: i64,
}

#[derive(Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
struct VersionRow {
    id: String,
    #[serde(skip)]
    pack_id: String,
    version: String,
    minecraft: String,
    loader: Option<String>,
    loader_version: Option<String>,
    changelog: String,
    #[serde(skip)]
    file: String,
    size: i64,
    sha1: String,
    sha512: String,
    published_at: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SharedPack {
    id: String,
    name: String,
    /// Spielername des Erstellers.
    owner: String,
    /// Der angemeldete Spieler hat das Pack erstellt.
    mine: bool,
    link: String,
    created_at: i64,
    updated_at: i64,
    expires_at: i64,
    /// Neueste zuerst; höchstens `KEEP_VERSIONS`.
    versions: Vec<VersionRow>,
}

fn not_found() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "Dieses Pack gibt es nicht mehr oder der Link ist falsch",
    )
}

/// Pack-IDs aus Links: Großbuchstaben und Ziffern aus dem Code-Alphabet.
fn normalize_id(id: &str) -> Option<String> {
    let id = id.trim().to_ascii_uppercase();
    (id.len() == 10 && id.bytes().all(|b| crate::social::ALPHABET.contains(&b))).then_some(id)
}

async fn load_pack(state: &State, id: &str) -> ApiResult<PackRow> {
    let id = normalize_id(id).ok_or_else(not_found)?;
    sqlx::query_as(
        "SELECT s.id, s.owner_uuid, p.name AS owner_name, s.name, s.created_at, s.updated_at
         FROM shared_packs s JOIN players p ON p.uuid = s.owner_uuid
         WHERE s.id = ? AND s.updated_at > ?",
    )
    .bind(id)
    .bind(now() - PACK_TTL)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(not_found)
}

async fn versions_of(state: &State, pack_id: &str) -> ApiResult<Vec<VersionRow>> {
    Ok(sqlx::query_as(
        "SELECT * FROM shared_pack_versions WHERE pack_id = ? ORDER BY published_at DESC, rowid DESC",
    )
    .bind(pack_id)
    .fetch_all(&state.db)
    .await?)
}

async fn describe(state: &State, pack: PackRow, player: &Player) -> ApiResult<SharedPack> {
    let versions = versions_of(state, &pack.id).await?;
    Ok(SharedPack {
        link: format!("pinecone://pack/{}", pack.id),
        mine: pack.owner_uuid == player.uuid,
        expires_at: pack.updated_at + PACK_TTL,
        id: pack.id,
        name: pack.name,
        owner: pack.owner_name,
        created_at: pack.created_at,
        updated_at: pack.updated_at,
        versions,
    })
}

/// Eigene geteilte Packs.
async fn mine(
    AxumState(state): AxumState<State>,
    player: Player,
) -> ApiResult<Json<Vec<SharedPack>>> {
    let rows: Vec<PackRow> = sqlx::query_as(
        "SELECT s.id, s.owner_uuid, p.name AS owner_name, s.name, s.created_at, s.updated_at
         FROM shared_packs s JOIN players p ON p.uuid = s.owner_uuid
         WHERE s.owner_uuid = ? AND s.updated_at > ? ORDER BY s.updated_at DESC",
    )
    .bind(&player.uuid)
    .bind(now() - PACK_TTL)
    .fetch_all(&state.db)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(describe(&state, row, &player).await?);
    }
    Ok(Json(out))
}

async fn get_pack(
    AxumState(state): AxumState<State>,
    player: Player,
    Path(id): Path<String>,
) -> ApiResult<Json<SharedPack>> {
    let pack = load_pack(&state, &id).await?;
    Ok(Json(describe(&state, pack, &player).await?))
}

struct Upload {
    name: Option<String>,
    changelog: String,
    stored: files::StoredFile,
    parsed: crate::mrpack::Mrpack,
    file_name: String,
}

/// Multipart mit `file` (.mrpack), optional `name` und `changelog`; prüft das Pack wie bei TwoGG-Packs.
async fn read_upload(state: &State, mut multipart: Multipart) -> ApiResult<Upload> {
    let (mut name, mut changelog) = (None::<String>, String::new());
    let id = uuid::Uuid::new_v4().simple().to_string();
    let file_name = format!("{id}.mrpack");
    let dir = dir(state);
    let mut stored = None;
    let next = |e: axum::extract::multipart::MultipartError| {
        ApiError::bad_request(format!("Upload fehlerhaft: {e}"))
    };
    let result: ApiResult<()> = async {
        while let Some(field) = multipart.next_field().await.map_err(next)? {
            match field.name().unwrap_or_default() {
                "file" => {
                    stored =
                        Some(files::store_field(field, &dir, &file_name, MAX_PACK_BYTES).await?);
                }
                field_name => {
                    let field_name = field_name.to_string();
                    let text = field.text().await.map_err(next)?;
                    match field_name.as_str() {
                        "name" => name = Some(text.trim().to_string()).filter(|t| !t.is_empty()),
                        "changelog" => changelog = text.trim().to_string(),
                        _ => {}
                    }
                }
            }
        }
        Ok(())
    }
    .await;
    let remove = |stored: &Option<files::StoredFile>| {
        if let Some(s) = stored {
            let _ = std::fs::remove_file(&s.path);
        }
    };
    if let Err(e) = result {
        remove(&stored);
        if e.status == StatusCode::BAD_REQUEST && e.message == "Datei ist zu groß" {
            return Err(ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "Das Pack ist zu groß (höchstens 100 MB)",
            ));
        }
        return Err(e);
    }
    let check = || -> Result<(), ApiError> {
        if name.as_ref().is_some_and(|n| n.chars().count() > 64) {
            return Err(ApiError::bad_request(
                "Der Name ist zu lang (höchstens 64 Zeichen)",
            ));
        }
        if changelog.chars().count() > 4000 {
            return Err(ApiError::bad_request(
                "Die Änderungen sind zu lang (höchstens 4000 Zeichen)",
            ));
        }
        Ok(())
    };
    if let Err(e) = check() {
        remove(&stored);
        return Err(e);
    }
    let stored = stored.ok_or_else(|| ApiError::bad_request("Die .mrpack-Datei fehlt"))?;
    let path = stored.path.clone();
    let parsed = tokio::task::spawn_blocking(move || crate::mrpack::read(&path))
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let parsed = match parsed {
        Ok(p) if !p.version.trim().is_empty() && p.version.chars().count() <= 64 => p,
        Ok(_) => {
            remove(&Some(stored));
            return Err(ApiError::bad_request(
                "Das Modpack braucht eine Versionsangabe (höchstens 64 Zeichen)",
            ));
        }
        Err(e) => {
            remove(&Some(stored));
            return Err(ApiError::bad_request(format!("Kein gültiges Modpack: {e}")));
        }
    };
    Ok(Upload {
        name,
        changelog,
        stored,
        parsed,
        file_name,
    })
}

async fn insert_version(state: &State, pack_id: &str, up: &Upload) -> ApiResult<String> {
    let id = up.file_name.trim_end_matches(".mrpack").to_string();
    let (loader, loader_version) = up.parsed.loader.clone().unzip();
    let r = sqlx::query(
        "INSERT INTO shared_pack_versions (id, pack_id, version, minecraft, loader, loader_version, changelog, file, size, sha1, sha512, published_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(pack_id)
    .bind(up.parsed.version.trim())
    .bind(&up.parsed.minecraft)
    .bind(loader)
    .bind(loader_version)
    .bind(&up.changelog)
    .bind(&up.file_name)
    .bind(up.stored.size as i64)
    .bind(&up.stored.sha1)
    .bind(&up.stored.sha512)
    .bind(now())
    .execute(&state.db)
    .await;
    match r {
        Ok(_) => Ok(id),
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => Err(ApiError::new(
            StatusCode::CONFLICT,
            format!(
                "Version {} gibt es schon. Gib der neuen Version eine andere Nummer.",
                up.parsed.version.trim()
            ),
        )),
        Err(e) => Err(e.into()),
    }
}

/// Ältere Versionen über `KEEP_VERSIONS` hinaus samt Dateien entfernen.
async fn prune(state: &State, pack_id: &str) -> ApiResult<()> {
    let versions = versions_of(state, pack_id).await?;
    for old in versions.into_iter().skip(KEEP_VERSIONS) {
        sqlx::query("DELETE FROM shared_pack_versions WHERE id = ?")
            .bind(&old.id)
            .execute(&state.db)
            .await?;
        let _ = tokio::fs::remove_file(dir(state).join(&old.file)).await;
    }
    Ok(())
}

/// Neues geteiltes Pack mit erster Version.
async fn create(
    AxumState(state): AxumState<State>,
    player: Player,
    multipart: Multipart,
) -> ApiResult<(StatusCode, Json<SharedPack>)> {
    let (count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM shared_packs WHERE owner_uuid = ? AND updated_at > ?")
            .bind(&player.uuid)
            .bind(now() - PACK_TTL)
            .fetch_one(&state.db)
            .await?;
    if count >= MAX_PACKS {
        return Err(ApiError::bad_request(format!(
            "Du teilst schon {MAX_PACKS} Packs. Lösch eins, bevor du ein neues teilst."
        )));
    }
    let up = read_upload(&state, multipart).await?;
    let name = up.name.clone().unwrap_or_else(|| "Modpack".into());
    let t = now();
    let id = random_code(10);
    let result = async {
        sqlx::query(
            "INSERT INTO shared_packs (id, owner_uuid, name, created_at, updated_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&player.uuid)
        .bind(&name)
        .bind(t)
        .bind(t)
        .execute(&state.db)
        .await?;
        insert_version(&state, &id, &up).await
    }
    .await;
    if let Err(e) = result {
        let _ = tokio::fs::remove_file(&up.stored.path).await;
        let _ = sqlx::query("DELETE FROM shared_packs WHERE id = ?")
            .bind(&id)
            .execute(&state.db)
            .await;
        return Err(e);
    }
    let pack = load_pack(&state, &id).await?;
    Ok((
        StatusCode::CREATED,
        Json(describe(&state, pack, &player).await?),
    ))
}

/// Neue Version veröffentlichen (nur der Ersteller). `name` benennt das Pack auf Wunsch um.
async fn add_version(
    AxumState(state): AxumState<State>,
    player: Player,
    Path(id): Path<String>,
    multipart: Multipart,
) -> ApiResult<(StatusCode, Json<SharedPack>)> {
    let pack = load_pack(&state, &id).await?;
    if pack.owner_uuid != player.uuid {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "Nur wer das Pack geteilt hat, kann neue Versionen veröffentlichen",
        ));
    }
    let up = read_upload(&state, multipart).await?;
    if let Err(e) = insert_version(&state, &pack.id, &up).await {
        let _ = tokio::fs::remove_file(&up.stored.path).await;
        return Err(e);
    }
    sqlx::query("UPDATE shared_packs SET updated_at = ?, name = COALESCE(?, name) WHERE id = ?")
        .bind(now())
        .bind(&up.name)
        .bind(&pack.id)
        .execute(&state.db)
        .await?;
    prune(&state, &pack.id).await?;
    let pack = load_pack(&state, &pack.id).await?;
    Ok((
        StatusCode::CREATED,
        Json(describe(&state, pack, &player).await?),
    ))
}

/// Pack mit allen Versionen löschen (nur der Ersteller). Installierte Instanzen bleiben.
async fn delete_pack(
    AxumState(state): AxumState<State>,
    player: Player,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let pack = load_pack(&state, &id).await?;
    if pack.owner_uuid != player.uuid {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "Nur wer das Pack geteilt hat, kann es löschen",
        ));
    }
    let versions = versions_of(&state, &pack.id).await?;
    sqlx::query("DELETE FROM shared_packs WHERE id = ?")
        .bind(&pack.id)
        .execute(&state.db)
        .await?;
    for v in versions {
        let _ = tokio::fs::remove_file(dir(&state).join(&v.file)).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Signierter, kurzlebiger Download-Link für eine Version (wie bei TwoGG-Packs).
async fn download_link(
    AxumState(state): AxumState<State>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    _player: Player,
    Path((id, version)): Path<(String, String)>,
) -> ApiResult<Json<DownloadLink>> {
    if !is_launcher(&headers) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "Geteilte Packs gibt es nur im TwoGG-Pinecone-Launcher",
        ));
    }
    if !state.limiter.allow(client_ip(&headers, peer)) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "Zu viele Downloads, bitte später erneut versuchen",
        ));
    }
    let pack = load_pack(&state, &id).await?;
    let v: VersionRow =
        sqlx::query_as("SELECT * FROM shared_pack_versions WHERE id = ? AND pack_id = ?")
            .bind(&version)
            .bind(&pack.id)
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::GONE,
                    "Diese Version gibt es nicht mehr, es gibt eine neuere",
                )
            })?;
    let expires = now() + state.config.download_ttl as i64;
    let token = files::sign_download(&state.secret, &format!("{TOKEN_PREFIX}{}", v.id), expires);
    Ok(Json(DownloadLink {
        url: format!("{}/dl/shared/{token}", state.config.public_url),
        expires,
        size: v.size,
        sha1: v.sha1,
        sha512: v.sha512,
        file_name: format!("{}-{}.mrpack", sanitize(&pack.name), sanitize(&v.version)),
    }))
}

pub async fn download(
    AxumState(state): AxumState<State>,
    Path(token): Path<String>,
) -> ApiResult<Response> {
    let forbidden = || {
        ApiError::new(
            StatusCode::FORBIDDEN,
            "Download-Link ungültig oder abgelaufen",
        )
    };
    let id = files::verify_download(&state.secret, &token, now()).ok_or_else(forbidden)?;
    let id = id.strip_prefix(TOKEN_PREFIX).ok_or_else(forbidden)?;
    let v: VersionRow = sqlx::query_as("SELECT * FROM shared_pack_versions WHERE id = ?")
        .bind(id)
        .fetch_one(&state.db)
        .await?;
    let file = tokio::fs::File::open(dir(&state).join(&v.file)).await?;
    let body = axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(file));
    Ok((
        [
            (
                header::CONTENT_TYPE,
                "application/x-modrinth-modpack+zip".to_string(),
            ),
            (header::CONTENT_LENGTH, v.size.to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!(
                    "attachment; filename=\"{}-{}.mrpack\"",
                    v.pack_id,
                    sanitize(&v.version)
                ),
            ),
            (header::CACHE_CONTROL, "private, no-store".to_string()),
        ],
        body,
    )
        .into_response())
}

/// Abgelaufene Packs löschen und Dateien entfernen, zu denen es keine Version mehr gibt
/// (z. B. nach „Meine Freunde-Daten löschen“).
pub async fn cleanup(state: &State) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM shared_packs WHERE updated_at <= ?")
        .bind(now() - PACK_TTL)
        .execute(&state.db)
        .await?;
    let known: HashSet<String> =
        sqlx::query_as::<_, (String,)>("SELECT file FROM shared_pack_versions")
            .fetch_all(&state.db)
            .await?
            .into_iter()
            .map(|(f,)| f)
            .collect();
    let dir = dir(state);
    let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
        return Ok(());
    };
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name().to_string_lossy().into_owned();
        if known.contains(&name) {
            continue;
        }
        // Laufende Uploads (gerade geschrieben) nicht anfassen.
        let recent = entry
            .metadata()
            .await
            .and_then(|m| m.modified())
            .is_ok_and(|m| m > cutoff);
        if !recent {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids() {
        assert_eq!(normalize_id(" ab23cd45ef ").as_deref(), Some("AB23CD45EF"));
        assert_eq!(normalize_id("AB23CD45E"), None, "zu kurz");
        assert_eq!(normalize_id("AB23CD45E0"), None, "0 nicht im Alphabet");
        assert_eq!(normalize_id("../../etc/x"), None);
    }
}
