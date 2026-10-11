//! SQLite: Schema und Datentypen. Migrationen laufen beim Start (über `PRAGMA user_version`).

use std::path::Path;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{FromRow, SqlitePool};

/// Schemaänderungen nur anhängen, nie bestehende ändern.
const MIGRATIONS: &[&str] = &[
    r#"
CREATE TABLE packs (
    id          TEXT PRIMARY KEY,
    title       TEXT NOT NULL,
    summary     TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL DEFAULT '',
    icon        TEXT,
    -- public: im Launcher gelistet; unlisted: nur per Link/ID; hidden: nirgends
    visibility  TEXT NOT NULL DEFAULT 'hidden',
    featured    INTEGER NOT NULL DEFAULT 0,
    sort_order  INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
CREATE TABLE versions (
    id             TEXT PRIMARY KEY,
    pack_id        TEXT NOT NULL REFERENCES packs(id) ON DELETE CASCADE,
    version        TEXT NOT NULL,
    minecraft      TEXT NOT NULL,
    loader         TEXT,
    loader_version TEXT,
    channel        TEXT NOT NULL DEFAULT 'release',
    changelog      TEXT NOT NULL DEFAULT '',
    file           TEXT NOT NULL,
    size           INTEGER NOT NULL,
    sha1           TEXT NOT NULL,
    sha512         TEXT NOT NULL,
    downloads      INTEGER NOT NULL DEFAULT 0,
    published_at   INTEGER NOT NULL
);
CREATE INDEX versions_pack ON versions(pack_id, published_at DESC);
CREATE TABLE news (
    id           TEXT PRIMARY KEY,
    title        TEXT NOT NULL,
    body         TEXT NOT NULL DEFAULT '',
    image        TEXT,
    link         TEXT,
    pinned       INTEGER NOT NULL DEFAULT 0,
    visible      INTEGER NOT NULL DEFAULT 1,
    published_at INTEGER NOT NULL
);
"#,
    r#"
-- Einstellungen aus der Admin-Seite (z. B. TwoGG-Server für den Launcher)
CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
-- Freundesliste: Spieler sind Minecraft-Accounts, nachgewiesen über Mojangs Sitzungsserver
CREATE TABLE players (
    uuid       TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    name_lower TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_seen  INTEGER NOT NULL DEFAULT 0,
    -- JSON: was der Spieler gerade macht (nur für Freunde sichtbar)
    activity   TEXT
);
CREATE INDEX players_name ON players(name_lower);
CREATE TABLE player_sessions (
    token_hash TEXT PRIMARY KEY,
    uuid       TEXT NOT NULL REFERENCES players(uuid) ON DELETE CASCADE,
    expires_at INTEGER NOT NULL
);
CREATE TABLE friend_requests (
    from_uuid  TEXT NOT NULL REFERENCES players(uuid) ON DELETE CASCADE,
    to_uuid    TEXT NOT NULL REFERENCES players(uuid) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (from_uuid, to_uuid)
);
-- Freundschaft einmal gespeichert, mit a_uuid < b_uuid
CREATE TABLE friendships (
    a_uuid     TEXT NOT NULL REFERENCES players(uuid) ON DELETE CASCADE,
    b_uuid     TEXT NOT NULL REFERENCES players(uuid) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (a_uuid, b_uuid)
);
"#,
];

pub async fn connect(path: &Path) -> anyhow::Result<SqlitePool> {
    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(options)
        .await?;
    migrate(&pool).await?;
    Ok(pool)
}

async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    let (current,): (i64,) = sqlx::query_as("PRAGMA user_version")
        .fetch_one(pool)
        .await?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        let mut tx = pool.begin().await?;
        sqlx::raw_sql(sql).execute(&mut *tx).await?;
        sqlx::raw_sql(&format!("PRAGMA user_version = {}", i + 1))
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        tracing::info!("Datenbank auf Schema {} migriert", i + 1);
    }
    Ok(())
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "lowercase")]
#[sqlx(rename_all = "lowercase")]
pub enum Visibility {
    Public,
    Unlisted,
    Hidden,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Pack {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub description: String,
    pub icon: Option<String>,
    pub visibility: Visibility,
    pub featured: bool,
    pub sort_order: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    pub id: String,
    pub pack_id: String,
    pub version: String,
    pub minecraft: String,
    pub loader: Option<String>,
    pub loader_version: Option<String>,
    pub channel: String,
    pub changelog: String,
    #[serde(skip)]
    pub file: String,
    pub size: i64,
    pub sha1: String,
    pub sha512: String,
    pub downloads: i64,
    pub published_at: i64,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct News {
    pub id: String,
    pub title: String,
    pub body: String,
    pub image: Option<String>,
    pub link: Option<String>,
    pub pinned: bool,
    pub visible: bool,
    pub published_at: i64,
}

/// Pack-IDs erscheinen in URLs: klein, ASCII, Bindestriche.
pub fn valid_pack_id(id: &str) -> bool {
    (2..=48).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
        && !id.ends_with('-')
}

pub async fn versions_of(db: &SqlitePool, pack_id: &str) -> sqlx::Result<Vec<Version>> {
    sqlx::query_as("SELECT * FROM versions WHERE pack_id = ? ORDER BY published_at DESC")
        .bind(pack_id)
        .fetch_all(db)
        .await
}
