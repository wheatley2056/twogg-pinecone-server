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
    /// Impressum (https-Adresse). Erscheint auf den Pack-Seiten und im Launcher.
    #[serde(default, deserialize_with = "null_default")]
    pub imprint_url: String,
    /// Datenschutzerklärung (https-Adresse).
    #[serde(default, deserialize_with = "null_default")]
    pub privacy_url: String,
    /// Kontakt-E-Mail für Rechtliches (die Minecraft-Richtlinien verlangen einen Kontakt, der
    /// kein Chat- oder Forenlink ist).
    #[serde(default, deserialize_with = "null_default")]
    pub contact_email: String,
}

fn main_id() -> String {
    "main".into()
}

const KEYS: [&str; 5] = [
    "game_server_name",
    "game_server_address",
    "imprint_url",
    "privacy_url",
    "contact_email",
];

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
            "imprint_url" => s.imprint_url = v,
            "privacy_url" => s.privacy_url = v,
            "contact_email" => s.contact_email = v,
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

/// Leer oder eine https-Adresse ohne Leerzeichen.
fn valid_url(u: &str) -> bool {
    u.is_empty()
        || (u.len() <= 500 && u.starts_with("https://") && !u.contains(char::is_whitespace))
}

/// Leer oder eine einfache E-Mail-Adresse.
fn valid_email(e: &str) -> bool {
    e.is_empty()
        || (e.len() <= 254
            && !e.contains(char::is_whitespace)
            && e.split_once('@')
                .is_some_and(|(local, domain)| !local.is_empty() && domain.contains('.')))
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
    s.imprint_url = s.imprint_url.trim().to_string();
    s.privacy_url = s.privacy_url.trim().to_string();
    s.contact_email = s.contact_email.trim().to_string();
    if !valid_url(&s.imprint_url) || !valid_url(&s.privacy_url) {
        return Err(ApiError::bad_request(
            "Impressum und Datenschutz brauchen eine https-Adresse",
        ));
    }
    if !valid_email(&s.contact_email) {
        return Err(ApiError::bad_request("Ungültige E-Mail-Adresse"));
    }
    let values = [
        &s.game_server_name,
        &s.game_server_address,
        &s.imprint_url,
        &s.privacy_url,
        &s.contact_email,
    ];
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

    #[test]
    fn legal_fields() {
        assert!(valid_url(""));
        assert!(valid_url("https://twogg.app/datenschutz"));
        assert!(!valid_url("http://twogg.app"));
        assert!(!valid_url("javascript:alert(1)"));
        assert!(valid_email("kontakt@twogg.de"));
        assert!(valid_email(""));
        assert!(!valid_email("kein-at.de"));
        assert!(!valid_email("a@b"));
    }
}
