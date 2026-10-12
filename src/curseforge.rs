//! CurseForge-Proxy für den Launcher (`/api/v1/curseforge/v1/...`).
//!
//! Die CurseForge-API-Bedingungen erlauben keine Weitergabe des API-Schlüssels. Darum steckt er
//! nicht im Launcher, sondern nur hier in `PINECONE_CURSEFORGE_API_KEY`; der Launcher fragt über
//! diesen Server. Weitergeleitet werden nur die Abfragen, die der Launcher braucht, nur für
//! Minecraft, und nichts wird zwischengespeichert (die Bedingungen verbieten auch das Cachen).

use std::net::SocketAddr;
use std::time::Duration;

use axum::extract::{ConnectInfo, Path, RawQuery, State as AxumState};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::Deserialize;

use crate::api::{client_ip, is_launcher};
use crate::{ApiError, ApiResult, JsonBody, State};

pub const DEFAULT_API: &str = "https://api.curseforge.com";
/// Spiel-ID von Minecraft bei CurseForge.
const MINECRAFT: &str = "432";
/// Anfragen je IP und Minute (Suche, Dateilisten, Abgleich beim Modpack-Import).
pub const REQUESTS_PER_MINUTE: u32 = 240;
pub const WINDOW: Duration = Duration::from_secs(60);
/// Höchstens so viele IDs in einer Sammelabfrage.
const MAX_IDS: usize = 500;

/// Erlaubte Query-Parameter je Abfrage; alles andere wird verworfen.
const SEARCH_PARAMS: &[&str] = &[
    "classId",
    "categoryId",
    "gameVersion",
    "searchFilter",
    "sortField",
    "sortOrder",
    "modLoaderType",
    "index",
    "pageSize",
];
const FILES_PARAMS: &[&str] = &["gameVersion", "modLoaderType", "index", "pageSize"];

pub fn routes() -> axum::Router<State> {
    axum::Router::new()
        .route("/curseforge/v1/mods/search", get(search))
        .route("/curseforge/v1/mods/{id}/files", get(mod_files))
        .route("/curseforge/v1/mods/files", post(files_by_ids))
        .route("/curseforge/v1/mods", post(mods_by_ids))
}

/// Gemeinsame Prüfungen: Launcher-Kennung, Schlüssel eingerichtet, Ratenbegrenzung.
fn admit<'a>(state: &'a State, headers: &HeaderMap, peer: SocketAddr) -> ApiResult<&'a str> {
    if !is_launcher(headers) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "Nur für den TwoGG-Pinecone-Launcher",
        ));
    }
    let key = state.config.curseforge_key.as_deref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "CurseForge ist auf diesem Server nicht eingerichtet",
        )
    })?;
    if !state.cf_limiter.allow(client_ip(headers, peer)) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "Zu viele CurseForge-Anfragen, bitte kurz warten",
        ));
    }
    Ok(key)
}

/// Nur erlaubte Parameter übernehmen; `extra` wird vorangestellt (z. B. die feste Spiel-ID).
fn filter_query(
    raw: Option<&str>,
    allowed: &[&str],
    extra: &[(&str, &str)],
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = extra
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    for pair in raw.unwrap_or_default().split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode(k);
        if allowed.contains(&key.as_str()) && !out.iter().any(|(e, _)| *e == key) {
            out.push((key, percent_decode(v)));
        }
    }
    out
}

fn percent_decode(s: &str) -> String {
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                (Some(h), Some(l)) => {
                    out.push(h * 16 + l);
                    i += 2;
                }
                _ => out.push(b'%'),
            },
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Antwort von CurseForge 1:1 (Status und JSON) weiterreichen, ohne Zwischenspeicher.
async fn forward(req: reqwest::RequestBuilder, key: &str) -> ApiResult<Response> {
    let resp = req
        .header("x-api-key", key)
        .header(header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, format!("CurseForge: {e}")))?;
    let status = resp.status();
    if status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::UNAUTHORIZED {
        // Schlüssel abgelehnt: das ist ein Serverproblem, nicht eins des Launchers.
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "CurseForge lehnt den Server-Schlüssel ab",
        ));
    }
    let body = resp
        .bytes()
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, format!("CurseForge: {e}")))?;
    let status = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    Ok((
        status,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body,
    )
        .into_response())
}

fn url(state: &State, path: &str) -> String {
    format!("{}{path}", state.config.curseforge_api)
}

async fn search(
    AxumState(state): AxumState<State>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> ApiResult<Response> {
    let key = admit(&state, &headers, peer)?;
    // Immer Minecraft: der Schlüssel ist nicht für andere Spiele da.
    let query = filter_query(raw.as_deref(), SEARCH_PARAMS, &[("gameId", MINECRAFT)]);
    forward(
        state.http.get(url(&state, "/v1/mods/search")).query(&query),
        key,
    )
    .await
}

async fn mod_files(
    AxumState(state): AxumState<State>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<u64>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Response> {
    let key = admit(&state, &headers, peer)?;
    let query = filter_query(raw.as_deref(), FILES_PARAMS, &[]);
    forward(
        state
            .http
            .get(url(&state, &format!("/v1/mods/{id}/files")))
            .query(&query),
        key,
    )
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileIds {
    file_ids: Vec<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModIds {
    mod_ids: Vec<u64>,
}

fn check_ids(ids: &[u64]) -> ApiResult<()> {
    if ids.len() > MAX_IDS {
        return Err(ApiError::bad_request(format!(
            "Höchstens {MAX_IDS} IDs je Abfrage"
        )));
    }
    Ok(())
}

async fn files_by_ids(
    AxumState(state): AxumState<State>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    JsonBody(body): JsonBody<FileIds>,
) -> ApiResult<Response> {
    let key = admit(&state, &headers, peer)?;
    check_ids(&body.file_ids)?;
    let json = serde_json::json!({ "fileIds": body.file_ids });
    forward(
        state.http.post(url(&state, "/v1/mods/files")).json(&json),
        key,
    )
    .await
}

async fn mods_by_ids(
    AxumState(state): AxumState<State>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    JsonBody(body): JsonBody<ModIds>,
) -> ApiResult<Response> {
    let key = admit(&state, &headers, peer)?;
    check_ids(&body.mod_ids)?;
    let json = serde_json::json!({ "modIds": body.mod_ids });
    forward(state.http.post(url(&state, "/v1/mods")).json(&json), key).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_keeps_only_allowed_params_and_fixes_game() {
        let q = filter_query(
            Some("gameId=999&searchFilter=sodium%20extra&pageSize=20&apiKey=x&classId=6"),
            SEARCH_PARAMS,
            &[("gameId", MINECRAFT)],
        );
        assert_eq!(
            q,
            [
                ("gameId".to_string(), "432".to_string()),
                ("searchFilter".to_string(), "sodium extra".to_string()),
                ("pageSize".to_string(), "20".to_string()),
                ("classId".to_string(), "6".to_string()),
            ]
        );
    }

    #[test]
    fn decodes_percent_and_plus() {
        assert_eq!(percent_decode("a+b%C3%A4%"), "a bä%");
    }
}
