//! Anmeldung für die Admin-Oberfläche: Konten mit Rolle in der Datenbank (Argon2-Hash, siehe
//! `accounts.rs`), Sitzungen als zufällige Bearer-Tokens im Speicher (nach einem Neustart neu
//! anmelden).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use base64::Engine;

use crate::{ApiError, State};

const SESSION_TTL: Duration = Duration::from_secs(12 * 3600);
/// Nach so vielen Fehlversuchen kurz sperren (gegen Durchprobieren).
const MAX_FAILURES: u32 = 5;
const LOCKOUT: Duration = Duration::from_secs(60);

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    if password.len() < 10 {
        anyhow::bail!("Das Admin-Passwort muss mindestens 10 Zeichen haben");
    }
    let salt = SaltString::generate(&mut rand::rngs::OsRng);
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("Passwort-Hash: {e}"))?
        .to_string())
}

pub fn verify_password(hash: &str, password: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|h| {
        Argon2::default()
            .verify_password(password.as_bytes(), &h)
            .is_ok()
    })
}

/// Rolle eines Admin-Kontos (F-63).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Alles, auch Packs, Einstellungen und Konten.
    Admin,
    /// Neuigkeiten, TwoGG-Auswahl und Bilder; alles andere nur lesen.
    Editor,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Editor => "editor",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "admin" => Some(Role::Admin),
            "editor" => Some(Role::Editor),
            _ => None,
        }
    }
}

#[derive(Default)]
pub struct Sessions {
    tokens: Mutex<HashMap<String, (String, Role, Instant)>>,
    failures: Mutex<(u32, Option<Instant>)>,
}

impl Sessions {
    pub fn create(&self, user: &str, role: Role) -> String {
        let mut raw = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut raw);
        let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw);
        let mut map = self.tokens.lock().unwrap();
        let now = Instant::now();
        map.retain(|_, (_, _, exp)| *exp > now);
        map.insert(token.clone(), (user.to_string(), role, now + SESSION_TTL));
        token
    }

    pub fn user(&self, token: &str) -> Option<(String, Role)> {
        let map = self.tokens.lock().unwrap();
        map.get(token)
            .filter(|(_, _, exp)| *exp > Instant::now())
            .map(|(u, r, _)| (u.clone(), *r))
    }

    pub fn revoke(&self, token: &str) {
        self.tokens.lock().unwrap().remove(token);
    }

    /// Alle Sitzungen eines Kontos beenden (nach Löschen, Rollen- oder Passwortwechsel).
    pub fn revoke_user(&self, user: &str) {
        self.tokens.lock().unwrap().retain(|_, (u, _, _)| u != user);
    }

    /// `Err(sekunden)`, solange nach zu vielen Fehlversuchen gesperrt ist.
    pub fn check_lockout(&self) -> Result<(), u64> {
        let f = self.failures.lock().unwrap();
        match f.1 {
            Some(until) if until > Instant::now() => Err((until - Instant::now()).as_secs() + 1),
            _ => Ok(()),
        }
    }

    pub fn record_failure(&self) {
        let mut f = self.failures.lock().unwrap();
        f.0 += 1;
        if f.0 >= MAX_FAILURES {
            f.0 = 0;
            f.1 = Some(Instant::now() + LOCKOUT);
        }
    }

    pub fn record_success(&self) {
        *self.failures.lock().unwrap() = (0, None);
    }
}

/// Angemeldeter Admin (als Extraktor in Admin-Handlern).
pub struct Admin {
    pub user: String,
    pub role: Role,
    pub token: String,
}

impl Admin {
    /// Nur für die Rolle „Admin“.
    pub fn require_admin(&self) -> Result<(), ApiError> {
        if self.role == Role::Admin {
            Ok(())
        } else {
            Err(ApiError::new(
                axum::http::StatusCode::FORBIDDEN,
                "Dafür braucht es die Rolle Admin",
            ))
        }
    }
}

impl FromRequestParts<State> for Admin {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &State) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or_else(ApiError::unauthorized)?
            .to_string();
        let (user, role) = state
            .sessions
            .user(&token)
            .ok_or_else(ApiError::unauthorized)?;
        Ok(Admin { user, role, token })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_and_sessions() {
        assert!(hash_password("kurz").is_err());
        let hash = hash_password("ein-langes-passwort").unwrap();
        assert!(verify_password(&hash, "ein-langes-passwort"));
        assert!(!verify_password(&hash, "falsch-falsch-falsch"));
        assert!(!verify_password("kein-hash", "x"));

        let s = Sessions::default();
        let t = s.create("admin", Role::Admin);
        assert_eq!(s.user(&t), Some(("admin".into(), Role::Admin)));
        s.revoke(&t);
        assert_eq!(s.user(&t), None);
        let a = s.create("lena", Role::Editor);
        let b = s.create("lena", Role::Editor);
        s.revoke_user("lena");
        assert!(s.user(&a).is_none() && s.user(&b).is_none());

        for _ in 0..MAX_FAILURES {
            assert!(s.check_lockout().is_ok());
            s.record_failure();
        }
        assert!(s.check_lockout().is_err());
    }
}
