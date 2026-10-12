//! Öffentliche Modpack-Seiten (`/packs`, `/packs/{id}`) im Stil einer CurseForge-Projektseite,
//! gestaltet wie Pinecone. „In Pinecone installieren“ öffnet `pinecone://twogg/<pack>[/<version>]`
//! im Launcher. Die Seiten kommen ohne Skripte aus (Tabs per CSS) und laden nur eigene Schriften.

use std::fmt::Write as _;

use axum::Router;
use axum::extract::{Path, State as AxumState};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::get;

use crate::State;
use crate::db::{self, Pack, Version};
use crate::files::media_url;
use crate::markdown;
use crate::settings::Settings;

/// Landing Page mit den Downloads des Launchers.
pub const DEFAULT_LAUNCHER_URL: &str = "https://wheatley2056.github.io/twogg-pinecone/";

pub fn routes() -> Router<State> {
    Router::new()
        .route("/packs", get(index))
        .route("/packs/", get(|| async { Redirect::permanent("/packs") }))
        .route("/packs/{id}", get(page))
        .route("/assets/{file}", get(asset))
}

/// Adresse der öffentlichen Seite eines Packs.
pub fn page_url(public_url: &str, id: &str) -> String {
    format!("{public_url}/packs/{id}")
}

// ---------------------------------------------------------------- Seiten

async fn index(AxumState(state): AxumState<State>) -> Response {
    let rows: Vec<Pack> = match sqlx::query_as(
        "SELECT * FROM packs WHERE visibility = 'public' \
         ORDER BY featured DESC, sort_order ASC, updated_at DESC",
    )
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => rows,
        Err(e) => return failure(e),
    };
    let mut cards = String::new();
    for p in rows {
        let versions = match db::versions_of(&state.db, &p.id).await {
            Ok(v) => v,
            Err(e) => return failure(e),
        };
        let Some(latest) = latest(&versions) else {
            continue;
        };
        let downloads: i64 = versions.iter().map(|v| v.downloads).sum();
        let _ = write!(
            cards,
            r#"<a class="card" href="/packs/{id}">
  {icon}
  <span class="card-body">
    <span class="card-title">{title}</span>
    <span class="card-summary">{summary}</span>
    <span class="card-meta"><span class="tag">{loader}</span><span>{dl}</span><span>{updated}</span></span>
  </span>
</a>"#,
            id = esc(&p.id),
            icon = icon_html(&state.config.public_url, &p, 64),
            title = esc(&p.title),
            summary = esc(&p.summary),
            loader = esc(&loader_label(latest)),
            dl = downloads_label(downloads),
            updated = esc(&format!("aktualisiert {}", date(p.updated_at))),
        );
    }
    if cards.is_empty() {
        cards.push_str(r#"<p class="empty">Noch keine Modpacks veröffentlicht.</p>"#);
    }
    let body = format!(
        r#"<section class="hero">
  <div class="wrap hero-inner">
    <div class="hero-text">
      <span class="label">TwoGG · Modpacks</span>
      <h1>TwoGG-Modpacks</h1>
      <p class="summary">Packs der TwoGG-Community für Events und Server. Installiert und aktuell gehalten mit Pinecone.</p>
    </div>
  </div>
</section>
<main class="wrap list">{cards}</main>"#
    );
    let launcher = state.config.launcher_url.clone();
    let legal = legal(&state).await;
    html_page(
        "TwoGG-Modpacks",
        "Modpacks der TwoGG-Community, installierbar mit dem Pinecone-Launcher.",
        None,
        &format!("{}/packs", state.config.public_url),
        &launcher,
        &legal,
        &body,
    )
}

async fn page(AxumState(state): AxumState<State>, Path(id): Path<String>) -> Response {
    let pack: Option<Pack> =
        match sqlx::query_as("SELECT * FROM packs WHERE id = ? AND visibility != 'hidden'")
            .bind(&id)
            .fetch_optional(&state.db)
            .await
        {
            Ok(p) => p,
            Err(e) => return failure(e),
        };
    let Some(p) = pack else {
        return not_found(&state.config.launcher_url, &legal(&state).await);
    };
    let versions = match db::versions_of(&state.db, &p.id).await {
        Ok(v) => v,
        Err(e) => return failure(e),
    };
    let base = &state.config.public_url;
    let launcher = &state.config.launcher_url;
    let legal = legal(&state).await;
    let downloads: i64 = versions.iter().map(|v| v.downloads).sum();
    let newest = latest(&versions);
    let link = format!("pinecone://twogg/{}", p.id);

    // Kopf: Logo, Titel, Kurzbeschreibung, Kennzahlen, Installieren.
    let mut facts = format!(
        r#"<span class="fact"><b>TwoGG</b></span><span class="fact">{}</span>"#,
        downloads_label(downloads)
    );
    let _ = write!(
        facts,
        r#"<span class="fact">aktualisiert {}</span>"#,
        date(p.updated_at)
    );
    if let Some(v) = newest {
        let _ = write!(
            facts,
            r#"<span class="tag">{}</span>"#,
            esc(&loader_label(v))
        );
    }
    let install = if newest.is_some() {
        format!(
            r#"<a class="btn harz big" href="{link}">{play}In Pinecone installieren</a>
      <span class="hint">Noch kein Pinecone? <a href="{launcher}">Launcher herunterladen</a></span>"#,
            link = esc(&link),
            play = ICON_PLAY,
            launcher = esc(launcher),
        )
    } else {
        r#"<span class="btn ghost big" aria-disabled="true">Noch keine Version</span>"#.into()
    };

    // Beschreibung
    let description = if p.description.trim().is_empty() {
        format!(r#"<p class="muted">{}</p>"#, esc(&p.summary))
    } else {
        markdown::to_html(&p.description)
    };

    // Versionen (Dateien)
    let mut rows = String::new();
    for v in &versions {
        let changelog = if v.changelog.trim().is_empty() {
            String::new()
        } else {
            format!(
                r#"<details><summary>Änderungen</summary><div class="prose">{}</div></details>"#,
                markdown::to_html(&v.changelog)
            )
        };
        let _ = write!(
            rows,
            r#"<div class="file">
  <span class="channel {chan}">{chan_label}</span>
  <span class="file-main"><span class="file-name">{version}</span>{changelog}</span>
  <span class="file-cell"><span class="k">Minecraft</span>{mc}</span>
  <span class="file-cell"><span class="k">Loader</span>{loader}</span>
  <span class="file-cell"><span class="k">Größe</span>{size}</span>
  <span class="file-cell"><span class="k">Veröffentlicht</span>{date}</span>
  <a class="btn small" href="{link}/{vid}" title="Diese Version in Pinecone installieren">{dl}Installieren</a>
</div>"#,
            chan = if v.channel == "release" {
                "release"
            } else {
                "beta"
            },
            chan_label = if v.channel == "release" { "R" } else { "B" },
            version = esc(&v.version),
            mc = esc(&v.minecraft),
            loader = esc(&loader_name(v)),
            size = size_label(v.size),
            date = date(v.published_at),
            link = esc(&link),
            vid = esc(&v.id),
            dl = ICON_DOWNLOAD,
        );
    }
    if rows.is_empty() {
        rows.push_str(r#"<p class="muted">Noch keine Version veröffentlicht.</p>"#);
    }

    // Seitenleiste
    let mut details = String::new();
    let mut fact = |k: &str, v: String| {
        let _ = write!(details, "<dt>{}</dt><dd>{}</dd>", esc(k), v);
    };
    if let Some(v) = newest {
        fact("Neueste Version", esc(&v.version));
        fact("Minecraft", esc(&v.minecraft));
        fact("Loader", esc(&loader_name(v)));
    }
    fact("Downloads", esc(&group(downloads)));
    fact("Erstellt", date(p.created_at));
    fact("Aktualisiert", date(p.updated_at));
    fact(
        "Projekt-ID",
        format!(r#"<span class="mono">{}</span>"#, esc(&p.id)),
    );
    let event = p
        .event_id
        .as_deref()
        .map(|e| {
            format!(
                r#"<section class="box event">
        <span class="label">TwoGG-Event</span>
        <p>Dieses Pack gehört zu einem Event auf twogg.app. Anmeldung, Teams und Termine findest du dort.</p>
        <a class="btn small" href="https://twogg.app/events/{}">Zum Event</a>
      </section>"#,
                esc(e)
            )
        })
        .unwrap_or_default();

    let count = versions.len();
    let body = format!(
        r#"<section class="hero">
  <div class="wrap hero-inner">
    {icon}
    <div class="hero-text">
      <span class="label">TwoGG · Modpack</span>
      <h1>{title}</h1>
      <p class="summary">{summary}</p>
      <div class="facts">{facts}</div>
    </div>
    <div class="cta">{install}</div>
  </div>
</section>
<div class="wrap layout">
  <div class="main">
    <input type="radio" name="tab" id="tab-desc" checked>
    <input type="radio" name="tab" id="tab-files">
    <nav class="tabs" aria-label="Bereiche">
      <label for="tab-desc">Beschreibung</label>
      <label for="tab-files">Versionen <span class="count">{count}</span></label>
    </nav>
    <section class="pane pane-desc prose">{description}</section>
    <section class="pane pane-files">{rows}</section>
  </div>
  <aside class="side">
    <section class="box">
      <span class="label">Über das Pack</span>
      <dl>{details}</dl>
    </section>
    {event}
    <section class="box">
      <span class="label">Pinecone</span>
      <p>Pinecone ist der Launcher der TwoGG-Community. Er installiert das Pack mit dem passenden Loader und hält es aktuell.</p>
      <a class="btn small" href="{launcher}">Launcher herunterladen</a>
    </section>
  </aside>
</div>"#,
        icon = icon_html(base, &p, 112),
        title = esc(&p.title),
        summary = esc(&p.summary),
        launcher = esc(launcher),
    );
    let image = p.icon.as_deref().map(|i| media_url(base, i));
    html_page(
        &p.title,
        &p.summary,
        image.as_deref(),
        &page_url(base, &p.id),
        launcher,
        &legal,
        &body,
    )
}

/// Rechtliche Angaben aus der Admin-Seite; bei Datenbankfehlern leer (der Pflichthinweis zu
/// Minecraft steht trotzdem im Fuß).
async fn legal(state: &State) -> Settings {
    crate::settings::load(state).await.unwrap_or_default()
}

/// Pflichthinweis nach den Minecraft-Nutzungsrichtlinien (minecraft.net/usage-guidelines).
pub const DISCLAIMER: &str = "Kein offizielles Minecraft-Produkt. Nicht von Mojang oder Microsoft genehmigt oder mit ihnen verbunden. NOT AN OFFICIAL MINECRAFT PRODUCT. NOT APPROVED BY OR ASSOCIATED WITH MOJANG OR MICROSOFT.";

/// Impressum, Datenschutz und Kontakt, soweit in der Admin-Seite gepflegt.
fn legal_links(legal: &Settings) -> String {
    let mut out = String::new();
    if !legal.imprint_url.is_empty() {
        let _ = write!(
            out,
            r#" · <a href="{}">Impressum</a>"#,
            esc(&legal.imprint_url)
        );
    }
    if !legal.privacy_url.is_empty() {
        let _ = write!(
            out,
            r#" · <a href="{}">Datenschutz</a>"#,
            esc(&legal.privacy_url)
        );
    }
    if !legal.contact_email.is_empty() {
        let _ = write!(
            out,
            r#" · <a href="mailto:{e}">{e}</a>"#,
            e = esc(&legal.contact_email)
        );
    }
    out
}

fn not_found(launcher: &str, legal: &Settings) -> Response {
    let body = r#"<section class="hero"><div class="wrap hero-inner"><div class="hero-text">
  <span class="label">404</span><h1>Pack nicht gefunden</h1>
  <p class="summary">Dieses Modpack gibt es nicht (mehr). <a href="/packs">Alle TwoGG-Modpacks</a></p>
</div></div></section>"#;
    let mut res = html_page("Nicht gefunden", "", None, "", launcher, legal, body);
    *res.status_mut() = StatusCode::NOT_FOUND;
    res
}

fn failure(e: sqlx::Error) -> Response {
    tracing::error!("Pack-Seite: {e}");
    (StatusCode::INTERNAL_SERVER_ERROR, "Interner Fehler").into_response()
}

fn html_page(
    title: &str,
    description: &str,
    image: Option<&str>,
    url: &str,
    launcher: &str,
    legal: &Settings,
    body: &str,
) -> Response {
    let mut meta = format!(
        r#"<meta property="og:title" content="{t}"><meta property="og:description" content="{d}"><meta property="og:site_name" content="TwoGG Pinecone">"#,
        t = esc(title),
        d = esc(description),
    );
    if !url.is_empty() {
        let _ = write!(meta, r#"<meta property="og:url" content="{}">"#, esc(url));
    }
    if let Some(img) = image {
        let _ = write!(
            meta,
            r#"<meta property="og:image" content="{}"><meta name="twitter:card" content="summary">"#,
            esc(img)
        );
    }
    let html = format!(
        r##"<!doctype html>
<html lang="de">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} · TwoGG Pinecone</title>
<meta name="description" content="{description}">
{meta}
<meta name="theme-color" content="#12100D">
<link rel="icon" href="/assets/favicon.svg" type="image/svg+xml">
<style>{CSS}</style>
</head>
<body>
<header class="top"><div class="wrap">
  <a class="brand" href="/packs">{LOGO}Pinecone</a>
  <nav><a href="/packs">TwoGG-Modpacks</a></nav>
  <span class="spacer"></span>
  <a class="btn small" href="{launcher}">{ICON_DOWNLOAD}Pinecone herunterladen</a>
</div></header>
{body}
<footer class="foot"><div class="wrap">
  <p>TwoGG Pinecone · <a href="{launcher}">Launcher</a> · <a href="https://twogg.app">twogg.app</a>{legal_links}</p>
  <p class="disclaimer">{DISCLAIMER}</p>
</div></footer>
</body>
</html>"##,
        title = esc(title),
        description = esc(description),
        launcher = esc(launcher),
        legal_links = legal_links(legal),
    );
    let mut res = Html(html).into_response();
    let headers = res.headers_mut();
    // Keine Skripte; Bilder auch von außen (Beschreibungen dürfen Bilder verlinken).
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; img-src https: data: 'self'; style-src 'unsafe-inline'; \
             font-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
        ),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=60"),
    );
    res
}

// ---------------------------------------------------------------- Dateien

const FONTS: [(&str, &[u8]); 5] = [
    (
        "pixelify-sans-600.woff2",
        include_bytes!("../assets/fonts/pixelify-sans-latin-600-normal.woff2"),
    ),
    (
        "figtree-400.woff2",
        include_bytes!("../assets/fonts/figtree-latin-400-normal.woff2"),
    ),
    (
        "figtree-600.woff2",
        include_bytes!("../assets/fonts/figtree-latin-600-normal.woff2"),
    ),
    (
        "figtree-700.woff2",
        include_bytes!("../assets/fonts/figtree-latin-700-normal.woff2"),
    ),
    (
        "jetbrains-mono-400.woff2",
        include_bytes!("../assets/fonts/jetbrains-mono-latin-400-normal.woff2"),
    ),
];

const FAVICON: &[u8] = include_bytes!("../assets/favicon.svg");

async fn asset(Path(file): Path<String>) -> Response {
    let (body, mime): (&'static [u8], &str) = if file == "favicon.svg" {
        (FAVICON, "image/svg+xml")
    } else if let Some((_, bytes)) = FONTS.iter().find(|(name, _)| *name == file) {
        (*bytes, "font/woff2")
    } else {
        return StatusCode::NOT_FOUND.into_response();
    };
    (
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, "public, max-age=604800, immutable"),
        ],
        body,
    )
        .into_response()
}

// ---------------------------------------------------------------- Hilfen

/// Neueste Release-Version, sonst die neueste überhaupt (wie die API).
fn latest(versions: &[Version]) -> Option<&Version> {
    versions
        .iter()
        .find(|v| v.channel == "release")
        .or(versions.first())
}

fn loader_name(v: &Version) -> String {
    match v.loader.as_deref() {
        Some("fabric") => "Fabric".into(),
        Some("quilt") => "Quilt".into(),
        Some("forge") => "Forge".into(),
        Some("neoforge") => "NeoForge".into(),
        Some(other) => other.into(),
        None => "Vanilla".into(),
    }
}

fn loader_label(v: &Version) -> String {
    format!("{} {}", loader_name(v), v.minecraft)
}

fn icon_html(base: &str, p: &Pack, size: u32) -> String {
    match p.icon.as_deref() {
        Some(i) => format!(
            r#"<img class="logo" src="{}" alt="" width="{size}" height="{size}">"#,
            esc(&media_url(base, i))
        ),
        None => format!(
            r#"<span class="logo letters" style="width:{size}px;height:{size}px;font-size:{}px">{}</span>"#,
            size * 2 / 5,
            esc(&initials(&p.title))
        ),
    }
}

fn initials(title: &str) -> String {
    let s: String = title
        .split_whitespace()
        .take(2)
        .filter_map(|w| w.chars().next())
        .flat_map(char::to_uppercase)
        .collect();
    if s.is_empty() { "?".into() } else { s }
}

/// HTML-Sonderzeichen maskieren (Text und Attributwerte).
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Tausenderpunkte: 12345 → „12.345“.
fn group(n: i64) -> String {
    let digits = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('.');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

fn downloads_label(n: i64) -> String {
    format!(
        "{} {}",
        group(n),
        if n == 1 { "Download" } else { "Downloads" }
    )
}

fn size_label(bytes: i64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb >= 1.0 {
        format!("{mb:.1} MB").replace('.', ",")
    } else {
        format!("{} KB", (bytes as f64 / 1024.0).ceil() as i64)
    }
}

/// Unix-Zeit → „11.10.2026“ (UTC).
fn date(ts: i64) -> String {
    let days = ts.div_euclid(86_400);
    // Tage seit 1970-01-01 → Kalenderdatum (H. Hinnant, „civil_from_days“).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{d:02}.{m:02}.{y}")
}

const LOGO: &str = r##"<svg viewBox="-2 0 13 13" width="18" height="26" shape-rendering="crispEdges" aria-hidden="true"><path fill="#8CCB7E" d="M3 0h1v1H3z M5 0h1v1H5z M4 1h1v1H4z"/><path fill="#5A3416" d="M3 2h3v1H3z M2 3h1v1H2z M6 3h1v1H6z M1 4h1v1H1z M7 4h1v1H7z M0 6h1v2H0z M8 6h1v2H8z M1 9h1v1H1z M7 9h1v1H7z M2 10h1v1H2z M6 10h1v1H6z M3 11h1v1H3z M5 11h1v1H5z M4 12h1v1H4z"/><path fill="#E9A23B" d="M3 3h3v1H3z M2 4h5v1H2z M1 5h7v4H1z M2 9h5v1H2z M3 10h3v1H3z M4 11h1v1H4z"/><path fill="#A8641F" d="M4 3h1v1H4z M3 4h1v1H3z M5 4h1v1H5z M2 5h1v1H2z M6 5h1v1H6z M3 6h1v1H3z M5 6h1v1H5z M2 7h1v1H2z M6 7h1v1H6z M3 8h1v1H3z M5 8h1v1H5z M4 9h1v1H4z M4 10h1v1H4z"/><path fill="#F6C77A" d="M4 5h1v1H4z M4 7h1v1H4z"/></svg>"##;

const ICON_PLAY: &str = r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7 4l13 8-13 8z" fill="currentColor"/></svg>"#;

const ICON_DOWNLOAD: &str = r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 4v11M7 10l5 5 5-5M5 20h14" fill="none" stroke="currentColor" stroke-width="2.2"/></svg>"#;

/// Farben, Schriften und Schnitte aus dem Pinecone-Design-System (wie die Landing Page).
const CSS: &str = r#"
@font-face { font-family: "Pixelify Sans"; font-weight: 600; font-display: swap; src: url(/assets/pixelify-sans-600.woff2) format("woff2"); }
@font-face { font-family: "Figtree"; font-weight: 400; font-display: swap; src: url(/assets/figtree-400.woff2) format("woff2"); }
@font-face { font-family: "Figtree"; font-weight: 600; font-display: swap; src: url(/assets/figtree-600.woff2) format("woff2"); }
@font-face { font-family: "Figtree"; font-weight: 700; font-display: swap; src: url(/assets/figtree-700.woff2) format("woff2"); }
@font-face { font-family: "JetBrains Mono"; font-weight: 400; font-display: swap; src: url(/assets/jetbrains-mono-400.woff2) format("woff2"); }
:root {
  --bg: #12100D; --bar: #0D0B09; --side: #0F0D0B; --stage: #17120D;
  --surface: #1C1814; --surface-2: #241F19; --surface-active: #2A2219;
  --line: #2A231B; --line-strong: #33281F;
  --text: #F1E8DA; --text-soft: #D2C4AC; --text-muted: #B5A790; --text-dim: #8F8270;
  --harz: #E9A23B; --harz-hover: #F3BC64; --harz-edge: #B8742A;
  --ink: #1B1206; --nadel: #8CCB7E; --frost: #9FC3E8;
  --pixel: "Pixelify Sans", "Figtree", sans-serif;
  --ui: "Figtree", system-ui, sans-serif;
  --mono: "JetBrains Mono", ui-monospace, monospace;
  --cut-1: polygon(10px 0, 100% 0, 100% 100%, 0 100%, 0 10px);
  --cut-2: polygon(8px 0, 100% 0, 100% calc(100% - 8px), calc(100% - 8px) 100%, 0 100%, 0 8px);
}
* { box-sizing: border-box; }
body { margin: 0; background: var(--bg); color: var(--text); font: 400 16px/1.55 var(--ui); -webkit-font-smoothing: antialiased; }
h1, h2, h3 { font-family: var(--pixel); font-weight: 600; line-height: 1.1; margin: 0; font-variant-ligatures: none; letter-spacing: 0.5px; }
p { margin: 0; }
a { color: var(--harz); text-decoration: none; }
a:hover { color: var(--harz-hover); }
:focus-visible { outline: 2px solid var(--harz); outline-offset: 3px; }
.wrap { width: min(1180px, 100% - 32px); margin-inline: auto; }
.label { font: 500 11px/1 var(--mono); letter-spacing: 1.5px; text-transform: uppercase; color: var(--text-dim); }
.mono { font-family: var(--mono); font-size: 13px; }
.muted { color: var(--text-muted); }

.top { position: sticky; top: 0; z-index: 10; background: rgba(13, 11, 9, 0.9); backdrop-filter: blur(6px); border-bottom: 1px solid var(--line); }
.top .wrap { display: flex; align-items: center; gap: 24px; height: 60px; }
.brand { display: flex; align-items: center; gap: 10px; color: var(--text); font: 600 22px/1 var(--pixel); font-variant-ligatures: none; }
.brand:hover { color: var(--text); }
.top nav a { color: var(--text-muted); font: 600 16px/1 var(--pixel); font-variant-ligatures: none; padding: 21px 0 19px; border-bottom: 3px solid var(--harz); color: var(--text); }
.spacer { flex: 1; }

.btn { display: inline-flex; align-items: center; justify-content: center; gap: 9px; border: 0; cursor: pointer; font: 700 14px/1 var(--ui); padding: 11px 16px; clip-path: var(--cut-2); background: var(--surface-2); color: var(--text); white-space: nowrap; }
.btn:hover { background: var(--surface-active); color: var(--text); }
.btn svg { width: 16px; height: 16px; flex: none; }
.btn.small { padding: 9px 13px; font-size: 13px; }
.btn.harz { background: var(--harz); color: var(--ink); box-shadow: inset 0 -3px 0 var(--harz-edge); }
.btn.harz:hover { background: var(--harz-hover); color: var(--ink); }
.btn.big { padding: 16px 24px; font-size: 16px; }
.btn.big svg { width: 18px; height: 18px; }
.btn.ghost { background: var(--surface); color: var(--text-dim); cursor: default; }

.hero { background: var(--stage); border-bottom: 1px solid var(--line); position: relative; overflow: hidden; }
.hero::before { content: ""; position: absolute; inset: 0; opacity: 0.07; pointer-events: none;
  background-image: radial-gradient(circle at 50% 100%, transparent 9px, var(--harz) 10px, transparent 11px);
  background-size: 22px 14px; }
.hero-inner { position: relative; display: flex; align-items: center; gap: 26px; padding: 40px 0 36px; }
.hero-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 10px; }
.hero h1 { font-size: 40px; }
.summary { color: var(--text-soft); font-size: 17px; max-width: 70ch; }
.facts { display: flex; flex-wrap: wrap; align-items: center; gap: 8px 18px; margin-top: 4px; font-size: 14px; color: var(--text-muted); }
.facts b { color: var(--harz); font-weight: 700; }
.tag { display: inline-block; padding: 4px 9px; background: var(--surface-2); color: var(--text-soft); font: 400 12px/1.2 var(--mono); }
.cta { display: flex; flex-direction: column; align-items: flex-end; gap: 10px; }
.hint { font-size: 13px; color: var(--text-dim); }
.logo { flex: none; object-fit: cover; clip-path: var(--cut-1); background: var(--surface-2); image-rendering: auto; }
.letters { display: grid; place-items: center; font-family: var(--pixel); font-weight: 600; color: var(--text-soft); }

.layout { display: grid; grid-template-columns: minmax(0, 1fr) 300px; gap: 32px; padding: 28px 0 56px; }
.main > input { position: absolute; opacity: 0; pointer-events: none; }
.tabs { display: flex; gap: 26px; border-bottom: 1px solid var(--line-strong); margin-bottom: 22px; }
.tabs label { cursor: pointer; padding: 12px 0 11px; font: 600 17px/1 var(--pixel); font-variant-ligatures: none; color: var(--text-muted); border-bottom: 3px solid transparent; margin-bottom: -1px; }
.tabs label:hover { color: var(--text); }
.count { font: 400 12px/1 var(--mono); color: var(--text-dim); margin-left: 4px; }
#tab-desc:checked ~ .tabs label[for="tab-desc"], #tab-files:checked ~ .tabs label[for="tab-files"] { color: var(--text); border-bottom-color: var(--harz); }
#tab-desc:focus-visible ~ .tabs label[for="tab-desc"], #tab-files:focus-visible ~ .tabs label[for="tab-files"] { outline: 2px solid var(--harz); outline-offset: 3px; }
.pane { display: none; }
#tab-desc:checked ~ .pane-desc, #tab-files:checked ~ .pane-files { display: block; }

.prose { color: var(--text-soft); line-height: 1.7; }
.prose > * + * { margin-top: 14px; }
.prose h1, .prose h2, .prose h3 { color: var(--text); margin-top: 26px; }
.prose h1 { font-size: 28px; } .prose h2 { font-size: 23px; } .prose h3 { font-size: 19px; }
.prose img { max-width: 100%; height: auto; }
.prose code { font-family: var(--mono); font-size: 13px; background: var(--surface-2); padding: 1px 5px; }
.prose pre { background: var(--surface); padding: 14px; overflow-x: auto; }
.prose blockquote { margin: 0; padding-left: 14px; border-left: 3px solid var(--harz-edge); color: var(--text-muted); }
.prose table { border-collapse: collapse; }
.prose th, .prose td { border: 1px solid var(--line-strong); padding: 6px 10px; }
.prose ul, .prose ol { padding-left: 22px; }

.file { display: grid; grid-template-columns: 34px minmax(0, 1fr) repeat(4, auto) auto; align-items: center; gap: 18px; padding: 14px 16px; background: var(--surface); clip-path: var(--cut-1); }
.file + .file { margin-top: 8px; }
.channel { display: grid; place-items: center; width: 30px; height: 30px; font: 600 15px/1 var(--pixel); }
.channel.release { background: rgba(140, 203, 126, 0.16); color: var(--nadel); }
.channel.beta { background: rgba(159, 195, 232, 0.16); color: var(--frost); }
.file-main { min-width: 0; display: flex; flex-direction: column; gap: 4px; }
.file-name { font-weight: 700; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.file-cell { display: flex; flex-direction: column; gap: 3px; font-size: 14px; color: var(--text-soft); min-width: 74px; }
.k { font: 500 10px/1 var(--mono); letter-spacing: 1.2px; text-transform: uppercase; color: var(--text-dim); }
details summary { cursor: pointer; font-size: 13px; color: var(--text-muted); }
details[open] summary { margin-bottom: 8px; }
details .prose { font-size: 14px; }

.side { display: flex; flex-direction: column; gap: 14px; }
.box { background: var(--surface); clip-path: var(--cut-1); padding: 18px; display: flex; flex-direction: column; gap: 12px; font-size: 14px; color: var(--text-soft); }
.box .btn { align-self: flex-start; }
.box.event { background: color-mix(in srgb, var(--harz) 9%, var(--surface)); }
dl { margin: 0; display: grid; grid-template-columns: auto 1fr; gap: 8px 14px; }
dt { color: var(--text-dim); }
dd { margin: 0; color: var(--text); text-align: right; overflow-wrap: anywhere; }

.list { display: grid; grid-template-columns: repeat(auto-fill, minmax(340px, 1fr)); gap: 14px; padding: 28px 0 56px; }
.card { display: flex; gap: 16px; padding: 16px; background: var(--surface); clip-path: var(--cut-1); color: var(--text); }
.card:hover { background: var(--surface-active); color: var(--text); }
.card-body { min-width: 0; display: flex; flex-direction: column; gap: 6px; }
.card-title { font: 600 19px/1.1 var(--pixel); font-variant-ligatures: none; }
.card-summary { font-size: 14px; color: var(--text-soft); display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
.card-meta { display: flex; flex-wrap: wrap; align-items: center; gap: 6px 12px; font-size: 12px; color: var(--text-dim); }
.empty { color: var(--text-muted); }

.foot { border-top: 1px solid var(--line); padding: 22px 0; font-size: 13px; color: var(--text-dim); }
.foot p { margin: 0 0 6px; }
.foot .disclaimer { font-size: 12px; }

@media (max-width: 900px) {
  .layout { grid-template-columns: 1fr; }
  .hero-inner { flex-wrap: wrap; }
  .cta { align-items: flex-start; width: 100%; }
  .file { grid-template-columns: 34px minmax(0, 1fr) auto; }
  .file-cell { display: none; }
  .top nav { display: none; }
}
@media (max-width: 560px) {
  .hero h1 { font-size: 30px; }
  .hero-inner { align-items: flex-start; }
  .hero-inner > .logo { width: 72px; height: 72px; }
  .top .btn { display: none; }
  .list { grid-template-columns: 1fr; }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_for_german_readers() {
        assert_eq!(date(0), "01.01.1970");
        assert_eq!(date(1_791_741_552), "11.10.2026");
        assert_eq!(group(1_234_567), "1.234.567");
        assert_eq!(group(999), "999");
        assert_eq!(size_label(5 * 1024 * 1024 + 300_000), "5,3 MB");
        assert_eq!(size_label(2048), "2 KB");
        assert_eq!(
            esc(r#"<a href="x">&'"#),
            "&lt;a href=&quot;x&quot;&gt;&amp;&#39;"
        );
        assert_eq!(initials("Rise of Atlantis"), "RO");
    }
}
