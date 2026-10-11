//! Einstellungen aus der Admin-Seite (Schlüssel/Wert in `settings`). Für React-Admin als ein
//! Datensatz `settings/main`.

use axum::Json;
use axum::extract::State as AxumState;
use serde::{Deserialize, Serialize};

use crate::auth::Admin;
use crate::error::null_default;
use crate::{ApiError, ApiResult, JsonBody, State};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Für React-Admin immer `main`.
    #[serde(default = "main_id")]
    pub id: String,
    /// Anzeigename des Community-Servers im Launcher, z. B. „TwoGG Server“.
    #[serde(default, deserialize_with = "null_default")]
    pub game_server_name: String,
    /// Adresse für den Schnellbeitritt, z. B. `mc.twogg.de`; leer blendet die Kachel aus.
    #[serde(default, deserialize_with = "null_default")]
    pub game_server_address: String,
}

fn main_id() -> String {
    "main".into()
}

const KEYS: [&str; 2] = ["game_server_name", "game_server_address"];

pub async fn load(state: &State) -> ApiResult<Settings> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT key, value FROM settings")
        .fetch_all(&state.db)
        .await?;
    let mut s = Settings {
        id: main_id(),
        ..Default::default()
    };
    for (k, v) in rows {
        match k.as_str() {
            "game_server_name" => s.game_server_name = v,
            "game_server_address" => s.game_server_address = v,
            _ => {}
        }
    }
    Ok(s)
}

/// Server-Adresse: Host mit optionalem Port, keine Leerzeichen oder Schemata.
fn valid_address(a: &str) -> bool {
    a.is_empty()
        || (a.len() <= 255
            && !a.contains("://")
            && a.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':' | b'_')))
}

pub async fn get(_admin: Admin, AxumState(state): AxumState<State>) -> ApiResult<Json<Settings>> {
    Ok(Json(load(&state).await?))
}

pub async fn put(
    _admin: Admin,
    AxumState(state): AxumState<State>,
    JsonBody(mut s): JsonBody<Settings>,
) -> ApiResult<Json<Settings>> {
    s.game_server_name = s.game_server_name.trim().chars().take(48).collect();
    s.game_server_address = s.game_server_address.trim().to_string();
    if !valid_address(&s.game_server_address) {
        return Err(ApiError::bad_request(
            "Ungültige Server-Adresse (z. B. mc.twogg.de oder host:25570)",
        ));
    }
    let values = [&s.game_server_name, &s.game_server_address];
    let mut tx = state.db.begin().await?;
    for (key, value) in KEYS.iter().zip(values) {
        sqlx::query("INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
            .bind(key)
            .bind(value)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(Json(load(&state).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert!(valid_address("mc.twogg.de"));
        assert!(valid_address("game01.twogg.de:25570"));
        assert!(valid_address(""));
        assert!(!valid_address("https://twogg.de"));
        assert!(!valid_address("mc twogg"));
    }
}
