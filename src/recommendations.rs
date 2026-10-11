//! TwoGG-Auswahl: kuratierte Projekte je Inhaltsart für „Entdecken“ im Launcher. Öffentlich
//! lesbar, in der Admin-Seite gepflegt.

use axum::Json;
use axum::extract::{Path, Query, State as AxumState};
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};

use crate::admin::{ListQuery, check_media, content_range, list};
use crate::auth::Admin;
use crate::db::{self, Recommendation};
use crate::error::null_default;
use crate::files::media_url;
use crate::{ApiError, ApiResult, JsonBody, State};

const TYPES: [&str; 5] = ["modpack", "mod", "resourcepack", "shader", "datapack"];
const SOURCES: [&str; 3] = ["twogg", "modrinth", "curseforge"];
const MOTIFS: [&str; 8] = [
    "shield",
    "block",
    "house",
    "sliders",
    "waves",
    "snowflake",
    "server",
    "bolt",
];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecommendationDto {
    #[serde(flatten)]
    rec: Recommendation,
    icon_url: Option<String>,
}

fn dto(state: &State, rec: Recommendation) -> RecommendationDto {
    RecommendationDto {
        icon_url: rec
            .icon
            .as_deref()
            .map(|i| media_url(&state.config.public_url, i)),
        rec,
    }
}

#[derive(Deserialize)]
pub struct TypeQuery {
    #[serde(rename = "type")]
    project_type: Option<String>,
}

/// Sichtbare Empfehlungen, optional nur einer Inhaltsart.
pub async fn list_public(
    AxumState(state): AxumState<State>,
    Query(q): Query<TypeQuery>,
) -> ApiResult<Json<Vec<RecommendationDto>>> {
    let rows: Vec<Recommendation> = match q.project_type.as_deref() {
        Some(t) if TYPES.contains(&t) => {
            sqlx::query_as(
                "SELECT * FROM recommendations WHERE visible = 1 AND project_type = ? ORDER BY sort_order, created_at",
            )
            .bind(t)
            .fetch_all(&state.db)
            .await?
        }
        Some(_) => return Err(ApiError::bad_request("Unbekannte Inhaltsart")),
        None => {
            sqlx::query_as("SELECT * FROM recommendations WHERE visible = 1 ORDER BY sort_order, created_at")
                .fetch_all(&state.db)
                .await?
        }
    };
    Ok(Json(rows.into_iter().map(|r| dto(&state, r)).collect()))
}

pub async fn list_admin(
    AxumState(state): AxumState<State>,
    _: Admin,
    Query(q): Query<ListQuery>,
) -> ApiResult<(HeaderMap, Json<Vec<RecommendationDto>>)> {
    let p = q.parse(
        &[
            ("sortOrder", "sort_order"),
            ("title", "title"),
            ("projectType", "project_type"),
            ("createdAt", "created_at"),
        ],
        false,
    )?;
    let (rows, total) = list::<Recommendation>(
        &state,
        "recommendations",
        &p,
        "title",
        &[
            ("projectType", "project_type"),
            ("source", "source"),
            ("visible", "visible"),
        ],
    )
    .await?;
    let out: Vec<RecommendationDto> = rows.into_iter().map(|r| dto(&state, r)).collect();
    Ok((
        content_range("recommendations", p.offset, out.len(), total),
        Json(out),
    ))
}

async fn load(state: &State, id: &str) -> ApiResult<Json<RecommendationDto>> {
    let r: Recommendation = sqlx::query_as("SELECT * FROM recommendations WHERE id = ?")
        .bind(id)
        .fetch_one(&state.db)
        .await?;
    Ok(Json(dto(state, r)))
}

pub async fn get(
    AxumState(state): AxumState<State>,
    _: Admin,
    Path(id): Path<String>,
) -> ApiResult<Json<RecommendationDto>> {
    load(&state, &id).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Input {
    project_type: String,
    source: String,
    project_id: String,
    title: String,
    #[serde(default, deserialize_with = "null_default")]
    subtitle: String,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    motif: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    sort_order: i64,
    #[serde(default = "yes")]
    visible: bool,
}

fn yes() -> bool {
    true
}

fn check(input: &Input) -> ApiResult<()> {
    if !TYPES.contains(&input.project_type.as_str()) {
        return Err(ApiError::bad_request(
            "Inhaltsart: modpack, mod, resourcepack, shader oder datapack",
        ));
    }
    if !SOURCES.contains(&input.source.as_str()) {
        return Err(ApiError::bad_request(
            "Quelle: twogg, modrinth oder curseforge",
        ));
    }
    if input.source == "twogg" && input.project_type != "modpack" {
        return Err(ApiError::bad_request("TwoGG-Packs gibt es nur als Modpack"));
    }
    let id = input.project_id.trim();
    let id_ok = !id.is_empty()
        && id.len() <= 64
        && match input.source.as_str() {
            "curseforge" => id.bytes().all(|b| b.is_ascii_digit()),
            _ => id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        };
    if !id_ok {
        return Err(ApiError::bad_request(
            "Projekt: Modrinth-ID oder -Slug, CurseForge-Zahl bzw. TwoGG-Pack-ID",
        ));
    }
    if input.title.trim().is_empty() {
        return Err(ApiError::bad_request("Bitte einen Titel angeben"));
    }
    if let Some(m) = input.motif.as_deref().filter(|m| !m.is_empty())
        && !MOTIFS.contains(&m)
    {
        return Err(ApiError::bad_request("Unbekanntes Motiv"));
    }
    check_media(&input.icon)
}

pub async fn create(
    AxumState(state): AxumState<State>,
    _: Admin,
    JsonBody(input): JsonBody<Input>,
) -> ApiResult<Json<RecommendationDto>> {
    check(&input)?;
    let id = uuid::Uuid::new_v4().simple().to_string();
    sqlx::query(
        "INSERT INTO recommendations (id, project_type, source, project_id, title, subtitle, icon, motif, sort_order, visible, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&input.project_type)
    .bind(&input.source)
    .bind(input.project_id.trim())
    .bind(input.title.trim())
    .bind(input.subtitle.trim())
    .bind(&input.icon)
    .bind(input.motif.as_deref().filter(|m| !m.is_empty()))
    .bind(input.sort_order)
    .bind(input.visible)
    .bind(db::now())
    .execute(&state.db)
    .await?;
    load(&state, &id).await
}

pub async fn update(
    AxumState(state): AxumState<State>,
    _: Admin,
    Path(id): Path<String>,
    JsonBody(input): JsonBody<Input>,
) -> ApiResult<Json<RecommendationDto>> {
    check(&input)?;
    let done = sqlx::query(
        "UPDATE recommendations SET project_type = ?, source = ?, project_id = ?, title = ?, subtitle = ?, \
         icon = ?, motif = ?, sort_order = ?, visible = ? WHERE id = ?",
    )
    .bind(&input.project_type)
    .bind(&input.source)
    .bind(input.project_id.trim())
    .bind(input.title.trim())
    .bind(input.subtitle.trim())
    .bind(&input.icon)
    .bind(input.motif.as_deref().filter(|m| !m.is_empty()))
    .bind(input.sort_order)
    .bind(input.visible)
    .bind(&id)
    .execute(&state.db)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    load(&state, &id).await
}

pub async fn delete(
    AxumState(state): AxumState<State>,
    _: Admin,
    Path(id): Path<String>,
) -> ApiResult<Json<RecommendationDto>> {
    let out = load(&state, &id).await?;
    sqlx::query("DELETE FROM recommendations WHERE id = ?")
        .bind(&id)
        .execute(&state.db)
        .await?;
    Ok(out)
}
