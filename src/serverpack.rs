//! Server-Pack aus einer Pack-Version erzeugen (F-61).
//!
//! Aus der `.mrpack`-Datei entsteht ein ZIP für den Minecraft-Server: alle Dateien außer denen
//! mit `env.server = "unsupported"` (heruntergeladen von den erlaubten Hosts und per SHA-1
//! geprüft), dazu `overrides/` und `server-overrides/` (gewinnt bei gleichen Pfaden). Für Fabric
//! liegt der Server-Starter von meta.fabricmc.net bei, für andere Loader beschreibt
//! `PINECONE-SERVER.txt` den Installer. Die EULA nimmt das Pack nicht an.
//!
//! Das Erzeugen läuft im Hintergrund; der Admin fragt den Stand über `GET …/server-pack` ab.
//! Fertige Packs liegen in `server-packs/<version>.zip` mit Metadaten in `<version>.json`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use axum::Json;
use axum::extract::{Path as UrlPath, State as AxumState};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use sha2::Sha512;

use crate::auth::Admin;
use crate::db::{self, Version};
use crate::error::{ApiError, ApiResult};
use crate::{State, files};

/// Hosts, von denen Mod-Dateien geladen werden (wie in der Modrinth-Spezifikation).
pub const DOWNLOAD_HOSTS: &[&str] = &[
    "cdn.modrinth.com",
    "github.com",
    "raw.githubusercontent.com",
    "gitlab.com",
];
pub const DEFAULT_FABRIC_META: &str = "https://meta.fabricmc.net";
/// Obergrenze für alle heruntergeladenen Dateien zusammen.
const MAX_TOTAL: u64 = 4 * 1024 * 1024 * 1024;
const INFO_FILE: &str = "PINECONE-SERVER.txt";

pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("server-packs")
}

/// Laufende und fehlgeschlagene Erzeugungen (nur im Speicher).
#[derive(Default)]
pub struct Jobs {
    inner: Mutex<HashMap<String, Option<String>>>,
}

impl Jobs {
    /// true, wenn der Job neu gestartet werden darf (läuft nicht schon).
    fn start(&self, id: &str) -> bool {
        let mut map = self.inner.lock().unwrap();
        if matches!(map.get(id), Some(None)) {
            return false;
        }
        map.insert(id.to_string(), None);
        true
    }

    fn finish(&self, id: &str, error: Option<String>) {
        let mut map = self.inner.lock().unwrap();
        match error {
            Some(e) => map.insert(id.to_string(), Some(e)),
            None => map.remove(id),
        };
    }

    /// `Some(None)` = läuft, `Some(Some(e))` = fehlgeschlagen.
    fn get(&self, id: &str) -> Option<Option<String>> {
        self.inner.lock().unwrap().get(id).cloned()
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    pub created_at: i64,
    pub size: u64,
    pub files: usize,
    /// Dateien, die nur der Client braucht und daher fehlen.
    pub removed: Vec<String>,
    pub loader: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// `none`, `building`, `ready` oder `failed`.
    state: &'static str,
    error: Option<String>,
    #[serde(flatten)]
    meta: Option<Meta>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Index {
    #[serde(default)]
    files: Vec<IndexFile>,
    #[serde(default)]
    dependencies: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndexFile {
    path: String,
    hashes: Hashes,
    #[serde(default)]
    env: Option<Env>,
    #[serde(default)]
    downloads: Vec<String>,
}

#[derive(Deserialize)]
struct Hashes {
    sha1: String,
    #[serde(default)]
    sha512: Option<String>,
}

#[derive(Deserialize)]
struct Env {
    #[serde(default)]
    server: String,
}

async fn version(state: &State, id: &str) -> ApiResult<Version> {
    Ok(sqlx::query_as("SELECT * FROM versions WHERE id = ?")
        .bind(id)
        .fetch_one(&state.db)
        .await?)
}

fn read_meta(data_dir: &Path, id: &str) -> Option<Meta> {
    let dir = dir(data_dir);
    if !dir.join(format!("{id}.zip")).is_file() {
        return None;
    }
    serde_json::from_slice(&std::fs::read(dir.join(format!("{id}.json"))).ok()?).ok()
}

/// `GET /api/admin/versions/{id}/server-pack`: Stand der Erzeugung.
pub async fn status(
    AxumState(state): AxumState<State>,
    _: Admin,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<Json<Status>> {
    version(&state, &id).await?;
    let meta = read_meta(&state.config.data_dir, &id);
    let (st, error) = match state.server_packs.get(&id) {
        Some(None) => ("building", None),
        Some(Some(e)) => ("failed", Some(e)),
        None if meta.is_some() => ("ready", None),
        None => ("none", None),
    };
    Ok(Json(Status {
        state: st,
        error,
        meta,
    }))
}

/// `POST /api/admin/versions/{id}/server-pack`: Erzeugung starten (oder neu erzeugen).
pub async fn start(
    AxumState(state): AxumState<State>,
    admin: Admin,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<Json<Status>> {
    let v = version(&state, &id).await?;
    if state.server_packs.start(&id) {
        let st = state.clone();
        tokio::spawn(async move {
            let res = build(&st, &v).await;
            if let Err(e) = &res {
                tracing::warn!("Server-Pack für {}/{}: {e}", v.pack_id, v.version);
            } else {
                tracing::info!(
                    "Server-Pack für {}/{} erzeugt ({})",
                    v.pack_id,
                    v.version,
                    admin.user
                );
            }
            st.server_packs.finish(&v.id, res.err());
        });
    }
    Ok(Json(Status {
        state: "building",
        error: None,
        meta: None,
    }))
}

/// `POST /api/admin/versions/{id}/server-pack/link`: kurzlebiger Download-Link für den Browser.
pub async fn link(
    AxumState(state): AxumState<State>,
    _: Admin,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<Json<serde_json::Value>> {
    if read_meta(&state.config.data_dir, &id).is_none() {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "Noch kein Server-Pack erzeugt",
        ));
    }
    let token = files::sign_download(&state.secret, &format!("server-{id}"), db::now() + 300);
    Ok(Json(
        serde_json::json!({ "url": format!("/dl/server/{token}") }),
    ))
}

/// `GET /dl/server/{token}`: das ZIP.
pub async fn download(
    AxumState(state): AxumState<State>,
    UrlPath(token): UrlPath<String>,
) -> ApiResult<Response> {
    let id = files::verify_download(&state.secret, &token, db::now())
        .and_then(|p| p.strip_prefix("server-").map(str::to_string))
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::FORBIDDEN,
                "Download-Link ungültig oder abgelaufen",
            )
        })?;
    let v = version(&state, &id).await?;
    let file = tokio::fs::File::open(dir(&state.config.data_dir).join(format!("{id}.zip"))).await?;
    let body = axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(file));
    Ok((
        [
            (header::CONTENT_TYPE, "application/zip".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!(
                    "attachment; filename=\"{}-{}-server.zip\"",
                    v.pack_id, v.version
                ),
            ),
        ],
        body,
    )
        .into_response())
}

/// Server-Packs zu gelöschten Versionen entfernen.
pub async fn cleanup(state: &State) -> anyhow::Result<()> {
    let dir = dir(&state.config.data_dir);
    let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
        return Ok(());
    };
    let ids: HashSet<String> = sqlx::query_scalar("SELECT id FROM versions")
        .fetch_all(&state.db)
        .await?
        .into_iter()
        .collect();
    while let Some(e) = entries.next_entry().await? {
        let name = e.file_name().to_string_lossy().to_string();
        let id = name.split('.').next().unwrap_or_default();
        if !ids.contains(id) {
            let _ = tokio::fs::remove_file(e.path()).await;
        }
    }
    Ok(())
}

fn allowed(state: &State, url: &str) -> bool {
    let Ok(u) = reqwest::Url::parse(url) else {
        return false;
    };
    let host = u.host_str().unwrap_or_default();
    (u.scheme() == "https" && DOWNLOAD_HOSTS.contains(&host))
        || state.config.extra_download_hosts.iter().any(|h| h == host)
}

/// Lädt `url` nach `dest` und liefert SHA-1 und SHA-512.
async fn fetch(
    state: &State,
    url: &str,
    dest: &Path,
    budget: &mut u64,
) -> Result<(String, String), String> {
    use futures::StreamExt;
    let resp = state
        .http
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("{url}: {e}"))?;
    let mut out = tokio::fs::File::create(dest)
        .await
        .map_err(|e| e.to_string())?;
    let mut hash = Sha1::new();
    let mut hash512 = Sha512::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("{url}: {e}"))?;
        *budget = budget
            .checked_sub(chunk.len() as u64)
            .ok_or("Server-Pack wäre größer als 4 GB")?;
        hash.update(&chunk);
        hash512.update(&chunk);
        tokio::io::AsyncWriteExt::write_all(&mut out, &chunk)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok((hex(&hash.finalize()), hex(&hash512.finalize())))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn file_name(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

async fn build(state: &State, v: &Version) -> Result<(), String> {
    let data_dir = &state.config.data_dir;
    let mrpack = files::packs_dir(data_dir).join(&v.file);
    let index: Index = {
        let path = mrpack.clone();
        tokio::task::spawn_blocking(move || -> Result<Index, String> {
            let mut zip =
                zip::ZipArchive::new(std::fs::File::open(path).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            let mut text = String::new();
            zip.by_name("modrinth.index.json")
                .map_err(|e| e.to_string())?
                .take(16 * 1024 * 1024)
                .read_to_string(&mut text)
                .map_err(|e| e.to_string())?;
            serde_json::from_str(&text).map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())??
    };

    let out_dir = dir(data_dir);
    tokio::fs::create_dir_all(&out_dir)
        .await
        .map_err(|e| e.to_string())?;
    let work = out_dir.join(format!("{}.work", v.id));
    let _ = tokio::fs::remove_dir_all(&work).await;
    tokio::fs::create_dir_all(&work)
        .await
        .map_err(|e| e.to_string())?;
    let result = build_in(state, v, &index, &mrpack, &work).await;
    let _ = tokio::fs::remove_dir_all(&work).await;
    result
}

async fn build_in(
    state: &State,
    v: &Version,
    index: &Index,
    mrpack: &Path,
    work: &Path,
) -> Result<(), String> {
    let mut budget = MAX_TOTAL;
    let mut downloaded: Vec<(String, PathBuf)> = Vec::new();
    let mut removed = Vec::new();
    for (i, f) in index.files.iter().enumerate() {
        if f.env.as_ref().is_some_and(|e| e.server == "unsupported") {
            removed.push(file_name(&f.path));
            continue;
        }
        let url = f
            .downloads
            .iter()
            .find(|u| allowed(state, u))
            .ok_or_else(|| format!("{}: kein erlaubter Download-Host", f.path))?;
        let dest = work.join(format!("{i}.bin"));
        let (sha1, sha512) = fetch(state, url, &dest, &mut budget).await?;
        // SHA-512 ist in der Modrinth-Spezifikation Pflicht; ältere Packs haben teils nur SHA-1.
        let expected512 = f.hashes.sha512.as_deref().unwrap_or_default();
        if !sha1.eq_ignore_ascii_case(&f.hashes.sha1)
            || (!expected512.is_empty() && !sha512.eq_ignore_ascii_case(expected512))
        {
            return Err(format!("{}: Prüfsumme stimmt nicht", f.path));
        }
        downloaded.push((f.path.clone(), dest));
    }

    let deps = &index.dependencies;
    let minecraft = deps.get("minecraft").cloned().unwrap_or_default();
    let loader = [
        ("neoforge", "NeoForge"),
        ("forge", "Forge"),
        ("quilt-loader", "Quilt"),
        ("fabric-loader", "Fabric"),
    ]
    .into_iter()
    .find_map(|(dep, name)| deps.get(dep).map(|ver| (dep, name, ver.clone())));

    // Fabric: Server-Starter mitliefern.
    let mut launcher = None;
    if let Some(("fabric-loader", _, loader_version)) = &loader {
        let meta = &state.config.fabric_meta;
        #[derive(Deserialize)]
        struct Installer {
            version: String,
            stable: bool,
        }
        let installers: Vec<Installer> = state
            .http
            .get(format!("{meta}/v2/versions/installer"))
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("Fabric-Meta: {e}"))?
            .json()
            .await
            .map_err(|e| format!("Fabric-Meta: {e}"))?;
        let installer = installers
            .iter()
            .find(|i| i.stable)
            .or(installers.first())
            .ok_or("Fabric-Meta: kein Installer")?;
        let dest = work.join("fabric-server-launch.jar");
        fetch(
            state,
            &format!(
                "{meta}/v2/versions/loader/{minecraft}/{loader_version}/{}/server/jar",
                installer.version
            ),
            &dest,
            &mut budget,
        )
        .await?;
        launcher = Some(dest);
    }

    let info = server_info(
        v,
        &minecraft,
        loader.as_ref().map(|(_, n, ver)| (*n, ver.as_str())),
        &removed,
    );
    let loader_name = loader.as_ref().map(|(_, n, ver)| format!("{n} {ver}"));
    let zip_path = dir(&state.config.data_dir).join(format!("{}.zip", v.id));
    let part = zip_path.with_extension("zip.part");
    let mrpack = mrpack.to_path_buf();
    let (size, count) = {
        let part = part.clone();
        tokio::task::spawn_blocking(move || {
            write_zip(&part, &mrpack, &downloaded, launcher.as_deref(), &info)
        })
        .await
        .map_err(|e| e.to_string())??
    };
    tokio::fs::rename(&part, &zip_path)
        .await
        .map_err(|e| e.to_string())?;
    let meta = Meta {
        created_at: db::now(),
        size,
        files: count,
        removed,
        loader: loader_name,
    };
    tokio::fs::write(
        zip_path.with_extension("json"),
        serde_json::to_vec(&meta).unwrap_or_default(),
    )
    .await
    .map_err(|e| e.to_string())
}

fn server_info(
    v: &Version,
    minecraft: &str,
    loader: Option<(&str, &str)>,
    removed: &[String],
) -> String {
    let mut s = format!(
        "Server-Pack für {} {}, erzeugt von TwoGG Pinecone\r\n\r\nMinecraft {minecraft}\r\n",
        v.pack_id, v.version
    );
    match loader {
        Some(("Fabric", ver)) => {
            s += &format!(
                "Fabric Loader {ver}\r\n\r\nStarten: java -Xmx4G -jar fabric-server-launch.jar nogui\r\n\
             (lädt beim ersten Start den Minecraft-Server von Mojang)\r\n"
            )
        }
        Some(("Quilt", ver)) => {
            s += &format!(
                "Quilt Loader {ver}\r\n\r\nInstallieren: Quilt-Installer von https://quiltmc.org/install/server/ laden, dann\r\n\
             java -jar quilt-installer.jar install server {minecraft} {ver} --download-server --install-dir=.\r\n"
            )
        }
        Some((name @ ("Forge" | "NeoForge"), ver)) => {
            s += &format!(
                "{name} {ver}\r\n\r\nInstallieren: {name}-Installer {ver} für Minecraft {minecraft} laden\r\n\
             ({}), dann\r\njava -jar <installer>.jar --installServer\r\n",
                if name == "Forge" {
                    "https://files.minecraftforge.net"
                } else {
                    "https://neoforged.net"
                }
            )
        }
        _ => {
            s += "Ohne Mod-Loader: server.jar für diese Minecraft-Version von minecraft.net laden.\r\n"
        }
    }
    s += "\r\nVor dem ersten Start die Minecraft-EULA lesen und in eula.txt eula=true setzen:\r\n\
          https://aka.ms/MinecraftEULA\r\n";
    if !removed.is_empty() {
        s += "\r\nWeggelassen, weil nur für den Client:\r\n";
        for r in removed {
            s += &format!("- {r}\r\n");
        }
    }
    s
}

fn write_zip(
    dest: &Path,
    mrpack: &Path,
    downloaded: &[(String, PathBuf)],
    launcher: Option<&Path>,
    info: &str,
) -> Result<(u64, usize), String> {
    let err = |e: &dyn std::fmt::Display| e.to_string();
    let mut zip = zip::ZipWriter::new(std::fs::File::create(dest).map_err(|e| err(&e))?);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true);
    let mut written: HashSet<String> = HashSet::new();
    let mut add = |zip: &mut zip::ZipWriter<std::fs::File>,
                   name: &str,
                   reader: &mut dyn Read|
     -> Result<(), String> {
        if !written.insert(name.to_string()) {
            return Ok(());
        }
        zip.start_file(name, opts).map_err(|e| err(&e))?;
        std::io::copy(reader, zip).map_err(|e| err(&e))?;
        Ok(())
    };
    add(&mut zip, INFO_FILE, &mut info.as_bytes())?;
    if let Some(l) = launcher {
        add(
            &mut zip,
            "fabric-server-launch.jar",
            &mut std::fs::File::open(l).map_err(|e| err(&e))?,
        )?;
    }
    // server-overrides vor overrides, damit sie bei gleichen Pfaden gewinnen.
    let mut src = zip::ZipArchive::new(std::fs::File::open(mrpack).map_err(|e| err(&e))?)
        .map_err(|e| err(&e))?;
    for prefix in ["server-overrides/", "overrides/"] {
        for i in 0..src.len() {
            let mut entry = src.by_index(i).map_err(|e| err(&e))?;
            let Some(rel) = entry.name().strip_prefix(prefix).map(str::to_string) else {
                continue;
            };
            if entry.is_dir() || rel.is_empty() || !crate::mrpack::safe_relative(&rel) {
                continue;
            }
            add(&mut zip, &rel, &mut entry)?;
        }
    }
    for (path, file) in downloaded {
        add(
            &mut zip,
            path,
            &mut std::fs::File::open(file).map_err(|e| err(&e))?,
        )?;
    }
    let count = written.len();
    let mut file = zip.finish().map_err(|e| err(&e))?;
    file.flush().map_err(|e| err(&e))?;
    let size = file.metadata().map_err(|e| err(&e))?.len();
    Ok((size, count))
}
