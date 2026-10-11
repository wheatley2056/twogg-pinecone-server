//! Öffentliche API für den Launcher (`/api/v1`) und die Downloads (`/dl/<token>`).
//!
//! Modpack-Dateien sind nicht öffentlich verlinkt: Pinecone fordert pro Download einen
//! kurzlebigen, signierten Link an (mit `X-Pinecone-Client`-Kennung und Ratenbegrenzung).
//! Das hält Gelegenheits-Downloads außerhalb des Launchers fern; echter Kopierschutz ist es nicht.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::{ConnectInfo, Path, Query, State as AxumState};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};

use crate::db::{self, News, Pack, Version};
use crate::files::{self, media_url};
use crate::markdown;
use crate::{ApiError, ApiResult, State};

/// Download-Links je IP und Zeitfenster.
const LINKS_PER_WINDOW: u32 = 30;
const WINDOW: Duration = Duration::from_secs(600);

pub fn routes() -> axum::Router<State> {
    axum::Router::new()
        .route("/info", get(info))
        .route("/news", get(news))
        .route("/packs", get(packs))
        .route("/packs/{id}", get(pack))
        .route(
            "/packs/{id}/versions/{version}/download",
            post(download_link),
        )
}

pub async fn index() -> &'static str {
    "TwoGG Pinecone Server. Die Verwaltung liegt unter /admin."
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Info {
    name: &'static str,
    version: &'static str,
    packs: i64,
    /// Community-Server für den Schnellbeitritt im Launcher (in der Admin-Seite gepflegt).
    game_server: Option<GameServer>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GameServer {
    name: String,
    address: String,
}

async fn info(AxumState(state): AxumState<State>) -> ApiResult<Json<Info>> {
    let (packs,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM packs WHERE visibility = 'public'")
        .fetch_one(&state.db)
        .await?;
    let settings = crate::settings::load(&state).await?;
    let game_server = (!settings.game_server_address.is_empty()).then(|| GameServer {
        name: if settings.game_server_name.is_empty() {
            "TwoGG Server".into()
        } else {
            settings.game_server_name
        },
        address: settings.game_server_address,
    });
    Ok(Json(Info {
        name: "TwoGG Pinecone",
        version: env!("CARGO_PKG_VERSION"),
        packs,
        game_server,
    }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewsDto {
    id: String,
    title: String,
    body_html: String,
    image_url: Option<String>,
    link: Option<String>,
    pinned: bool,
    published_at: i64,
}

#[derive(Deserialize)]
struct NewsQuery {
    limit: Option<i64>,
}

async fn news(
    AxumState(state): AxumState<State>,
    Query(q): Query<NewsQuery>,
) -> ApiResult<Json<Vec<NewsDto>>> {
    let limit = q.limit.unwrap_or(10).clamp(1, 50);
    let rows: Vec<News> = sqlx::query_as(
        "SELECT * FROM news WHERE visible = 1 AND published_at <= ? \
         ORDER BY pinned DESC, published_at DESC LIMIT ?",
    )
    .bind(db::now())
    .bind(limit)
    .fetch_all(&state.db)
    .await?;
    let base = &state.config.public_url;
    Ok(Json(
        rows.into_iter()
            .map(|n| NewsDto {
                body_html: markdown::to_html(&n.body),
                image_url: n.image.as_deref().map(|i| media_url(base, i)),
                id: n.id,
                title: n.title,
                link: n.link,
                pinned: n.pinned,
                published_at: n.published_at,
            })
            .collect(),
    ))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionDto {
    id: String,
    version: String,
    minecraft: String,
    loader: Option<String>,
    loader_version: Option<String>,
    channel: String,
    changelog_html: String,
    size: i64,
    sha1: String,
    published_at: i64,
}

impl From<Version> for VersionDto {
    fn from(v: Version) -> Self {
        Self {
            changelog_html: markdown::to_html(&v.changelog),
            id: v.id,
            version: v.version,
            minecraft: v.minecraft,
            loader: v.loader,
            loader_version: v.loader_version,
            channel: v.channel,
            size: v.size,
            sha1: v.sha1,
            published_at: v.published_at,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackDto {
    id: String,
    title: String,
    summary: String,
    icon_url: Option<String>,
    featured: bool,
    updated_at: i64,
    downloads: i64,
    latest: Option<VersionDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description_html: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    versions: Option<Vec<VersionDto>>,
}

fn pack_dto(base: &str, p: Pack, versions: Vec<Version>, full: bool) -> PackDto {
    let downloads = versions.iter().map(|v| v.downloads).sum();
    // „Neueste“ = neueste Release-Version, sonst die neueste überhaupt.
    let latest = versions
        .iter()
        .find(|v| v.channel == "release")
        .or(versions.first())
        .cloned()
        .map(VersionDto::from);
    PackDto {
        icon_url: p.icon.as_deref().map(|i| media_url(base, i)),
        description_html: full.then(|| markdown::to_html(&p.description)),
        versions: full.then(|| versions.into_iter().map(VersionDto::from).collect()),
        id: p.id,
        title: p.title,
        summary: p.summary,
        featured: p.featured,
        updated_at: p.updated_at,
        downloads,
        latest,
    }
}

async fn packs(AxumState(state): AxumState<State>) -> ApiResult<Json<Vec<PackDto>>> {
    let rows: Vec<Pack> = sqlx::query_as(
        "SELECT * FROM packs WHERE visibility = 'public' \
         ORDER BY featured DESC, sort_order ASC, updated_at DESC",
    )
    .fetch_all(&state.db)
    .await?;
    let mut out = Vec::new();
    for p in rows {
        let versions = db::versions_of(&state.db, &p.id).await?;
        // Packs ohne Version sind (noch) nicht installierbar.
        if !versions.is_empty() {
            out.push(pack_dto(&state.config.public_url, p, versions, false));
        }
    }
    Ok(Json(out))
}

async fn pack(
    AxumState(state): AxumState<State>,
    Path(id): Path<String>,
) -> ApiResult<Json<PackDto>> {
    let p: Pack = sqlx::query_as("SELECT * FROM packs WHERE id = ? AND visibility != 'hidden'")
        .bind(&id)
        .fetch_one(&state.db)
        .await?;
    let versions = db::versions_of(&state.db, &p.id).await?;
    Ok(Json(pack_dto(&state.config.public_url, p, versions, true)))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadLink {
    url: String,
    expires: i64,
    size: i64,
    sha1: String,
    sha512: String,
    file_name: String,
}

/// Echte Client-IP hinter Cloudflare/Nginx Proxy Manager.
fn client_ip(headers: &HeaderMap, peer: SocketAddr) -> IpAddr {
    let header_ip = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .and_then(|v| v.trim().parse().ok())
    };
    header_ip("cf-connecting-ip")
        .or_else(|| header_ip("x-forwarded-for"))
        .or_else(|| header_ip("x-real-ip"))
        .unwrap_or(peer.ip())
}

async fn download_link(
    AxumState(state): AxumState<State>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path((id, version)): Path<(String, String)>,
) -> ApiResult<Json<DownloadLink>> {
    let client = headers
        .get("x-pinecone-client")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if !client.starts_with("pinecone/") {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "Dieses Modpack gibt es nur im TwoGG-Pinecone-Launcher",
        ));
    }
    if !state.limiter.allow(client_ip(&headers, peer)) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "Zu viele Downloads, bitte später erneut versuchen",
        ));
    }
    let v: Version = sqlx::query_as(
        "SELECT v.* FROM versions v JOIN packs p ON p.id = v.pack_id \
         WHERE v.id = ? AND v.pack_id = ? AND p.visibility != 'hidden'",
    )
    .bind(&version)
    .bind(&id)
    .fetch_one(&state.db)
    .await?;
    let expires = db::now() + state.config.download_ttl as i64;
    let token = files::sign_download(&state.secret, &v.id, expires);
    Ok(Json(DownloadLink {
        url: format!("{}/dl/{token}", state.config.public_url),
        expires,
        size: v.size,
        sha1: v.sha1,
        sha512: v.sha512,
        file_name: format!("{}-{}.mrpack", v.pack_id, v.version),
    }))
}

pub async fn download(
    AxumState(state): AxumState<State>,
    Path(token): Path<String>,
) -> ApiResult<Response> {
    let id = files::verify_download(&state.secret, &token, db::now()).ok_or_else(|| {
        ApiError::new(
            StatusCode::FORBIDDEN,
            "Download-Link ungültig oder abgelaufen",
        )
    })?;
    let v: Version = sqlx::query_as("SELECT * FROM versions WHERE id = ?")
        .bind(&id)
        .fetch_one(&state.db)
        .await?;
    let path = files::packs_dir(&state.config.data_dir).join(&v.file);
    let file = tokio::fs::File::open(&path).await?;
    sqlx::query("UPDATE versions SET downloads = downloads + 1 WHERE id = ?")
        .bind(&v.id)
        .execute(&state.db)
        .await?;
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

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || ".-_+".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Einfache Ratenbegrenzung im Speicher: höchstens `LINKS_PER_WINDOW` Links je IP und Fenster.
#[derive(Default)]
pub struct RateLimiter(Mutex<HashMap<IpAddr, (u32, Instant)>>);

impl RateLimiter {
    pub fn allow(&self, ip: IpAddr) -> bool {
        let mut map = self.0.lock().unwrap();
        let now = Instant::now();
        if map.len() > 10_000 {
            map.retain(|_, (_, start)| now.duration_since(*start) < WINDOW);
        }
        let entry = map.entry(ip).or_insert((0, now));
        if now.duration_since(entry.1) >= WINDOW {
            *entry = (0, now);
        }
        entry.0 += 1;
        entry.0 <= LINKS_PER_WINDOW
    }
}
