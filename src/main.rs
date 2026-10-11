//! TwoGG Pinecone Server: liefert dem Launcher Neuigkeiten und TwoGG-Modpacks und bietet unter
//! `/admin` eine Verwaltungsoberfläche (React-Admin).
//!
//! Konfiguration über Umgebungsvariablen (siehe `Config::from_env` und `server/README.md`).

pub(crate) mod admin;
mod api;
mod auth;
mod db;
mod error;
mod files;
mod friends;
mod markdown;
mod recommendations;
mod settings;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderValue, header};
use axum::routing::get;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

pub use error::{ApiError, ApiResult, JsonBody};

/// Größte erlaubte Modpack-Datei beim Hochladen.
pub const MAX_UPLOAD: usize = 1024 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Config {
    pub addr: SocketAddr,
    pub data_dir: PathBuf,
    /// Öffentliche Adresse, z. B. `https://pinecone.twogg.app` (für Links in Antworten).
    pub public_url: String,
    pub admin_user: String,
    /// Argon2-Hash (PHC-String) des Admin-Passworts.
    pub admin_password_hash: String,
    /// Gebaute Admin-Oberfläche (`admin/dist`).
    pub admin_dir: PathBuf,
    /// Gültigkeit von Download-Links in Sekunden.
    pub download_ttl: u64,
    /// Mojangs Sitzungsserver (Anmeldung für die Freundesliste); in Tests ein Mock.
    pub session_server: String,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        let admin_password_hash = match (
            var("PINECONE_ADMIN_PASSWORD_HASH"),
            var("PINECONE_ADMIN_PASSWORD"),
        ) {
            (Some(hash), _) => hash,
            (None, Some(plain)) => auth::hash_password(&plain)?,
            (None, None) => anyhow::bail!(
                "PINECONE_ADMIN_PASSWORD oder PINECONE_ADMIN_PASSWORD_HASH muss gesetzt sein"
            ),
        };
        Ok(Self {
            addr: var("PINECONE_ADDR")
                .unwrap_or_else(|| "0.0.0.0:8080".into())
                .parse()?,
            data_dir: var("PINECONE_DATA_DIR")
                .unwrap_or_else(|| "./data".into())
                .into(),
            public_url: var("PINECONE_PUBLIC_URL")
                .unwrap_or_else(|| "http://localhost:8080".into())
                .trim_end_matches('/')
                .to_string(),
            admin_user: var("PINECONE_ADMIN_USER").unwrap_or_else(|| "admin".into()),
            admin_password_hash,
            admin_dir: var("PINECONE_ADMIN_DIR")
                .unwrap_or_else(|| "./admin/dist".into())
                .into(),
            download_ttl: var("PINECONE_DOWNLOAD_TTL")
                .and_then(|v| v.parse().ok())
                .unwrap_or(600),
            session_server: var("PINECONE_SESSION_SERVER")
                .unwrap_or_else(|| friends::DEFAULT_SESSION_SERVER.into()),
        })
    }
}

pub struct AppState {
    pub config: Config,
    pub db: sqlx::SqlitePool,
    /// Schlüssel für signierte Download-Links (liegt im Datenordner).
    pub secret: Vec<u8>,
    pub sessions: auth::Sessions,
    pub limiter: api::RateLimiter,
    pub challenges: friends::Challenges,
    pub http: reqwest::Client,
}

pub type State = Arc<AppState>;

pub async fn build_state(config: Config) -> anyhow::Result<State> {
    std::fs::create_dir_all(files::packs_dir(&config.data_dir))?;
    std::fs::create_dir_all(files::media_dir(&config.data_dir))?;
    let db = db::connect(&config.data_dir.join("pinecone.db")).await?;
    let secret = files::load_or_create_secret(&config.data_dir.join("secret.key"))?;
    Ok(Arc::new(AppState {
        config,
        db,
        secret,
        sessions: auth::Sessions::default(),
        limiter: api::RateLimiter::default(),
        challenges: friends::Challenges::default(),
        http: reqwest::Client::builder()
            .user_agent(concat!("pinecone-server/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(10))
            .build()?,
    }))
}

pub fn router(state: State) -> Router {
    let admin_dir = state.config.admin_dir.clone();
    // Admin-SPA: unbekannte Pfade unter /admin liefern index.html (clientseitiges Routing).
    let admin_spa =
        ServeDir::new(&admin_dir).fallback(ServeFile::new(admin_dir.join("index.html")));
    let media = ServeDir::new(files::media_dir(&state.config.data_dir));
    Router::new()
        .route("/", get(api::index))
        .route("/health", get(|| async { "ok" }))
        .nest("/api/v1", api::routes().merge(friends::routes()))
        .nest(
            "/api/admin",
            admin::routes(state.clone()).layer(DefaultBodyLimit::max(MAX_UPLOAD)),
        )
        .route("/dl/{token}", get(api::download))
        .nest_service(
            "/media",
            tower::ServiceBuilder::new()
                .layer(SetResponseHeaderLayer::overriding(
                    header::CACHE_CONTROL,
                    HeaderValue::from_static("public, max-age=86400"),
                ))
                .layer(SetResponseHeaderLayer::overriding(
                    header::X_CONTENT_TYPE_OPTIONS,
                    HeaderValue::from_static("nosniff"),
                ))
                .service(media),
        )
        .nest_service("/admin", admin_spa)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=info".into()),
        )
        .init();
    // Für den Docker-HEALTHCHECK (das Image enthält kein curl).
    if std::env::args().nth(1).as_deref() == Some("health") {
        let addr = std::env::var("PINECONE_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into());
        let port = addr.rsplit(':').next().unwrap_or("8080");
        return health(&format!("127.0.0.1:{port}"));
    }
    if std::env::args().nth(1).as_deref() == Some("hash-password") {
        let password = std::env::args()
            .nth(2)
            .ok_or_else(|| anyhow::anyhow!("Aufruf: pinecone-server hash-password <passwort>"))?;
        println!("{}", auth::hash_password(&password)?);
        return Ok(());
    }
    let config = Config::from_env()?;
    let addr = config.addr;
    let state = build_state(config).await?;
    tracing::info!("Pinecone-Server lauscht auf {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown())
    .await?;
    Ok(())
}

fn health(addr: &str) -> anyhow::Result<()> {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(addr)?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
    stream.write_all(b"GET /health HTTP/1.0\r\nHost: localhost\r\n\r\n")?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    if response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.0 200") {
        Ok(())
    } else {
        anyhow::bail!("Server antwortet nicht mit 200")
    }
}

/// Beenden bei Strg+C oder SIGTERM (`docker stop`).
async fn shutdown() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM-Handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("Server wird beendet");
}

#[cfg(test)]
mod tests;
