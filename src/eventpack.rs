//! Signierte Event-Pack-Manifeste (F-10, Plan `04` §6).
//!
//! Der Server prüft beim Hochladen nur, ob das Manifest echt ist und zur Version passt, und reicht
//! es unverändert an den Launcher weiter. Der Launcher prüft die Signatur selbst noch einmal gegen
//! seine eingebauten Schlüssel; die Echtheit hängt also nicht davon ab, dass der Server sicher ist.
//! Format und kanonisches JSON wie in `crates/core/src/eventpack.rs` im Launcher.

use std::collections::BTreeMap;

use axum::Json;
use axum::extract::{ConnectInfo, Path, Query, State as AxumState};
use axum::http::{HeaderMap, StatusCode};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::auth::Admin;
use crate::db::Version;
use crate::{ApiError, ApiResult, JsonBody, State};

pub const SCHEMA: &str = "pinecone-eventpack/1";

/// Öffentliche Schlüssel aus `PINECONE_EVENTPACK_KEYS` (`keyId=Base64`, mehrere durch `;`).
#[derive(Debug, Clone, Default)]
pub struct Keyring {
    keys: BTreeMap<String, VerifyingKey>,
}

impl Keyring {
    pub fn parse(spec: &str) -> anyhow::Result<Self> {
        let mut keys = BTreeMap::new();
        for part in spec.split(';').map(str::trim).filter(|p| !p.is_empty()) {
            let (id, b64) = part
                .split_once('=')
                .ok_or_else(|| anyhow::anyhow!("Schlüssel ohne keyId: {part:?}"))?;
            let bytes: [u8; 32] = B64
                .decode(b64.trim())
                .ok()
                .and_then(|b| b.try_into().ok())
                .ok_or_else(|| anyhow::anyhow!("Schlüssel {id}: 32 Byte in Base64 erwartet"))?;
            keys.insert(id.trim().to_string(), VerifyingKey::from_bytes(&bytes)?);
        }
        Ok(Self { keys })
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// Kanonische Form nach RFC 8785 (Objekte sortiert nach UTF-16, nur ganze Zahlen).
pub fn canonical_json(value: &Value) -> Result<String, String> {
    let mut out = String::new();
    write(value, &mut out)?;
    Ok(out)
}

fn write(value: &Value, out: &mut String) -> Result<(), String> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) if n.is_f64() => {
            return Err("Kommazahlen sind im Manifest nicht erlaubt".into());
        }
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => out.push_str(&serde_json::to_string(s).map_err(|e| e.to_string())?),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write(item, out)?;
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            out.push('{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key).map_err(|e| e.to_string())?);
                out.push(':');
                write(&map[key], out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Loader {
    kind: String,
    version: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MrpackRef {
    size: u64,
    sha512: String,
}

/// Die Felder, die der Server gegen die Version prüft.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestFields {
    schema: String,
    pack_id: String,
    version: String,
    channel: String,
    event_id: String,
    minecraft: String,
    loader: Option<Loader>,
    mrpack: MrpackRef,
    #[allow(dead_code)]
    locked: Vec<Value>,
    key_id: String,
}

/// Geprüftes Manifest, wie es gespeichert wird.
pub struct Checked {
    pub canonical: String,
    pub signature: String,
    pub key_id: String,
}

/// Prüft Signatur, Format und Übereinstimmung mit der Version. Fehlertexte gehen an den Admin.
pub fn check(
    keys: &Keyring,
    manifest: &Value,
    signature: &str,
    v: &Version,
    pack_event: Option<&str>,
) -> Result<Checked, String> {
    if keys.is_empty() {
        return Err(
            "Der Server kennt keine Event-Pack-Schlüssel (PINECONE_EVENTPACK_KEYS fehlt)".into(),
        );
    }
    let fields: ManifestFields = serde_json::from_value(manifest.clone())
        .map_err(|e| format!("Manifest unvollständig: {e}"))?;
    let key = keys
        .keys
        .get(&fields.key_id)
        .ok_or_else(|| format!("Unbekannter Schlüssel {:?}", fields.key_id))?;
    let canonical = canonical_json(manifest)?;
    let sig_bytes = B64
        .decode(signature.trim())
        .map_err(|_| "Signatur ist kein Base64".to_string())?;
    let sig =
        Signature::from_slice(&sig_bytes).map_err(|_| "Signatur hat nicht 64 Byte".to_string())?;
    key.verify_strict(canonical.as_bytes(), &sig).map_err(|_| {
        "Signatur ungültig: Manifest und Signatur passen nicht zusammen".to_string()
    })?;
    if fields.schema != SCHEMA {
        return Err(format!("Unbekanntes Format {:?}", fields.schema));
    }
    let mismatch = |what: &str, m: &str, actual: &str| {
        format!("{what} im Manifest ({m}) passt nicht zur Version ({actual})")
    };
    if fields.pack_id != v.pack_id {
        return Err(mismatch("Pack", &fields.pack_id, &v.pack_id));
    }
    if fields.version != v.version {
        return Err(mismatch("Version", &fields.version, &v.version));
    }
    if fields.channel != v.channel {
        return Err(mismatch("Kanal", &fields.channel, &v.channel));
    }
    if fields.minecraft != v.minecraft {
        return Err(mismatch("Minecraft", &fields.minecraft, &v.minecraft));
    }
    let loader = fields.loader.map(|l| (l.kind, l.version));
    let version_loader = v.loader.clone().zip(v.loader_version.clone());
    if loader != version_loader {
        return Err("Loader im Manifest passt nicht zur Version".into());
    }
    if fields.mrpack.size != v.size as u64 || !fields.mrpack.sha512.eq_ignore_ascii_case(&v.sha512)
    {
        return Err("Größe oder SHA-512 im Manifest passen nicht zur hochgeladenen Datei".into());
    }
    if let Some(event) = pack_event
        && event != fields.event_id
    {
        return Err(format!(
            "Event im Manifest ({}) ist nicht das Event des Packs ({event})",
            fields.event_id
        ));
    }
    Ok(Checked {
        canonical,
        signature: signature.trim().to_string(),
        key_id: fields.key_id,
    })
}

async fn pack_event(state: &State, pack_id: &str) -> ApiResult<Option<String>> {
    let row: Option<(Option<String>,)> = sqlx::query_as("SELECT event_id FROM packs WHERE id = ?")
        .bind(pack_id)
        .fetch_optional(&state.db)
        .await?;
    Ok(row.and_then(|r| r.0).filter(|e| !e.is_empty()))
}

/// Manifest prüfen und an der Version speichern.
pub async fn attach(
    state: &State,
    v: &Version,
    manifest: &Value,
    signature: &str,
) -> ApiResult<()> {
    let event = pack_event(state, &v.pack_id).await?;
    let checked = check(
        &state.config.eventpack_keys,
        manifest,
        signature,
        v,
        event.as_deref(),
    )
    .map_err(ApiError::bad_request)?;
    sqlx::query(
        "UPDATE versions SET manifest = ?, signature = ?, manifest_key_id = ? WHERE id = ?",
    )
    .bind(&checked.canonical)
    .bind(&checked.signature)
    .bind(&checked.key_id)
    .bind(&v.id)
    .execute(&state.db)
    .await?;
    Ok(())
}

#[derive(Deserialize)]
pub struct AttachRequest {
    manifest: Value,
    signature: String,
}

/// `PUT /api/admin/versions/{id}/manifest` `{manifest, signature}`.
pub async fn put(
    AxumState(state): AxumState<State>,
    _: Admin,
    Path(id): Path<String>,
    JsonBody(req): JsonBody<AttachRequest>,
) -> ApiResult<Json<Version>> {
    let v = version(&state, &id).await?;
    attach(&state, &v, &req.manifest, &req.signature).await?;
    Ok(Json(version(&state, &id).await?))
}

/// `DELETE /api/admin/versions/{id}/manifest`.
pub async fn delete(
    AxumState(state): AxumState<State>,
    _: Admin,
    Path(id): Path<String>,
) -> ApiResult<Json<Version>> {
    version(&state, &id).await?;
    sqlx::query("UPDATE versions SET manifest = NULL, signature = NULL, manifest_key_id = NULL WHERE id = ?")
        .bind(&id)
        .execute(&state.db)
        .await?;
    Ok(Json(version(&state, &id).await?))
}

async fn version(state: &State, id: &str) -> ApiResult<Version> {
    Ok(sqlx::query_as("SELECT * FROM versions WHERE id = ?")
        .bind(id)
        .fetch_one(&state.db)
        .await?)
}

#[derive(Serialize)]
pub struct SignedManifest {
    manifest: Value,
    signature: String,
}

fn signed(v: Version) -> ApiResult<Json<SignedManifest>> {
    match (v.manifest, v.signature) {
        (Some(m), Some(signature)) => Ok(Json(SignedManifest {
            manifest: serde_json::from_str(&m)
                .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?,
            signature,
        })),
        _ => Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "Diese Version hat kein signiertes Manifest",
        )),
    }
}

fn launcher_only(state: &State, headers: &HeaderMap, peer: std::net::SocketAddr) -> ApiResult<()> {
    if !crate::api::is_launcher(headers) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "Dieses Modpack gibt es nur im TwoGG-Pinecone-Launcher",
        ));
    }
    if !state.limiter.allow(crate::api::client_ip(headers, peer)) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "Zu viele Anfragen, bitte später erneut versuchen",
        ));
    }
    Ok(())
}

/// `GET /api/v1/packs/{id}/versions/{version}/manifest` (Versions-ID).
pub async fn get_version(
    AxumState(state): AxumState<State>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    Path((id, version)): Path<(String, String)>,
) -> ApiResult<Json<SignedManifest>> {
    launcher_only(&state, &headers, peer)?;
    let v: Version = sqlx::query_as(
        "SELECT v.* FROM versions v JOIN packs p ON p.id = v.pack_id \
         WHERE v.id = ? AND v.pack_id = ? AND p.visibility != 'hidden' AND v.withdrawn_at IS NULL",
    )
    .bind(&version)
    .bind(&id)
    .fetch_one(&state.db)
    .await?;
    signed(v)
}

#[derive(Deserialize)]
pub struct LatestQuery {
    channel: Option<String>,
}

/// `GET /api/v1/packs/{id}/manifest/latest?channel=release`: neueste signierte Version, die
/// mindestens so stabil ist wie der Kanal.
pub async fn latest(
    AxumState(state): AxumState<State>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<LatestQuery>,
) -> ApiResult<Json<SignedManifest>> {
    launcher_only(&state, &headers, peer)?;
    let channels: &[&str] = match q.channel.as_deref().unwrap_or("release") {
        "release" => &["release"],
        "beta" => &["release", "beta"],
        "alpha" => &["release", "beta", "alpha"],
        _ => return Err(ApiError::bad_request("Kanal: release, beta oder alpha")),
    };
    let rows: Vec<Version> = sqlx::query_as(
        "SELECT v.* FROM versions v JOIN packs p ON p.id = v.pack_id \
         WHERE v.pack_id = ? AND p.visibility != 'hidden' AND v.withdrawn_at IS NULL \
         AND v.manifest IS NOT NULL ORDER BY v.published_at DESC",
    )
    .bind(&id)
    .fetch_all(&state.db)
    .await?;
    let v = rows
        .into_iter()
        .find(|v| channels.contains(&v.channel.as_str()))
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "Kein signiertes Manifest in diesem Kanal",
            )
        })?;
    signed(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_matches_launcher() {
        // Gleicher Testfall wie im Launcher (crates/core/src/eventpack.rs).
        let v = json!({ "b": [1, "x\n", true, null], "a": { "z": 1, "é": 2, "A": 3 }, "\u{20ac}": "€" });
        assert_eq!(
            canonical_json(&v).unwrap(),
            "{\"a\":{\"A\":3,\"z\":1,\"é\":2},\"b\":[1,\"x\\n\",true,null],\"€\":\"€\"}"
        );
        let v = json!({ "\u{fffd}": 1, "\u{1f600}": 2 });
        assert_eq!(
            canonical_json(&v).unwrap(),
            "{\"\u{1f600}\":2,\"\u{fffd}\":1}"
        );
        assert!(canonical_json(&json!({ "x": 0.5 })).is_err());
    }
}
