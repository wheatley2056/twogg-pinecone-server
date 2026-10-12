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
    /// Hinweis an alle Launcher (F-60); leer = kein Banner.
    #[serde(default, deserialize_with = "null_default")]
    pub banner_text: String,
    /// `info` oder `warning`.
    #[serde(default, deserialize_with = "null_default")]
    pub banner_level: String,
    /// Optionaler Link (https) im Banner.
    #[serde(default, deserialize_with = "null_default")]
    pub banner_link: String,
    /// Banner bis zu diesem Zeitpunkt (Unix-Sekunden); 0 = ohne Ende.
    #[serde(default, deserialize_with = "null_default")]
    pub banner_until: i64,
    /// Ältere Launcher müssen aktualisieren, um TwoGG-Packs und Events zu nutzen (z. B. `0.9.8`).
    #[serde(default, deserialize_with = "null_default")]
    pub min_launcher_version: String,
}

/// Banner für `/api/v1/info`, solange es gilt.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Banner {
    /// Ändert sich mit Text, Stufe oder Link: Ein geändertes Banner erscheint wieder (AK-60.1).
    pub id: String,
    pub level: String,
    pub text: String,
    pub link: Option<String>,
    pub until: Option<i64>,
}

impl Settings {
    pub fn banner(&self, now: i64) -> Option<Banner> {
        if self.banner_text.is_empty() || (self.banner_until > 0 && self.banner_until <= now) {
            return None;
        }
        use sha2::Digest;
        let level = if self.banner_level == "warning" {
            "warning"
        } else {
            "info"
        };
        let hash = sha2::Sha256::digest(format!(
            "{level}\n{}\n{}",
            self.banner_text, self.banner_link
        ));
        Some(Banner {
            id: hash.iter().take(6).map(|b| format!("{b:02x}")).collect(),
            level: level.into(),
            text: self.banner_text.clone(),
            link: (!self.banner_link.is_empty()).then(|| self.banner_link.clone()),
            until: (self.banner_until > 0).then_some(self.banner_until),
        })
    }
}

/// Versionsnummer `x.y.z` (nur Ziffern).
pub fn valid_version(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()))
}

fn main_id() -> String {
    "main".into()
}

const KEYS: [&str; 10] = [
    "game_server_name",
    "game_server_address",
    "imprint_url",
    "privacy_url",
    "contact_email",
    "banner_text",
    "banner_level",
    "banner_link",
    "banner_until",
    "min_launcher_version",
];

pub async fn load(state: &State) -> ApiResult<Settings> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT key, value FROM settings")
        .fetch_all(&state.db)
        .await?;
    let mut s = Settings {
        id: main_id(),
        banner_level: "info".into(),
        ..Default::default()
    };
    for (k, v) in rows {
        match k.as_str() {
            "game_server_name" => s.game_server_name = v,
            "game_server_address" => s.game_server_address = v,
            "imprint_url" => s.imprint_url = v,
            "privacy_url" => s.privacy_url = v,
            "contact_email" => s.contact_email = v,
            "banner_text" => s.banner_text = v,
            "banner_level" => s.banner_level = v,
            "banner_link" => s.banner_link = v,
            "banner_until" => s.banner_until = v.parse().unwrap_or(0),
            "min_launcher_version" => s.min_launcher_version = v,
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
    s.banner_text = s
        .banner_text
        .chars()
        .filter(|c| !c.is_control())
        .take(300)
        .collect::<String>()
        .trim()
        .to_string();
    s.banner_level = if s.banner_level == "warning" {
        "warning".into()
    } else {
        "info".into()
    };
    s.banner_link = s.banner_link.trim().to_string();
    if !valid_url(&s.banner_link) {
        return Err(ApiError::bad_request(
            "Der Banner-Link braucht eine https-Adresse",
        ));
    }
    s.banner_until = s.banner_until.max(0);
    s.min_launcher_version = s.min_launcher_version.trim().to_string();
    if !s.min_launcher_version.is_empty() && !valid_version(&s.min_launcher_version) {
        return Err(ApiError::bad_request(
            "Mindestversion im Format 0.9.8 angeben (oder leer lassen)",
        ));
    }
    let until = s.banner_until.to_string();
    let values = [
        &s.game_server_name,
        &s.game_server_address,
        &s.imprint_url,
        &s.privacy_url,
        &s.contact_email,
        &s.banner_text,
        &s.banner_level,
        &s.banner_link,
        &until,
        &s.min_launcher_version,
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
    fn banner_and_versions() {
        let mut s = Settings {
            banner_text: "Wartung heute ab 20 Uhr".into(),
            banner_level: "warning".into(),
            ..Default::default()
        };
        let b = s.banner(100).unwrap();
        assert_eq!(b.level, "warning");
        assert_eq!(b.id.len(), 12);
        s.banner_until = 100;
        assert!(s.banner(100).is_none(), "abgelaufen");
        s.banner_until = 0;
        s.banner_text = "Wartung heute ab 21 Uhr".into();
        assert_ne!(
            s.banner(100).unwrap().id,
            b.id,
            "geänderter Text → neues Banner"
        );
        s.banner_text.clear();
        assert!(s.banner(100).is_none());
        assert!(valid_version("0.9.8"));
        assert!(!valid_version("0.9"));
        assert!(!valid_version("1.0.0-beta"));
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
