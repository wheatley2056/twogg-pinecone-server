//! Admin-API für React-Admin (`ra-data-simple-rest`-Format): Listen mit `sort`, `range`, `filter`
//! und `Content-Range`-Header, dazu Anmeldung, Uploads (Modpack-Versionen, Bilder) und Statistik.

use axum::extract::{Multipart, Path, Query, State as AxumState};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{QueryBuilder, Sqlite};

use crate::auth::Admin;
use crate::db::{self, News, Pack, Version, Visibility};
use crate::error::null_default;
use crate::files::{self, media_url};
use crate::{ApiError, ApiResult, JsonBody, MAX_UPLOAD, State};

pub fn routes(state: State) -> Router<State> {
    Router::new()
        .merge(crate::accounts::routes())
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/me", get(me))
        .route("/stats", get(stats))
        .route("/media", post(upload_media))
        .route("/packs", get(list_packs).post(create_pack))
        .route(
            "/packs/{id}",
            get(get_pack).put(update_pack).delete(delete_pack),
        )
        .route("/versions", get(list_versions).post(create_version))
        .route(
            "/versions/{id}",
            get(get_version).put(update_version).delete(delete_version),
        )
        .route(
            "/recommendations",
            get(crate::recommendations::list_admin).post(crate::recommendations::create),
        )
        .route(
            "/recommendations/{id}",
            get(crate::recommendations::get)
                .put(crate::recommendations::update)
                .delete(crate::recommendations::delete),
        )
        .route(
            "/settings/main",
            get(crate::settings::get).put(crate::settings::put),
        )
        .route("/news", get(list_news).post(create_news))
        .route(
            "/news/{id}",
            get(get_news).put(update_news).delete(delete_news),
        )
        .layer(axum::middleware::from_fn_with_state(
            state,
            crate::accounts::guard,
        ))
}

// ---------- Anmeldung ----------

#[derive(Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

async fn login(
    AxumState(state): AxumState<State>,
    JsonBody(req): JsonBody<LoginRequest>,
) -> ApiResult<Json<Value>> {
    if let Err(secs) = state.sessions.check_lockout() {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            format!("Zu viele Fehlversuche, bitte {secs} s warten"),
        ));
    }
    let role = crate::accounts::verify(&state, req.username.trim(), &req.password).await?;
    let Some(role) = role else {
        state.sessions.record_failure();
        tracing::warn!("Fehlgeschlagene Admin-Anmeldung für {:?}", req.username);
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "Benutzername oder Passwort falsch",
        ));
    };
    state.sessions.record_success();
    let user = req.username.trim();
    let token = state.sessions.create(user, role);
    tracing::info!("Admin {user} angemeldet ({})", role.as_str());
    crate::accounts::record(&state, user, "login", "session", None).await;
    Ok(Json(
        serde_json::json!({ "token": token, "user": user, "role": role.as_str() }),
    ))
}

async fn logout(AxumState(state): AxumState<State>, admin: Admin) -> StatusCode {
    state.sessions.revoke(&admin.token);
    StatusCode::NO_CONTENT
}

async fn me(admin: Admin) -> Json<Value> {
    Json(
        serde_json::json!({ "id": admin.user, "fullName": admin.user, "role": admin.role.as_str() }),
    )
}

async fn stats(AxumState(state): AxumState<State>, _: Admin) -> ApiResult<Json<Value>> {
    let (packs, public): (i64, i64) =
        sqlx::query_as("SELECT COUNT(*), COALESCE(SUM(visibility = 'public'), 0) FROM packs")
            .fetch_one(&state.db)
            .await?;
    let (versions, downloads): (i64, i64) =
        sqlx::query_as("SELECT COUNT(*), COALESCE(SUM(downloads), 0) FROM versions")
            .fetch_one(&state.db)
            .await?;
    let (news,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM news")
        .fetch_one(&state.db)
        .await?;
    Ok(Json(serde_json::json!({
        "packs": packs, "publicPacks": public, "versions": versions,
        "downloads": downloads, "news": news,
    })))
}

/// Bild hochladen (Pack-Icon, Bild einer Neuigkeit); liefert den Dateinamen für `icon`/`image`.
async fn upload_media(
    AxumState(state): AxumState<State>,
    _: Admin,
    mut multipart: Multipart,
) -> ApiResult<Json<Value>> {
    while let Some(field) = next_field(&mut multipart).await? {
        if field.name() == Some("file") {
            let name = files::store_image(field, &state.config.data_dir).await?;
            return Ok(Json(serde_json::json!({
                "name": name, "url": media_url(&state.config.public_url, &name),
            })));
        }
    }
    Err(ApiError::bad_request("Feld „file“ fehlt"))
}

async fn next_field(
    multipart: &mut Multipart,
) -> ApiResult<Option<axum::extract::multipart::Field<'_>>> {
    multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(format!("Upload fehlerhaft: {e}")))
}

// ---------- Listen im React-Admin-Format ----------

#[derive(Deserialize, Default)]
pub(crate) struct ListQuery {
    /// `["feld","ASC"]`
    sort: Option<String>,
    /// `[0,24]`
    range: Option<String>,
    /// `{"q":"…","id":[…],…}`
    filter: Option<String>,
}

pub(crate) struct ListParams {
    sort: String,
    desc: bool,
    pub(crate) offset: i64,
    limit: i64,
    filter: serde_json::Map<String, Value>,
}

impl ListQuery {
    /// `sortable`: erlaubte Felder (API-Name → Spalte); das erste ist der Standard.
    pub(crate) fn parse(
        &self,
        sortable: &[(&str, &str)],
        default_desc: bool,
    ) -> ApiResult<ListParams> {
        let bad = |what: &str| ApiError::bad_request(format!("Ungültiger Parameter {what}"));
        let (mut sort, mut desc) = (sortable[0].1.to_string(), default_desc);
        if let Some(s) = &self.sort {
            let v: (String, String) = serde_json::from_str(s).map_err(|_| bad("sort"))?;
            sort = sortable
                .iter()
                .find(|(api, _)| *api == v.0)
                .map(|(_, col)| col.to_string())
                .ok_or_else(|| bad("sort"))?;
            desc = v.1.eq_ignore_ascii_case("DESC");
        }
        let (start, end) = match &self.range {
            Some(r) => serde_json::from_str::<(i64, i64)>(r).map_err(|_| bad("range"))?,
            None => (0, 99),
        };
        if start < 0 || end < start {
            return Err(bad("range"));
        }
        let filter = match &self.filter {
            Some(f) => serde_json::from_str(f).map_err(|_| bad("filter"))?,
            None => Default::default(),
        };
        Ok(ListParams {
            sort,
            desc,
            offset: start,
            limit: (end - start + 1).min(500),
            filter,
        })
    }
}

impl ListParams {
    pub(crate) fn sort_column(&self) -> &str {
        &self.sort
    }

    pub(crate) fn desc(&self) -> bool {
        self.desc
    }

    pub(crate) fn limit(&self) -> i64 {
        self.limit
    }

    /// Einfacher Textfilter (`{"feld":"wert"}`), sonst `None`.
    pub(crate) fn filter_str(&self, key: &str) -> Option<String> {
        self.filter
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::to_string)
    }
}

pub(crate) fn content_range(resource: &str, offset: i64, count: usize, total: i64) -> HeaderMap {
    let end = if count == 0 {
        offset
    } else {
        offset + count as i64 - 1
    };
    let mut h = HeaderMap::new();
    h.insert(
        "content-range",
        HeaderValue::from_str(&format!("{resource} {offset}-{end}/{total}")).unwrap(),
    );
    h
}

/// Fügt `WHERE`-Bedingungen für gemeinsame Filter an (`id`-Liste, Suche, Gleichheit).
fn push_filters<'a>(
    qb: &mut QueryBuilder<'a, Sqlite>,
    filter: &'a serde_json::Map<String, Value>,
    search_col: &str,
    equal: &[(&str, &str)],
) {
    qb.push(" WHERE 1=1");
    if let Some(ids) = filter.get("id").and_then(Value::as_array) {
        qb.push(" AND id IN (");
        let mut sep = qb.separated(", ");
        for id in ids {
            sep.push_bind(id.as_str().unwrap_or_default().to_string());
        }
        sep.push_unseparated(")");
        if ids.is_empty() {
            qb.push(" AND 0");
        }
    }
    if let Some(q) = filter
        .get("q")
        .and_then(Value::as_str)
        .filter(|q| !q.is_empty())
    {
        qb.push(format!(" AND {search_col} LIKE "));
        qb.push_bind(format!("%{}%", q.replace(['%', '_'], "")));
    }
    for (api, col) in equal {
        match filter.get(*api) {
            Some(Value::String(s)) => {
                qb.push(format!(" AND {col} = "));
                qb.push_bind(s.clone());
            }
            Some(Value::Bool(b)) => {
                qb.push(format!(" AND {col} = "));
                qb.push_bind(*b);
            }
            _ => {}
        }
    }
}

pub(crate) async fn list<T>(
    state: &State,
    table: &str,
    p: &ListParams,
    search_col: &str,
    equal: &[(&str, &str)],
) -> ApiResult<(Vec<T>, i64)>
where
    T: for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Unpin,
{
    let mut count = QueryBuilder::<Sqlite>::new(format!("SELECT COUNT(*) FROM {table}"));
    push_filters(&mut count, &p.filter, search_col, equal);
    let (total,): (i64,) = count.build_query_as().fetch_one(&state.db).await?;

    let mut qb = QueryBuilder::<Sqlite>::new(format!("SELECT * FROM {table}"));
    push_filters(&mut qb, &p.filter, search_col, equal);
    // Spaltenname stammt aus der Whitelist, nicht aus der Anfrage.
    qb.push(format!(
        " ORDER BY {} {} LIMIT ",
        p.sort,
        if p.desc { "DESC" } else { "ASC" }
    ));
    qb.push_bind(p.limit);
    qb.push(" OFFSET ");
    qb.push_bind(p.offset);
    let rows = qb.build_query_as::<T>().fetch_all(&state.db).await?;
    Ok((rows, total))
}

// ---------- Packs ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AdminPack {
    #[serde(flatten)]
    pack: Pack,
    icon_url: Option<String>,
    version_count: i64,
    downloads: i64,
}

async fn admin_pack(state: &State, pack: Pack) -> ApiResult<AdminPack> {
    let (version_count, downloads): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(downloads), 0) FROM versions WHERE pack_id = ?",
    )
    .bind(&pack.id)
    .fetch_one(&state.db)
    .await?;
    Ok(AdminPack {
        icon_url: pack
            .icon
            .as_deref()
            .map(|i| media_url(&state.config.public_url, i)),
        pack,
        version_count,
        downloads,
    })
}

async fn list_packs(
    AxumState(state): AxumState<State>,
    _: Admin,
    Query(q): Query<ListQuery>,
) -> ApiResult<(HeaderMap, Json<Vec<AdminPack>>)> {
    let p = q.parse(
        &[
            ("updatedAt", "updated_at"),
            ("title", "title"),
            ("id", "id"),
            ("sortOrder", "sort_order"),
            ("createdAt", "created_at"),
            ("visibility", "visibility"),
        ],
        true,
    )?;
    let (rows, total) = list::<Pack>(
        &state,
        "packs",
        &p,
        "title",
        &[("visibility", "visibility"), ("featured", "featured")],
    )
    .await?;
    let mut out = Vec::new();
    for r in rows {
        out.push(admin_pack(&state, r).await?);
    }
    Ok((
        content_range("packs", p.offset, out.len(), total),
        Json(out),
    ))
}

async fn load_pack(state: &State, id: &str) -> ApiResult<Json<AdminPack>> {
    let pack = sqlx::query_as("SELECT * FROM packs WHERE id = ?")
        .bind(id)
        .fetch_one(&state.db)
        .await?;
    Ok(Json(admin_pack(state, pack).await?))
}

async fn get_pack(
    AxumState(state): AxumState<State>,
    _: Admin,
    Path(id): Path<String>,
) -> ApiResult<Json<AdminPack>> {
    load_pack(&state, &id).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackInput {
    #[serde(default)]
    id: Option<String>,
    title: String,
    #[serde(default, deserialize_with = "null_default")]
    summary: String,
    #[serde(default, deserialize_with = "null_default")]
    description: String,
    #[serde(default)]
    icon: Option<String>,
    visibility: Option<Visibility>,
    #[serde(default, deserialize_with = "null_default")]
    featured: bool,
    #[serde(default, deserialize_with = "null_default")]
    sort_order: i64,
    /// twogg.app-Event-ID (UUID); leer = keinem Event zugeordnet.
    #[serde(default)]
    event_id: Option<String>,
}

/// twogg.app-Event-IDs sind UUIDs; leer heißt „kein Event“.
fn event_id(input: &Option<String>) -> ApiResult<Option<String>> {
    match input.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(id)
            if id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') =>
        {
            Ok(Some(id.to_string()))
        }
        Some(_) => Err(ApiError::bad_request(
            "Ungültige Event-ID (die UUID aus dem twogg.app-Link)",
        )),
    }
}

pub(crate) fn check_media(name: &Option<String>) -> ApiResult<()> {
    match name {
        Some(n) if n.contains(['/', '\\']) || n.starts_with('.') => {
            Err(ApiError::bad_request("Ungültiger Bildname"))
        }
        _ => Ok(()),
    }
}

async fn create_pack(
    AxumState(state): AxumState<State>,
    _: Admin,
    JsonBody(input): JsonBody<PackInput>,
) -> ApiResult<Json<AdminPack>> {
    let id = input.id.clone().unwrap_or_default();
    if !db::valid_pack_id(&id) {
        return Err(ApiError::bad_request(
            "Die ID braucht 2–48 Zeichen: a–z, 0–9 und Bindestriche (erscheint in Links)",
        ));
    }
    if input.title.trim().is_empty() {
        return Err(ApiError::bad_request("Bitte einen Titel angeben"));
    }
    check_media(&input.icon)?;
    let event = event_id(&input.event_id)?;
    let now = db::now();
    let result = sqlx::query(
        "INSERT INTO packs (id, title, summary, description, icon, visibility, featured, sort_order, created_at, updated_at, event_id) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(input.title.trim())
    .bind(&input.summary)
    .bind(&input.description)
    .bind(&input.icon)
    .bind(input.visibility.unwrap_or(Visibility::Hidden))
    .bind(input.featured)
    .bind(input.sort_order)
    .bind(now)
    .bind(now)
    .bind(&event)
    .execute(&state.db)
    .await;
    if let Err(sqlx::Error::Database(e)) = &result
        && e.is_unique_violation()
    {
        return Err(ApiError::bad_request(format!(
            "Die ID {id} ist schon vergeben"
        )));
    }
    result?;
    load_pack(&state, &id).await
}

async fn update_pack(
    AxumState(state): AxumState<State>,
    _: Admin,
    Path(id): Path<String>,
    JsonBody(input): JsonBody<PackInput>,
) -> ApiResult<Json<AdminPack>> {
    if input.title.trim().is_empty() {
        return Err(ApiError::bad_request("Bitte einen Titel angeben"));
    }
    check_media(&input.icon)?;
    let event = event_id(&input.event_id)?;
    let done = sqlx::query(
        "UPDATE packs SET title = ?, summary = ?, description = ?, icon = ?, visibility = ?, \
         featured = ?, sort_order = ?, updated_at = ?, event_id = ? WHERE id = ?",
    )
    .bind(input.title.trim())
    .bind(&input.summary)
    .bind(&input.description)
    .bind(&input.icon)
    .bind(input.visibility.unwrap_or(Visibility::Hidden))
    .bind(input.featured)
    .bind(input.sort_order)
    .bind(db::now())
    .bind(&event)
    .bind(&id)
    .execute(&state.db)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    load_pack(&state, &id).await
}

async fn delete_pack(
    AxumState(state): AxumState<State>,
    _: Admin,
    Path(id): Path<String>,
) -> ApiResult<Json<AdminPack>> {
    let pack: Pack = sqlx::query_as("SELECT * FROM packs WHERE id = ?")
        .bind(&id)
        .fetch_one(&state.db)
        .await?;
    let out = admin_pack(&state, pack).await?;
    let versions = db::versions_of(&state.db, &id).await?;
    sqlx::query("DELETE FROM packs WHERE id = ?")
        .bind(&id)
        .execute(&state.db)
        .await?;
    for v in versions {
        let _ = tokio::fs::remove_file(files::packs_dir(&state.config.data_dir).join(v.file)).await;
    }
    tracing::info!("Pack {id} gelöscht");
    Ok(Json(out))
}

// ---------- Versionen ----------

async fn list_versions(
    AxumState(state): AxumState<State>,
    _: Admin,
    Query(q): Query<ListQuery>,
) -> ApiResult<(HeaderMap, Json<Vec<Version>>)> {
    let p = q.parse(
        &[
            ("publishedAt", "published_at"),
            ("version", "version"),
            ("minecraft", "minecraft"),
            ("downloads", "downloads"),
            ("channel", "channel"),
        ],
        true,
    )?;
    let (rows, total) = list::<Version>(
        &state,
        "versions",
        &p,
        "version",
        &[("packId", "pack_id"), ("channel", "channel")],
    )
    .await?;
    Ok((
        content_range("versions", p.offset, rows.len(), total),
        Json(rows),
    ))
}

async fn load_version(state: &State, id: &str) -> ApiResult<Json<Version>> {
    Ok(Json(
        sqlx::query_as("SELECT * FROM versions WHERE id = ?")
            .bind(id)
            .fetch_one(&state.db)
            .await?,
    ))
}

async fn get_version(
    AxumState(state): AxumState<State>,
    _: Admin,
    Path(id): Path<String>,
) -> ApiResult<Json<Version>> {
    load_version(&state, &id).await
}

/// Neue Version: Multipart mit `packId`, `file` (.mrpack), optional `channel`, `changelog`,
/// `version` (sonst aus dem Pack). Minecraft-Version und Loader kommen aus dem Pack.
async fn create_version(
    AxumState(state): AxumState<State>,
    _: Admin,
    mut multipart: Multipart,
) -> ApiResult<Json<Version>> {
    let (mut pack_id, mut channel, mut changelog, mut label) = (
        None::<String>,
        None::<String>,
        String::new(),
        None::<String>,
    );
    let id = uuid::Uuid::new_v4().simple().to_string();
    let dir = files::packs_dir(&state.config.data_dir);
    let file_name = format!("{id}.mrpack");
    let mut stored = None;
    while let Some(field) = next_field(&mut multipart).await? {
        match field.name().unwrap_or_default() {
            "file" => {
                stored = Some(files::store_field(field, &dir, &file_name, MAX_UPLOAD).await?);
            }
            name => {
                let name = name.to_string();
                let text = field
                    .text()
                    .await
                    .map_err(|e| ApiError::bad_request(format!("Feld {name}: {e}")))?;
                match name.as_str() {
                    "packId" => pack_id = Some(text),
                    "channel" => channel = Some(text),
                    "changelog" => changelog = text,
                    "version" => label = Some(text).filter(|t| !t.trim().is_empty()),
                    _ => {}
                }
            }
        }
    }
    let stored = stored.ok_or_else(|| ApiError::bad_request("Bitte eine .mrpack-Datei wählen"))?;
    let cleanup = |e: ApiError| {
        let _ = std::fs::remove_file(&stored.path);
        e
    };
    let pack_id = pack_id.ok_or_else(|| cleanup(ApiError::bad_request("packId fehlt")))?;
    let channel = channel.unwrap_or_else(|| "release".into());
    if !matches!(channel.as_str(), "release" | "beta" | "alpha") {
        return Err(cleanup(ApiError::bad_request(
            "Kanal: release, beta oder alpha",
        )));
    }
    let exists: Option<(String,)> = sqlx::query_as("SELECT id FROM packs WHERE id = ?")
        .bind(&pack_id)
        .fetch_optional(&state.db)
        .await?;
    if exists.is_none() {
        return Err(cleanup(ApiError::bad_request("Dieses Pack gibt es nicht")));
    }
    // Inhalt prüfen wie der Launcher (Format, Pfade, Hashes, Loader).
    let path = stored.path.clone();
    let parsed = tokio::task::spawn_blocking(move || crate::mrpack::read(&path))
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| cleanup(ApiError::bad_request(format!("Kein gültiges Modpack: {e}"))))?;
    let version = label.unwrap_or(parsed.version.clone());
    let (loader, loader_version) = parsed.loader.clone().unzip();
    let now = db::now();
    sqlx::query(
        "INSERT INTO versions (id, pack_id, version, minecraft, loader, loader_version, channel, changelog, file, size, sha1, sha512, published_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&pack_id)
    .bind(version.trim())
    .bind(&parsed.minecraft)
    .bind(&loader)
    .bind(&loader_version)
    .bind(&channel)
    .bind(&changelog)
    .bind(&file_name)
    .bind(stored.size as i64)
    .bind(&stored.sha1)
    .bind(&stored.sha512)
    .bind(now)
    .execute(&state.db)
    .await?;
    sqlx::query("UPDATE packs SET updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(&pack_id)
        .execute(&state.db)
        .await?;
    tracing::info!(
        "Version {version} für {pack_id} hochgeladen ({} MB)",
        stored.size / 1_000_000
    );
    load_version(&state, &id).await
}

#[derive(Deserialize)]
struct VersionInput {
    version: String,
    channel: String,
    #[serde(default, deserialize_with = "null_default")]
    changelog: String,
}

async fn update_version(
    AxumState(state): AxumState<State>,
    _: Admin,
    Path(id): Path<String>,
    JsonBody(input): JsonBody<VersionInput>,
) -> ApiResult<Json<Version>> {
    if !matches!(input.channel.as_str(), "release" | "beta" | "alpha")
        || input.version.trim().is_empty()
    {
        return Err(ApiError::bad_request(
            "Version und Kanal (release/beta/alpha) angeben",
        ));
    }
    sqlx::query("UPDATE versions SET version = ?, channel = ?, changelog = ? WHERE id = ?")
        .bind(input.version.trim())
        .bind(&input.channel)
        .bind(&input.changelog)
        .bind(&id)
        .execute(&state.db)
        .await?;
    load_version(&state, &id).await
}

async fn delete_version(
    AxumState(state): AxumState<State>,
    _: Admin,
    Path(id): Path<String>,
) -> ApiResult<Json<Version>> {
    let v: Version = sqlx::query_as("SELECT * FROM versions WHERE id = ?")
        .bind(&id)
        .fetch_one(&state.db)
        .await?;
    sqlx::query("DELETE FROM versions WHERE id = ?")
        .bind(&id)
        .execute(&state.db)
        .await?;
    let _ = tokio::fs::remove_file(files::packs_dir(&state.config.data_dir).join(&v.file)).await;
    Ok(Json(v))
}

// ---------- Neuigkeiten ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AdminNews {
    #[serde(flatten)]
    news: News,
    image_url: Option<String>,
}

fn admin_news(state: &State, news: News) -> AdminNews {
    AdminNews {
        image_url: news
            .image
            .as_deref()
            .map(|i| media_url(&state.config.public_url, i)),
        news,
    }
}

async fn list_news(
    AxumState(state): AxumState<State>,
    _: Admin,
    Query(q): Query<ListQuery>,
) -> ApiResult<(HeaderMap, Json<Vec<AdminNews>>)> {
    let p = q.parse(&[("publishedAt", "published_at"), ("title", "title")], true)?;
    let (rows, total) = list::<News>(
        &state,
        "news",
        &p,
        "title",
        &[("visible", "visible"), ("pinned", "pinned")],
    )
    .await?;
    let out: Vec<AdminNews> = rows.into_iter().map(|n| admin_news(&state, n)).collect();
    Ok((content_range("news", p.offset, out.len(), total), Json(out)))
}

async fn load_news(state: &State, id: &str) -> ApiResult<Json<AdminNews>> {
    let n: News = sqlx::query_as("SELECT * FROM news WHERE id = ?")
        .bind(id)
        .fetch_one(&state.db)
        .await?;
    Ok(Json(admin_news(state, n)))
}

async fn get_news(
    AxumState(state): AxumState<State>,
    _: Admin,
    Path(id): Path<String>,
) -> ApiResult<Json<AdminNews>> {
    load_news(&state, &id).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NewsInput {
    title: String,
    #[serde(default, deserialize_with = "null_default")]
    body: String,
    #[serde(default)]
    image: Option<String>,
    #[serde(default)]
    link: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    pinned: bool,
    #[serde(default = "yes")]
    visible: bool,
    /// Unix-Zeit; in der Zukunft = geplant.
    published_at: Option<i64>,
}

fn yes() -> bool {
    true
}

fn check_news(input: &NewsInput) -> ApiResult<()> {
    if input.title.trim().is_empty() {
        return Err(ApiError::bad_request("Bitte einen Titel angeben"));
    }
    if let Some(link) = input.link.as_deref().filter(|l| !l.is_empty())
        && !(link.starts_with("https://") || link.starts_with("http://"))
    {
        return Err(ApiError::bad_request("Der Link muss mit https:// beginnen"));
    }
    check_media(&input.image)
}

async fn create_news(
    AxumState(state): AxumState<State>,
    _: Admin,
    JsonBody(input): JsonBody<NewsInput>,
) -> ApiResult<Json<AdminNews>> {
    check_news(&input)?;
    let id = uuid::Uuid::new_v4().simple().to_string();
    sqlx::query(
        "INSERT INTO news (id, title, body, image, link, pinned, visible, published_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(input.title.trim())
    .bind(&input.body)
    .bind(&input.image)
    .bind(input.link.as_deref().filter(|l| !l.is_empty()))
    .bind(input.pinned)
    .bind(input.visible)
    .bind(input.published_at.unwrap_or_else(db::now))
    .execute(&state.db)
    .await?;
    load_news(&state, &id).await
}

async fn update_news(
    AxumState(state): AxumState<State>,
    _: Admin,
    Path(id): Path<String>,
    JsonBody(input): JsonBody<NewsInput>,
) -> ApiResult<Json<AdminNews>> {
    check_news(&input)?;
    let done = sqlx::query(
        "UPDATE news SET title = ?, body = ?, image = ?, link = ?, pinned = ?, visible = ?, \
         published_at = COALESCE(?, published_at) WHERE id = ?",
    )
    .bind(input.title.trim())
    .bind(&input.body)
    .bind(&input.image)
    .bind(input.link.as_deref().filter(|l| !l.is_empty()))
    .bind(input.pinned)
    .bind(input.visible)
    .bind(input.published_at)
    .bind(&id)
    .execute(&state.db)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    load_news(&state, &id).await
}

async fn delete_news(
    AxumState(state): AxumState<State>,
    _: Admin,
    Path(id): Path<String>,
) -> ApiResult<Json<AdminNews>> {
    let out = load_news(&state, &id).await?;
    sqlx::query("DELETE FROM news WHERE id = ?")
        .bind(&id)
        .execute(&state.db)
        .await?;
    Ok(out)
}
