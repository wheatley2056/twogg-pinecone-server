//! Nutzungsstatistik ohne Personenbezug (F-59).
//!
//! Gespeichert werden nur Tageszähler (`daily_stats`): Downloads je Pack-Version, Launcher je
//! Version und aktive Freunde-Profile. Für „Launcher je Version“ merkt sich der Server pro Tag
//! nur im Speicher gesalzene Hashes aus IP und Version, damit ein Launcher am Tag einmal zählt.
//! Nach einem Neustart kann ein Launcher am selben Tag doppelt zählen. Zähler älter als
//! [`RETENTION_DAYS`] löscht das tägliche Aufräumen.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::net::IpAddr;
use std::sync::Mutex;

use axum::Json;
use axum::extract::{Query, State as AxumState};
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};

use crate::State;
use crate::auth::Admin;
use crate::db;
use crate::error::ApiResult;

pub const RETENTION_DAYS: i64 = 400;
const MAX_DAYS: i64 = 365;

pub const DOWNLOAD: &str = "download";
pub const LAUNCHER: &str = "launcher";
pub const ACTIVE: &str = "active";

/// Tag seit 1970 (UTC).
pub fn day(secs: i64) -> i64 {
    secs.div_euclid(86_400)
}

/// Zähler des heutigen Tages um eins erhöhen. Fehler werden nur protokolliert, damit die
/// Statistik nie eine Anfrage scheitern lässt.
pub async fn bump(state: &State, metric: &str, key: &str) {
    let res = sqlx::query(
        "INSERT INTO daily_stats (day, metric, key, count) VALUES (?, ?, ?, 1) \
         ON CONFLICT (day, metric, key) DO UPDATE SET count = count + 1",
    )
    .bind(day(db::now()))
    .bind(metric)
    .bind(key)
    .execute(&state.db)
    .await;
    if let Err(e) = res {
        tracing::warn!("Statistik {metric}: {e}");
    }
}

/// Aktives Freunde-Profil zählen, wenn es heute zum ersten Mal auftaucht (Anmeldung oder
/// Präsenz). `last_seen` ist der Stand vor der Aktualisierung, `None` für neue Profile.
pub async fn mark_active(state: &State, last_seen: Option<i64>) {
    if last_seen.map(day) != Some(day(db::now())) {
        bump(state, ACTIVE, "").await;
    }
}

/// Launcher-Version aus `X-Pinecone-Client` oder `User-Agent` (`pinecone/<version>`), nur `x.y.z`.
pub fn launcher_version(headers: &HeaderMap) -> Option<String> {
    ["x-pinecone-client", "user-agent"].iter().find_map(|name| {
        let v = headers
            .get(*name)?
            .to_str()
            .ok()?
            .strip_prefix("pinecone/")?;
        crate::settings::valid_version(v).then(|| v.to_string())
    })
}

/// Launcher, die heute schon gezählt wurden (nur im Speicher).
#[derive(Default)]
pub struct SeenToday {
    inner: Mutex<(i64, u64, HashSet<u64>)>,
}

impl SeenToday {
    /// true, wenn diese Kombination heute zum ersten Mal auftaucht.
    pub fn first(&self, ip: IpAddr, version: &str) -> bool {
        let today = day(db::now());
        let mut guard = self.inner.lock().unwrap();
        let (day, salt, set) = &mut *guard;
        if *day != today {
            *day = today;
            *salt = rand::random();
            set.clear();
        }
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (*salt, ip, version).hash(&mut h);
        set.insert(h.finish())
    }
}

/// Launcher zählen, einmal je IP, Version und Tag.
pub async fn count_launcher(state: &State, ip: IpAddr, headers: &HeaderMap) {
    if let Some(version) = launcher_version(headers)
        && state.seen_today.first(ip, &version)
    {
        bump(state, LAUNCHER, &version).await;
    }
}

pub async fn cleanup(state: &State) -> anyhow::Result<u64> {
    let res = sqlx::query("DELETE FROM daily_stats WHERE day < ?")
        .bind(day(db::now()) - RETENTION_DAYS)
        .execute(&state.db)
        .await?;
    Ok(res.rows_affected())
}

#[derive(Deserialize)]
pub struct DailyQuery {
    days: Option<i64>,
}

#[derive(Serialize)]
pub struct Series {
    key: String,
    total: i64,
    counts: Vec<i64>,
}

#[derive(Serialize)]
pub struct Daily {
    /// Erster Tag (Tage seit 1970, UTC); `counts[i]` gehört zu `from + i`.
    from: i64,
    days: i64,
    active: Vec<i64>,
    launchers: Vec<Series>,
    downloads: Vec<Series>,
}

/// `GET /api/admin/stats/daily?days=30`: Tageszähler, bis einschließlich heute.
pub async fn daily(
    AxumState(state): AxumState<State>,
    _: Admin,
    Query(q): Query<DailyQuery>,
) -> ApiResult<Json<Daily>> {
    let days = q.days.unwrap_or(30).clamp(1, MAX_DAYS);
    let from = day(db::now()) - days + 1;
    let rows: Vec<(i64, String, String, i64)> = sqlx::query_as(
        "SELECT day, metric, key, count FROM daily_stats WHERE day >= ? ORDER BY day",
    )
    .bind(from)
    .fetch_all(&state.db)
    .await?;
    let mut active = vec![0; days as usize];
    let mut launchers: Vec<Series> = Vec::new();
    let mut downloads: Vec<Series> = Vec::new();
    for (d, metric, key, count) in rows {
        let i = (d - from) as usize;
        if i >= days as usize {
            continue;
        }
        let list = match metric.as_str() {
            ACTIVE => {
                active[i] += count;
                continue;
            }
            LAUNCHER => &mut launchers,
            DOWNLOAD => &mut downloads,
            _ => continue,
        };
        let pos = match list.iter().position(|s| s.key == key) {
            Some(p) => p,
            None => {
                list.push(Series {
                    key,
                    total: 0,
                    counts: vec![0; days as usize],
                });
                list.len() - 1
            }
        };
        list[pos].counts[i] += count;
        list[pos].total += count;
    }
    launchers.sort_by(|a, b| b.total.cmp(&a.total).then(a.key.cmp(&b.key)));
    downloads.sort_by(|a, b| b.total.cmp(&a.total).then(a.key.cmp(&b.key)));
    Ok(Json(Daily {
        from,
        days,
        active,
        launchers,
        downloads,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seen_once_per_day() {
        let seen = SeenToday::default();
        let ip: IpAddr = "203.0.113.5".parse().unwrap();
        assert!(seen.first(ip, "0.9.8"));
        assert!(!seen.first(ip, "0.9.8"));
        assert!(seen.first(ip, "0.9.9"));
        assert_eq!(day(86_399), 0);
        assert_eq!(day(86_400), 1);
    }
}
