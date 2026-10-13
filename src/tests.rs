//! Ende-zu-Ende über den Router: Anmeldung, Pack anlegen, Version hochladen, öffentliche Liste,
//! signierter Download, Neuigkeiten.

use std::io::Write;
use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::{Config, build_state, router};

struct TestApp {
    app: axum::Router,
    _dir: tempfile::TempDir,
}

async fn setup() -> TestApp {
    setup_with_session("http://127.0.0.1:9".into()).await
}

async fn setup_with_session(session_server: String) -> TestApp {
    setup_with(session_server, None, "http://127.0.0.1:9".into()).await
}

async fn setup_with(
    session_server: String,
    curseforge_key: Option<String>,
    curseforge_api: String,
) -> TestApp {
    setup_full(
        session_server,
        curseforge_key,
        curseforge_api,
        "http://127.0.0.1:9".into(),
    )
    .await
}

async fn setup_full(
    session_server: String,
    curseforge_key: Option<String>,
    curseforge_api: String,
    fabric_meta: String,
) -> TestApp {
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        addr: "127.0.0.1:0".parse().unwrap(),
        data_dir: dir.path().join("data"),
        public_url: "https://pinecone.test".into(),
        admin_user: "admin".into(),
        admin_password_hash: crate::auth::hash_password("geheimes-passwort").unwrap(),
        admin_dir: dir.path().join("admin"),
        download_ttl: 600,
        session_server,
        launcher_url: "https://launcher.test/".into(),
        curseforge_key,
        curseforge_api,
        player_retention_days: 365,
        fabric_meta,
        extra_download_hosts: vec!["127.0.0.1".into()],
    };
    std::fs::create_dir_all(&config.admin_dir).unwrap();
    std::fs::write(config.admin_dir.join("index.html"), "<html>admin</html>").unwrap();
    let state = build_state(config).await.unwrap();
    TestApp {
        app: router(state),
        _dir: dir,
    }
}

impl TestApp {
    async fn send(&self, mut req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
        req.extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([10, 0, 0, 1], 5000))));
        let resp = self.app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let headers = resp.headers().clone();
        let body = resp
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec();
        (status, headers, body)
    }

    async fn json(
        &self,
        method: &str,
        uri: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut b = Request::builder().method(method).uri(uri);
        if let Some(t) = token {
            b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        let req = match body {
            Some(v) => b
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(v.to_string()))
                .unwrap(),
            None => b.body(Body::empty()).unwrap(),
        };
        let (status, _, bytes) = self.send(req).await;
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
}

fn mrpack() -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default();
        zip.start_file("modrinth.index.json", opts).unwrap();
        zip.write_all(
            json!({ "formatVersion": 1, "game": "minecraft", "versionId": "1.2.0", "name": "Horizons",
                    "files": [], "dependencies": { "minecraft": "1.21.1", "fabric-loader": "0.19.5" } })
                .to_string()
                .as_bytes(),
        )
        .unwrap();
        zip.start_file("overrides/config/twogg.json", opts).unwrap();
        zip.write_all(b"{}").unwrap();
        zip.finish().unwrap();
    }
    buf.into_inner()
}

fn multipart(fields: &[(&str, &str)], file: Option<(&str, &[u8])>) -> (String, Vec<u8>) {
    let boundary = "XpineconeX";
    let mut body = Vec::new();
    for (k, v) in fields {
        write!(
            body,
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n"
        )
        .unwrap();
    }
    if let Some((name, data)) = file {
        write!(body, "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: application/octet-stream\r\n\r\n").unwrap();
        body.extend_from_slice(data);
        body.extend_from_slice(b"\r\n");
    }
    write!(body, "--{boundary}--\r\n").unwrap();
    (format!("multipart/form-data; boundary={boundary}"), body)
}

async fn login(app: &TestApp) -> String {
    let (status, body) = app
        .json(
            "POST",
            "/api/admin/login",
            None,
            Some(json!({ "username": "admin", "password": "geheimes-passwort" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["token"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn admin_needs_login() {
    let app = setup().await;
    let (status, _) = app.json("GET", "/api/admin/packs", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = app
        .json("GET", "/api/admin/packs", Some("falsch"), None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = app
        .json(
            "POST",
            "/api/admin/login",
            None,
            Some(json!({ "username": "admin", "password": "falsch-falsch" })),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // Admin-SPA wird ausgeliefert (auch für Unterseiten).
    let (status, _, body) = app
        .send(Request::get("/admin/packs/x").body(Body::empty()).unwrap())
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"<html>admin</html>");
}

#[tokio::test]
async fn pack_lifecycle_and_signed_download() {
    let app = setup().await;
    let token = login(&app).await;
    let t = Some(token.as_str());

    let (status, body) = app
        .json(
            "POST",
            "/api/admin/packs",
            t,
            Some(json!({ "id": "Ungültig!", "title": "x" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = app
        .json(
            "POST",
            "/api/admin/packs",
            t,
            Some(json!({ "id": "x1", "title": 5 })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .starts_with("Ungültige Eingabe"),
        "{body}"
    );
    let (status, body) = app
        .json(
            "POST",
            "/api/admin/packs",
            t,
            Some(json!({ "id": "horizons", "title": "Horizons Unbound", "summary": "Unser Server-Pack",
                         "description": "# Willkommen\n<script>x</script>", "visibility": "public", "featured": true,
                         "sortOrder": null, "icon": null })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["versionCount"], 0);

    // Ohne Version taucht das Pack öffentlich noch nicht auf.
    let (_, list) = app.json("GET", "/api/v1/packs", None, None).await;
    assert_eq!(list.as_array().unwrap().len(), 0);

    // Kaputte Datei wird abgelehnt und nicht gespeichert.
    let (ct, body) = multipart(&[("packId", "horizons")], Some(("x.mrpack", b"kein zip")));
    let (status, _, _) = app
        .send(
            Request::post("/api/admin/versions")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let pack = mrpack();
    let (ct, body) = multipart(
        &[("packId", "horizons"), ("changelog", "- **Neu**")],
        Some(("h.mrpack", &pack)),
    );
    let (status, _, bytes) = app
        .send(
            Request::post("/api/admin/versions")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    let version: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(status, StatusCode::OK, "{version}");
    assert_eq!(version["version"], "1.2.0");
    assert_eq!(version["minecraft"], "1.21.1");
    assert_eq!(version["loader"], "fabric");
    assert_eq!(version["loaderVersion"], "0.19.5");
    let vid = version["id"].as_str().unwrap().to_string();

    // Admin-Liste im React-Admin-Format.
    let (status, headers, bytes) = app
        .send(Request::get("/api/admin/versions?filter=%7B%22packId%22%3A%22horizons%22%7D&range=%5B0%2C9%5D&sort=%5B%22publishedAt%22%2C%22DESC%22%5D")
            .header(header::AUTHORIZATION, format!("Bearer {token}")).body(Body::empty()).unwrap())
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-range"], "versions 0-0/1");
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let (status, _) = app
        .json(
            "GET",
            "/api/admin/packs?sort=%5B%22drop%20table%22%2C%22ASC%22%5D",
            t,
            None,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "Sortierfeld nur aus der Liste"
    );

    // Öffentlich: Liste, Details mit gerendertem Markdown.
    let (_, list) = app.json("GET", "/api/v1/packs", None, None).await;
    assert_eq!(list[0]["id"], "horizons");
    assert_eq!(list[0]["latest"]["version"], "1.2.0");
    let (_, detail) = app.json("GET", "/api/v1/packs/horizons", None, None).await;
    let html = detail["descriptionHtml"].as_str().unwrap();
    assert!(
        html.contains("<h1>Willkommen</h1>") && !html.contains("<script>"),
        "{html}"
    );
    assert!(
        detail["versions"][0]["changelogHtml"]
            .as_str()
            .unwrap()
            .contains("<strong>Neu</strong>")
    );

    // Download-Link nur mit Pinecone-Kennung.
    let uri = format!("/api/v1/packs/horizons/versions/{vid}/download");
    let (status, _) = app.json("POST", &uri, None, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, bytes) = app
        .send(
            Request::post(&uri)
                .header("x-pinecone-client", "pinecone/0.1.0")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let link: Value = serde_json::from_slice(&bytes).unwrap();
    let url = link["url"].as_str().unwrap();
    assert!(url.starts_with("https://pinecone.test/dl/"), "{url}");
    let path = url.trim_start_matches("https://pinecone.test");
    let (status, headers, file) = app
        .send(Request::get(path).body(Body::empty()).unwrap())
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(file, pack);
    assert!(
        headers[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .contains("horizons-1.2.0.mrpack")
    );
    let (status, _, _) = app
        .send(
            Request::get("/dl/gefaelscht.123.abc")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (_, detail) = app.json("GET", "/api/admin/packs/horizons", t, None).await;
    assert_eq!(detail["downloads"], 1);

    // Versteckt: weder gelistet noch herunterladbar.
    let (status, _) = app
        .json(
            "PUT",
            "/api/admin/packs/horizons",
            t,
            Some(json!({ "title": "Horizons Unbound", "visibility": "hidden" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = app.json("GET", "/api/v1/packs/horizons", None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = app
        .send(
            Request::post(&uri)
                .header("x-pinecone-client", "pinecone/0.1.0")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Löschen entfernt auch die Versionen.
    let (status, _) = app
        .json("DELETE", "/api/admin/packs/horizons", t, None)
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = app
        .json("GET", &format!("/api/admin/versions/{vid}"), t, None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn news_and_images() {
    let app = setup().await;
    let token = login(&app).await;
    let t = Some(token.as_str());

    // Nur echte Bilder.
    let (ct, body) = multipart(&[], Some(("x.png", b"<svg onload=alert(1)>")));
    let (status, _, _) = app
        .send(
            Request::post("/api/admin/media")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let png = [0x89u8, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3, 4];
    let (ct, body) = multipart(&[], Some(("x.png", &png)));
    let (status, _, bytes) = app
        .send(
            Request::post("/api/admin/media")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let media: Value = serde_json::from_slice(&bytes).unwrap();
    let name = media["name"].as_str().unwrap();
    assert!(name.ends_with(".png"));
    let (status, _, served) = app
        .send(
            Request::get(format!("/media/{name}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(
        (status, served.as_slice()),
        (StatusCode::OK, png.as_slice())
    );

    let (status, _) = app
        .json(
            "POST",
            "/api/admin/news",
            t,
            Some(json!({ "title": "x", "link": "javascript:alert(1)" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    for (title, pinned, visible, at) in [
        ("Alt", false, true, 100),
        ("Angepinnt", true, true, 50),
        ("Entwurf", false, false, 200),
    ] {
        let (status, body) = app
            .json("POST", "/api/admin/news", t, Some(json!({ "title": title, "body": "**Hallo**", "pinned": pinned,
                                                            "visible": visible, "publishedAt": at, "image": name })))
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    // Geplant (Zukunft) erscheint noch nicht.
    app.json(
        "POST",
        "/api/admin/news",
        t,
        Some(json!({ "title": "Später", "publishedAt": 4_000_000_000i64 })),
    )
    .await;
    let (_, news) = app.json("GET", "/api/v1/news", None, None).await;
    let titles: Vec<&str> = news
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["Angepinnt", "Alt"]);
    assert_eq!(news[0]["bodyHtml"], "<p><strong>Hallo</strong></p>\n");
    assert_eq!(
        news[0]["imageUrl"],
        format!("https://pinecone.test/media/{name}")
    );
    let (_, stats) = app.json("GET", "/api/admin/stats", t, None).await;
    assert_eq!(stats["news"], 4);
}

/// Nachgebauter Mojang-Sitzungsserver: kennt Alice und Bob, sobald sie sich mit einer Server-ID
/// „angemeldet“ haben (hier: jede Server-ID mit 32 Zeichen).
async fn mock_mojang() -> String {
    use axum::extract::Query;
    use axum::routing::get;
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Q {
        username: String,
        server_id: String,
    }
    let app = axum::Router::new().route(
        "/session/minecraft/hasJoined",
        get(|Query(q): Query<Q>| async move {
            let id = match q.username.as_str() {
                "Alice" => "11111111111111111111111111111111",
                "Bob" => "22222222222222222222222222222222",
                _ => return (StatusCode::NO_CONTENT, String::new()),
            };
            if q.server_id.len() != 32 {
                return (StatusCode::NO_CONTENT, String::new());
            }
            (
                StatusCode::OK,
                json!({ "id": id, "name": q.username }).to_string(),
            )
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

async fn player_login(t: &TestApp, name: &str) -> String {
    let (status, c) = t.json("POST", "/api/v1/auth/challenge", None, None).await;
    assert_eq!(status, StatusCode::OK);
    let server_id = c["serverId"].as_str().unwrap().to_string();
    let (status, v) = t
        .json(
            "POST",
            "/api/v1/auth/verify",
            None,
            Some(json!({ "username": name, "serverId": server_id })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    // Dieselbe Server-ID gilt nur einmal.
    let (again, _) = t
        .json(
            "POST",
            "/api/v1/auth/verify",
            None,
            Some(json!({ "username": name, "serverId": server_id })),
        )
        .await;
    assert_eq!(again, StatusCode::BAD_REQUEST);
    v["token"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn friends_and_presence() {
    let t = setup_with_session(mock_mojang().await).await;
    let (status, _) = t.json("GET", "/api/v1/friends", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Unbekannter Account wird von „Mojang“ abgelehnt.
    let (_, c) = t.json("POST", "/api/v1/auth/challenge", None, None).await;
    let (status, _) = t
        .json(
            "POST",
            "/api/v1/auth/verify",
            None,
            Some(json!({ "username": "Mallory", "serverId": c["serverId"] })),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let alice = player_login(&t, "Alice").await;
    let (status, v) = t
        .json(
            "POST",
            "/api/v1/friends/requests",
            Some(&alice),
            Some(json!({ "name": "bob" })),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "Bob nutzt Pinecone noch nicht: {v}"
    );

    let bob = player_login(&t, "Bob").await;
    let (_, v) = t
        .json(
            "POST",
            "/api/v1/friends/requests",
            Some(&alice),
            Some(json!({ "name": "bob" })),
        )
        .await;
    assert_eq!(v["status"], "requested");
    let (_, v) = t.json("GET", "/api/v1/friends", Some(&bob), None).await;
    assert_eq!(v["incoming"][0]["name"], "Alice");

    // Status ist vor der Freundschaft nicht sichtbar; nach dem Annehmen schon.
    let (status, _) = t
        .json("POST", "/api/v1/presence", Some(&alice), Some(json!({ "activity": {
            "instance": "REVIVAL: Avalon", "version": "Fabric 1.21.11", "server": "mc.twogg.de", "playing": true } })))
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = t
        .json(
            "POST",
            "/api/v1/friends/requests/11111111111111111111111111111111/accept",
            Some(&bob),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, v) = t.json("GET", "/api/v1/friends", Some(&bob), None).await;
    assert_eq!(v["friends"][0]["name"], "Alice");
    assert_eq!(v["friends"][0]["online"], true);
    assert_eq!(v["friends"][0]["activity"]["instance"], "REVIVAL: Avalon");
    assert!(v["incoming"].as_array().unwrap().is_empty());

    // Entfernen gilt für beide Seiten.
    let (status, _) = t
        .json(
            "DELETE",
            "/api/v1/friends/22222222222222222222222222222222",
            Some(&alice),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, v) = t.json("GET", "/api/v1/friends", Some(&bob), None).await;
    assert!(v["friends"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn game_server_setting() {
    let t = setup().await;
    let (_, v) = t.json("GET", "/api/v1/info", None, None).await;
    assert!(v["gameServer"].is_null());
    let token = login(&t).await;
    let (status, v) = t
        .json(
            "PUT",
            "/api/admin/settings/main",
            Some(&token),
            Some(json!({ "id": "main", "gameServerName": null, "gameServerAddress": "https://x" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    let (status, _) = t
        .json(
            "PUT",
            "/api/admin/settings/main",
            Some(&token),
            Some(json!({ "id": "main", "gameServerName": "", "gameServerAddress": "mc.twogg.de" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, v) = t.json("GET", "/api/v1/info", None, None).await;
    assert_eq!(
        v["gameServer"],
        json!({ "name": "TwoGG Server", "address": "mc.twogg.de" })
    );
}

#[tokio::test]
async fn recommendations_and_event_link() {
    let t = setup().await;
    let token = login(&t).await;
    let (status, v) = t
        .json(
            "POST",
            "/api/admin/recommendations",
            Some(&token),
            Some(json!({ "projectType": "shader", "source": "twogg", "projectId": "x", "title": "Falsch" })),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "TwoGG nur als Modpack: {v}"
    );
    let (status, v) = t
        .json(
            "POST",
            "/api/admin/recommendations",
            Some(&token),
            Some(json!({ "projectType": "shader", "source": "modrinth", "projectId": "complementary-reimagined",
                "title": "WINTERLAND-Shader", "subtitle": "Schneelicht für das Winter-Event", "motif": "snowflake", "subtitle2": null })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let (_, list) = t
        .json("GET", "/api/v1/recommendations?type=shader", None, None)
        .await;
    assert_eq!(list[0]["title"], "WINTERLAND-Shader");
    assert_eq!(list[0]["motif"], "snowflake");
    let (_, empty) = t
        .json("GET", "/api/v1/recommendations?type=mod", None, None)
        .await;
    assert!(empty.as_array().unwrap().is_empty());

    let (status, v) = t
        .json(
            "POST",
            "/api/admin/packs",
            Some(&token),
            Some(
                json!({ "id": "avalon", "title": "Avalon Event-Pack", "visibility": "public",
                "eventId": "ca7acfe0-fc10-44e4-a477-96b64c670631" }),
            ),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["eventId"], "ca7acfe0-fc10-44e4-a477-96b64c670631");
    let (status, _) = t
        .json(
            "PUT",
            "/api/admin/packs/avalon",
            Some(&token),
            Some(json!({ "title": "Avalon", "eventId": "../x" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn public_pack_pages() {
    let app = setup().await;
    let token = login(&app).await;
    let t = Some(token.as_str());
    let page = |uri: &str| {
        let req = Request::get(uri).body(Body::empty()).unwrap();
        let app = &app;
        async move {
            let (status, headers, body) = app.send(req).await;
            (status, headers, String::from_utf8_lossy(&body).into_owned())
        }
    };

    let (status, body) = app
        .json(
            "POST",
            "/api/admin/packs",
            t,
            Some(json!({ "id": "horizons", "title": "Horizons <Unbound>", "summary": "Unser Server-Pack",
                         "description": "# Willkommen\n<script>alert(1)</script>", "visibility": "unlisted",
                         "eventId": "3f2b6c1e-1a2b-4c3d-8e9f-0a1b2c3d4e5f" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let pack = mrpack();
    let (ct, body) = multipart(&[("packId", "horizons")], Some(("h.mrpack", &pack)));
    let (status, _, _) = app
        .send(
            Request::post("/api/admin/versions")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // Ungelistet: Seite per Link erreichbar, aber nicht in der Übersicht.
    let (status, headers, html) = page("/packs/horizons").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .starts_with("default-src 'none'")
    );
    assert!(html.contains("Horizons &lt;Unbound&gt;"), "Titel maskiert");
    assert!(
        !html.contains("<script>"),
        "kein Skript aus der Beschreibung"
    );
    assert!(html.contains(r#"href="pinecone://twogg/horizons""#));
    assert!(
        html.contains("pinecone://twogg/horizons/"),
        "Link je Version"
    );
    assert!(html.contains("Fabric 1.21.1"));
    assert!(html.contains("https://twogg.app/events/3f2b6c1e-1a2b-4c3d-8e9f-0a1b2c3d4e5f"));
    assert!(html.contains("https://launcher.test/"));
    let (_, _, index) = page("/packs").await;
    assert!(!index.contains("/packs/horizons"));

    let (status, _) = app
        .json(
            "PUT",
            "/api/admin/packs/horizons",
            t,
            Some(
                json!({ "id": "horizons", "title": "Horizons", "summary": "", "description": "",
                         "visibility": "public" }),
            ),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, _, index) = page("/packs").await;
    assert!(index.contains(r#"href="/packs/horizons""#));
    let (_, pack) = app.json("GET", "/api/v1/packs/horizons", None, None).await;
    assert_eq!(pack["pageUrl"], "https://pinecone.test/packs/horizons");

    let (status, _, _) = page("/packs/gibt-es-nicht").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, headers, _) = page("/assets/figtree-400.woff2").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "font/woff2");
    let (status, _, _) = page("/assets/../secret.key").await;
    assert_ne!(status, StatusCode::OK);
    let (status, headers, _) = page("/").await;
    assert_eq!(status, StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(headers[header::LOCATION], "/packs");
}

/// Nachgebaute CurseForge-API: prüft Schlüssel und Spiel-ID und gibt die Anfrage zurück.
async fn mock_curseforge() -> String {
    use axum::extract::RawQuery;
    use axum::http::HeaderMap;
    use axum::routing::{get, post};
    let app = axum::Router::new()
        .route(
            "/v1/mods/search",
            get(|headers: HeaderMap, RawQuery(q): RawQuery| async move {
                if headers.get("x-api-key").and_then(|v| v.to_str().ok()) != Some("cf-key") {
                    return (StatusCode::FORBIDDEN, String::new());
                }
                (
                    StatusCode::OK,
                    json!({ "data": [], "query": q.unwrap_or_default() }).to_string(),
                )
            }),
        )
        .route(
            "/v1/mods/{id}/files",
            get(
                |axum::extract::Path(id): axum::extract::Path<u64>| async move {
                    json!({ "data": [{ "id": id * 10 }] }).to_string()
                },
            ),
        )
        .route(
            "/v1/mods",
            post(|axum::Json(v): axum::Json<Value>| async move {
                json!({ "data": v["modIds"] }).to_string()
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

async fn launcher_get(t: &TestApp, uri: &str) -> (StatusCode, axum::http::HeaderMap, Value) {
    let req = Request::builder()
        .uri(uri)
        .header("x-pinecone-client", "pinecone/0.9.6")
        .body(Body::empty())
        .unwrap();
    let (status, headers, body) = t.send(req).await;
    (
        status,
        headers,
        serde_json::from_slice(&body).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn curseforge_proxy_keeps_key_on_server() {
    // Ohne Schlüssel: nicht verfügbar, und /info sagt das dem Launcher.
    let t = setup().await;
    let (_, info) = t.json("GET", "/api/v1/info", None, None).await;
    assert_eq!(info["curseforge"], false);
    let (status, _, _) = launcher_get(&t, "/api/v1/curseforge/v1/mods/search?searchFilter=x").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

    let t = setup_with(
        "http://127.0.0.1:9".into(),
        Some("cf-key".into()),
        mock_curseforge().await,
    )
    .await;
    let (_, info) = t.json("GET", "/api/v1/info", None, None).await;
    assert_eq!(info["curseforge"], true);

    // Nur der Launcher darf fragen.
    let (status, _) = t
        .json("GET", "/api/v1/curseforge/v1/mods/search", None, None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Spiel-ID ist fest Minecraft, fremde Parameter fallen weg, nichts wird gecacht.
    let (status, headers, v) = launcher_get(
        &t,
        "/api/v1/curseforge/v1/mods/search?gameId=1&searchFilter=sodium&apiKey=klau&pageSize=5",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["query"], "gameId=432&searchFilter=sodium&pageSize=5");
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert!(!v.to_string().contains("cf-key"));

    let (_, _, v) = launcher_get(&t, "/api/v1/curseforge/v1/mods/7/files?pageSize=50").await;
    assert_eq!(v["data"][0]["id"], 70);

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/curseforge/v1/mods")
        .header("x-pinecone-client", "pinecone/0.9.6")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "modIds": [1, 2] }).to_string()))
        .unwrap();
    let (status, _, body) = t.send(req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["data"],
        json!([1, 2])
    );

    // Abgelehnter Schlüssel wird zum Serverfehler, nicht zu einem 403 für den Launcher.
    let t = setup_with(
        "http://127.0.0.1:9".into(),
        Some("falsch".into()),
        mock_curseforge().await,
    )
    .await;
    let (status, _, _) = launcher_get(&t, "/api/v1/curseforge/v1/mods/search").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn players_can_delete_their_data_and_inactive_profiles_expire() {
    let t = setup_with_session(mock_mojang().await).await;
    let alice = player_login(&t, "Alice").await;
    let bob = player_login(&t, "Bob").await;
    t.json(
        "POST",
        "/api/v1/friends/requests",
        Some(&alice),
        Some(json!({ "name": "Bob" })),
    )
    .await;
    let (status, _) = t.json("DELETE", "/api/v1/me", Some(&alice), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    // Sitzung ist weg, Anfrage bei Bob auch.
    let (status, _) = t.json("GET", "/api/v1/friends", Some(&alice), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (_, v) = t.json("GET", "/api/v1/friends", Some(&bob), None).await;
    assert!(v["incoming"].as_array().unwrap().is_empty(), "{v}");
}

#[tokio::test]
async fn cleanup_removes_long_inactive_players() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        addr: "127.0.0.1:0".parse().unwrap(),
        data_dir: dir.path().join("data"),
        public_url: "https://pinecone.test".into(),
        admin_user: "admin".into(),
        admin_password_hash: crate::auth::hash_password("geheimes-passwort").unwrap(),
        admin_dir: dir.path().join("admin"),
        download_ttl: 600,
        session_server: "http://127.0.0.1:9".into(),
        launcher_url: "https://launcher.test/".into(),
        curseforge_key: None,
        curseforge_api: "http://127.0.0.1:9".into(),
        player_retention_days: 30,
        fabric_meta: "http://127.0.0.1:9".into(),
        extra_download_hosts: Vec::new(),
    };
    let state = build_state(config).await.unwrap();
    let now = crate::db::now();
    for (uuid, seen) in [("a", now), ("b", now - 31 * 86_400)] {
        sqlx::query(
            "INSERT INTO players (uuid, name, name_lower, created_at, last_seen) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(uuid)
        .bind(uuid)
        .bind(uuid)
        .bind(seen)
        .bind(seen)
        .execute(&state.db)
        .await
        .unwrap();
    }
    assert_eq!(crate::friends::cleanup(&state).await.unwrap(), 1);
    let (left,): (String,) = sqlx::query_as("SELECT uuid FROM players")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(left, "a");
}

#[tokio::test]
async fn legal_settings_and_disclaimer() {
    let app = setup().await;
    let t = login(&app).await;
    let (status, _) = app
        .json(
            "PUT",
            "/api/admin/settings/main",
            Some(&t),
            Some(json!({ "privacyUrl": "http://unsicher.de" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = app
        .json(
            "PUT",
            "/api/admin/settings/main",
            Some(&t),
            Some(json!({
                "imprintUrl": "https://twogg.app/impressum",
                "privacyUrl": "https://twogg.app/datenschutz",
                "contactEmail": "kontakt@twogg.de"
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, info) = app.json("GET", "/api/v1/info", None, None).await;
    assert_eq!(info["legal"]["privacyUrl"], "https://twogg.app/datenschutz");
    assert_eq!(info["legal"]["contactEmail"], "kontakt@twogg.de");

    let req = Request::builder()
        .uri("/packs")
        .body(Body::empty())
        .unwrap();
    let (_, _, body) = app.send(req).await;
    let html = String::from_utf8(body).unwrap();
    assert!(html.contains("NOT AN OFFICIAL MINECRAFT PRODUCT"));
    assert!(html.contains(r#"href="https://twogg.app/impressum""#));
    assert!(html.contains("mailto:kontakt@twogg.de"));
}

#[tokio::test]
async fn friend_codes_invites_and_sync_shares() {
    let t = setup_with_session(mock_mojang().await).await;
    let alice = player_login(&t, "Alice").await;
    let bob = player_login(&t, "Bob").await;

    // Freundescode: stabil, neu erzeugbar, alter sofort ungültig.
    let (status, v) = t
        .json("GET", "/api/v1/friends/code", Some(&alice), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    let code = v["code"].as_str().unwrap().to_string();
    let (_, again) = t
        .json("GET", "/api/v1/friends/code", Some(&alice), None)
        .await;
    assert_eq!(again["code"], code.as_str());
    let (_, v) = t
        .json("POST", "/api/v1/friends/code", Some(&alice), None)
        .await;
    let fresh = v["code"].as_str().unwrap().to_string();
    assert_ne!(fresh, code);
    let (status, _) = t
        .json(
            "POST",
            "/api/v1/friends/requests",
            Some(&bob),
            Some(json!({ "code": code })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "alter Code gilt nicht mehr");
    let (_, v) = t
        .json(
            "POST",
            "/api/v1/friends/requests",
            Some(&alice),
            Some(json!({ "code": fresh })),
        )
        .await;
    assert_eq!(
        v["error"].as_str().map(|e| e.contains("selbst")),
        Some(true),
        "{v}"
    );

    // Einladen geht erst nach der Freundschaft.
    let invite = json!({ "to": "22222222222222222222222222222222", "server": "mc.twogg.de",
        "pack": "pinecone://twogg/avalon", "instance": "REVIVAL: Avalon" });
    let (status, _) = t
        .json(
            "POST",
            "/api/v1/invites",
            Some(&alice),
            Some(invite.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // Bob schickt per Code (Kleinbuchstaben, Link-Form) eine Anfrage, Alice nimmt an.
    let link = format!("pinecone://friend/{}", fresh.to_lowercase());
    let (_, v) = t
        .json(
            "POST",
            "/api/v1/friends/requests",
            Some(&bob),
            Some(json!({ "code": link })),
        )
        .await;
    assert_eq!(v["status"], "requested", "{v}");
    t.json(
        "POST",
        "/api/v1/friends/requests/22222222222222222222222222222222/accept",
        Some(&alice),
        None,
    )
    .await;

    let (status, v) = t
        .json(
            "POST",
            "/api/v1/invites",
            Some(&alice),
            Some(invite.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    // Eine neue Einladung ersetzt die alte.
    let (_, v) = t
        .json(
            "POST",
            "/api/v1/invites",
            Some(&alice),
            Some(invite.clone()),
        )
        .await;
    let id = v["id"].as_str().unwrap().to_string();
    let (_, v) = t.json("GET", "/api/v1/friends", Some(&bob), None).await;
    let invites = v["invites"].as_array().unwrap();
    assert_eq!(invites.len(), 1);
    assert_eq!(invites[0]["name"], "Alice");
    assert_eq!(invites[0]["server"], "mc.twogg.de");
    assert_eq!(invites[0]["pack"], "pinecone://twogg/avalon");
    let (status, _) = t
        .json(
            "POST",
            &format!("/api/v1/invites/{id}/answer"),
            Some(&bob),
            Some(json!({ "accept": false })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, v) = t.json("GET", "/api/v1/friends", Some(&alice), None).await;
    assert_eq!(v["sentInvites"][0]["status"], "declined");
    let (_, v) = t.json("GET", "/api/v1/friends", Some(&bob), None).await;
    assert!(v["invites"].as_array().unwrap().is_empty());
    // Nochmal antworten geht nicht; fremde Packs (kein pinecone://) werden verworfen.
    let (status, _) = t
        .json(
            "POST",
            &format!("/api/v1/invites/{id}/answer"),
            Some(&bob),
            Some(json!({ "accept": true })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, v) = t
        .json("POST", "/api/v1/invites", Some(&alice),
            Some(json!({ "to": "22222222222222222222222222222222", "server": "mc.twogg.de", "pack": "https://evil.example" })))
        .await;
    assert!(v["id"].is_string());
    let (_, v) = t.json("GET", "/api/v1/friends", Some(&bob), None).await;
    assert_eq!(v["invites"][0]["pack"], Value::Null);
    // Ratenbegrenzung: höchstens 10 je Minute.
    let mut last = StatusCode::CREATED;
    for _ in 0..10 {
        (last, _) = t
            .json(
                "POST",
                "/api/v1/invites",
                Some(&alice),
                Some(invite.clone()),
            )
            .await;
    }
    assert_eq!(last, StatusCode::TOO_MANY_REQUESTS);

    // Sync-Pack teilen: nur angemeldet abrufbar.
    let pack =
        json!({ "name": "Event-Tasten", "options": { "key_key.jump": "key.keyboard.space" } });
    let (status, v) = t
        .json(
            "POST",
            "/api/v1/sync-shares",
            Some(&alice),
            Some(json!({ "data": pack })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let share = v["code"].as_str().unwrap().to_string();
    let (status, _) = t
        .json("GET", &format!("/api/v1/sync-shares/{share}"), None, None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (_, v) = t
        .json(
            "GET",
            &format!("/api/v1/sync-shares/{share}"),
            Some(&bob),
            None,
        )
        .await;
    assert_eq!(v["owner"], "Alice");
    assert_eq!(v["data"]["name"], "Event-Tasten");
    let (status, _) = t
        .json(
            "POST",
            "/api/v1/sync-shares",
            Some(&alice),
            Some(json!({ "data": "x".repeat(70_000) })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // „Meine Freunde-Daten löschen“ nimmt Einladungen und Freigaben mit.
    let (status, _) = t.json("DELETE", "/api/v1/me", Some(&alice), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = t
        .json(
            "GET",
            &format!("/api/v1/sync-shares/{share}"),
            Some(&bob),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, v) = t.json("GET", "/api/v1/friends", Some(&bob), None).await;
    assert!(v["invites"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn admin_accounts_roles_and_audit() {
    let t = setup().await;
    let admin = login(&t).await;
    let (_, me) = t.json("GET", "/api/admin/me", Some(&admin), None).await;
    assert_eq!(me["role"], "admin");

    // Konto aus der Konfiguration ist übernommen und geschützt (AK-63.1).
    let (_, list) = t
        .json("GET", "/api/admin/accounts", Some(&admin), None)
        .await;
    assert_eq!(list[0]["id"], "admin");
    assert_eq!(list[0]["protected"], true);
    let (status, _) = t
        .json("DELETE", "/api/admin/accounts/admin", Some(&admin), None)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = t
        .json(
            "PUT",
            "/api/admin/accounts/admin",
            Some(&admin),
            Some(json!({ "role": "editor" })),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "letzter Admin bleibt Admin (AK-63.3)"
    );

    // Redakteurin anlegen.
    let (status, v) = t
        .json(
            "POST",
            "/api/admin/accounts",
            Some(&admin),
            Some(json!({ "id": "lena", "role": "editor", "password": "redaktion-2026" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(v["id"], "lena");
    let (status, _) = t
        .json(
            "POST",
            "/api/admin/accounts",
            Some(&admin),
            Some(json!({ "id": "kurz", "role": "editor", "password": "kurz" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "Passwort zu kurz");
    let (_, v) = t
        .json(
            "POST",
            "/api/admin/login",
            None,
            Some(json!({ "username": "lena", "password": "redaktion-2026" })),
        )
        .await;
    assert_eq!(v["role"], "editor");
    let lena = v["token"].as_str().unwrap().to_string();

    // Redakteurin: Neuigkeiten ja, Packs/Einstellungen/Konten/Protokoll nein.
    let (status, _) = t
        .json(
            "POST",
            "/api/admin/news",
            Some(&lena),
            Some(json!({ "title": "Hallo", "body": "Text" })),
        )
        .await;
    assert!(status.is_success(), "{status}");
    let (status, _) = t.json("GET", "/api/admin/packs", Some(&lena), None).await;
    assert_eq!(status, StatusCode::OK, "lesen darf sie");
    let (status, _) = t
        .json(
            "POST",
            "/api/admin/packs",
            Some(&lena),
            Some(json!({ "id": "x", "title": "X" })),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = t
        .json(
            "PUT",
            "/api/admin/settings/main",
            Some(&lena),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = t
        .json("GET", "/api/admin/accounts", Some(&lena), None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = t.json("GET", "/api/admin/audit", Some(&lena), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Zum Admin befördern: alte Sitzung endet, neue Rolle gilt nach neuer Anmeldung.
    let (status, _) = t
        .json(
            "PUT",
            "/api/admin/accounts/lena",
            Some(&admin),
            Some(json!({ "role": "admin" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = t.json("GET", "/api/admin/packs", Some(&lena), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // Jetzt gibt es zwei Admins: lena darf herabgestuft und gelöscht werden.
    let (status, _) = t
        .json("DELETE", "/api/admin/accounts/lena", Some(&admin), None)
        .await;
    assert_eq!(status, StatusCode::OK);

    // Protokoll: Anmeldung, Neuanlage mit ID aus der Antwort, Änderungen, Löschung – keine Lesezugriffe.
    let (status, log) = t
        .json(
            "GET",
            "/api/admin/audit?sort=%5B%22at%22%2C%22ASC%22%5D",
            Some(&admin),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let entries: Vec<(String, String, String, String)> = log
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["username"].as_str().unwrap().into(),
                e["action"].as_str().unwrap().into(),
                e["objectType"].as_str().unwrap().into(),
                e["objectId"].as_str().unwrap_or("").into(),
            )
        })
        .collect();
    let has = |u: &str, a: &str, o: &str, id: &str| {
        entries
            .iter()
            .any(|e| e.0 == u && e.1 == a && e.2 == o && (id.is_empty() || e.3 == id))
    };
    assert!(has("admin", "login", "session", ""), "{entries:?}");
    assert!(has("admin", "create", "accounts", "lena"), "{entries:?}");
    assert!(has("lena", "create", "news", ""), "{entries:?}");
    assert!(has("admin", "update", "accounts", "lena"), "{entries:?}");
    assert!(has("admin", "delete", "accounts", "lena"), "{entries:?}");
    assert!(
        !entries.iter().any(|e| e.2 == "packs"),
        "abgelehnte Änderungen stehen nicht drin"
    );
    // Das Protokoll lässt sich nicht ändern (AK-63.2).
    let (status, _) = t
        .json("DELETE", "/api/admin/audit/1", Some(&admin), None)
        .await;
    assert!(
        status == StatusCode::METHOD_NOT_ALLOWED || status == StatusCode::NOT_FOUND,
        "{status}"
    );

    // Unbekannter Benutzer und falsches Passwort.
    let (status, _) = t
        .json(
            "POST",
            "/api/admin/login",
            None,
            Some(json!({ "username": "lena", "password": "redaktion-2026" })),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn withdrawn_versions_disappear_but_launchers_learn_why() {
    let app = setup().await;
    let token = login(&app).await;
    let t = Some(token.as_str());
    app.json(
        "POST",
        "/api/admin/packs",
        t,
        Some(json!({ "id": "avalon", "title": "Avalon", "visibility": "public" })),
    )
    .await;
    let upload = |version: &'static str| {
        let token = token.clone();
        let app = &app;
        async move {
            let (ct, body) = multipart(
                &[("packId", "avalon"), ("version", version)],
                Some(("a.mrpack", &mrpack())),
            );
            let (status, _, bytes) = app
                .send(
                    Request::post("/api/admin/versions")
                        .header(header::AUTHORIZATION, format!("Bearer {token}"))
                        .header(header::CONTENT_TYPE, ct)
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await;
            assert_eq!(status, StatusCode::OK);
            serde_json::from_slice::<Value>(&bytes).unwrap()["id"]
                .as_str()
                .unwrap()
                .to_string()
        }
    };
    let old = upload("1.0.0").await;
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let new = upload("1.1.0").await;

    let (status, _) = app
        .json(
            "POST",
            &format!("/api/admin/versions/{new}/withdraw"),
            t,
            Some(json!({ "reason": "  " })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "Grund ist Pflicht");
    let (status, v) = app
        .json(
            "POST",
            &format!("/api/admin/versions/{new}/withdraw"),
            t,
            Some(json!({ "reason": "Absturz beim Betreten des Nethers" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["withdrawnReason"], "Absturz beim Betreten des Nethers");

    // Öffentlich: neueste ist wieder die alte, die zurückgezogene steht nur unter `withdrawn`.
    let (_, pack) = app.json("GET", "/api/v1/packs/avalon", None, None).await;
    assert_eq!(pack["latest"]["id"], old.as_str());
    assert_eq!(pack["versions"].as_array().unwrap().len(), 1);
    assert_eq!(pack["withdrawn"][0]["id"], new.as_str());
    assert_eq!(
        pack["withdrawn"][0]["reason"],
        "Absturz beim Betreten des Nethers"
    );
    // Download geht nicht mehr (410 mit Grund).
    let req = Request::post(format!("/api/v1/packs/avalon/versions/{new}/download"))
        .header("x-pinecone-client", "pinecone/1.0.0")
        .body(Body::empty())
        .unwrap();
    let (status, _, bytes) = app.send(req).await;
    assert_eq!(status, StatusCode::GONE);
    assert!(String::from_utf8_lossy(&bytes).contains("Nether"));
    // Wiederherstellen (AK-62.1).
    let (status, _) = app
        .json(
            "POST",
            &format!("/api/admin/versions/{new}/restore"),
            t,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, pack) = app.json("GET", "/api/v1/packs/avalon", None, None).await;
    assert_eq!(pack["latest"]["id"], new.as_str());
    assert!(pack["withdrawn"].as_array().unwrap().is_empty());
    // Im Protokoll.
    let (_, log) = app.json("GET", "/api/admin/audit", t, None).await;
    let actions: Vec<&str> = log
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["action"].as_str().unwrap())
        .collect();
    assert!(
        actions.contains(&"withdraw") && actions.contains(&"restore"),
        "{actions:?}"
    );
}

#[tokio::test]
async fn banner_and_min_launcher_version_in_info() {
    let app = setup().await;
    let token = login(&app).await;
    let t = Some(token.as_str());
    let (_, info) = app.json("GET", "/api/v1/info", None, None).await;
    assert!(info.get("banner").is_none() && info.get("minLauncherVersion").is_none());

    let (status, v) = app
        .json(
            "PUT",
            "/api/admin/settings/main",
            t,
            Some(
                json!({ "bannerText": "Wartung heute ab 20 Uhr", "bannerLevel": "warning",
                         "bannerLink": "http://unsicher.example", "minLauncherVersion": "0.9.8" }),
            ),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "Link nur https: {v}");
    let (status, _) = app
        .json(
            "PUT",
            "/api/admin/settings/main",
            t,
            Some(json!({ "minLauncherVersion": "neu" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, v) = app
        .json(
            "PUT",
            "/api/admin/settings/main",
            t,
            Some(
                json!({ "bannerText": "Wartung heute ab 20 Uhr", "bannerLevel": "warning",
                         "bannerLink": "https://twogg.de/status", "minLauncherVersion": "0.9.8" }),
            ),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let (_, info) = app.json("GET", "/api/v1/info", None, None).await;
    assert_eq!(info["banner"]["text"], "Wartung heute ab 20 Uhr");
    assert_eq!(info["banner"]["level"], "warning");
    assert_eq!(info["banner"]["link"], "https://twogg.de/status");
    assert_eq!(info["minLauncherVersion"], "0.9.8");

    // Abgelaufenes Banner wird nicht mehr ausgeliefert.
    app.json(
        "PUT",
        "/api/admin/settings/main",
        t,
        Some(json!({ "bannerText": "Alt", "bannerUntil": 1 })),
    )
    .await;
    let (_, info) = app.json("GET", "/api/v1/info", None, None).await;
    assert!(info.get("banner").is_none(), "{info}");
}

#[tokio::test]
async fn daily_usage_stats_without_personal_data() {
    let t = setup_with_session(mock_mojang().await).await;
    let token = login(&t).await;
    let admin = Some(token.as_str());

    // Launcher zählen einmal je IP, Version und Tag; fremde Clients gar nicht.
    let info = |ip: &'static str, header: (&'static str, &'static str)| {
        let t = &t;
        async move {
            let req = Request::get("/api/v1/info")
                .header("x-real-ip", ip)
                .header(header.0, header.1)
                .body(Body::empty())
                .unwrap();
            assert_eq!(t.send(req).await.0, StatusCode::OK);
        }
    };
    info("203.0.113.1", ("user-agent", "pinecone/0.9.8")).await;
    info("203.0.113.1", ("user-agent", "pinecone/0.9.8")).await;
    info("203.0.113.2", ("user-agent", "pinecone/0.9.8")).await;
    info("203.0.113.1", ("x-pinecone-client", "pinecone/0.9.7")).await;
    info("203.0.113.3", ("user-agent", "Mozilla/5.0")).await;
    info("203.0.113.3", ("user-agent", "pinecone/kaputt")).await;

    // Aktive Profile: Anmeldung und Präsenz am selben Tag zählen einmal.
    let alice = player_login(&t, "Alice").await;
    t.json(
        "POST",
        "/api/v1/presence",
        Some(&alice),
        Some(json!({ "activity": {} })),
    )
    .await;
    player_login(&t, "Alice").await;
    player_login(&t, "Bob").await;

    // Downloads je Pack-Version.
    t.json(
        "POST",
        "/api/admin/packs",
        admin,
        Some(json!({ "id": "avalon", "title": "Avalon", "visibility": "public" })),
    )
    .await;
    let (ct, body) = multipart(&[("packId", "avalon")], Some(("a.mrpack", &mrpack())));
    let (_, _, bytes) = t
        .send(
            Request::post("/api/admin/versions")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    let vid = serde_json::from_slice::<Value>(&bytes).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    for _ in 0..2 {
        let (_, _, link) =
            launcher_post(&t, &format!("/api/v1/packs/avalon/versions/{vid}/download")).await;
        let path = link["url"]
            .as_str()
            .unwrap()
            .trim_start_matches("https://pinecone.test")
            .to_string();
        let (status, _, _) = t
            .send(Request::get(path).body(Body::empty()).unwrap())
            .await;
        assert_eq!(status, StatusCode::OK);
    }

    let (status, _) = t.json("GET", "/api/admin/stats/daily", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, s) = t
        .json("GET", "/api/admin/stats/daily?days=7", admin, None)
        .await;
    assert_eq!(status, StatusCode::OK, "{s}");
    assert_eq!(s["days"], 7);
    assert_eq!(s["active"].as_array().unwrap().len(), 7);
    assert_eq!(s["active"][6], 2, "{s}");
    assert_eq!(s["launchers"][0]["key"], "0.9.8");
    assert_eq!(s["launchers"][0]["total"], 2);
    assert_eq!(s["launchers"][1]["key"], "0.9.7");
    assert_eq!(s["launchers"].as_array().unwrap().len(), 2, "{s}");
    assert_eq!(s["downloads"][0]["key"], "avalon/1.2.0");
    assert_eq!(s["downloads"][0]["counts"][6], 2);
    let (_, s) = t
        .json("GET", "/api/admin/stats/daily?days=99999", admin, None)
        .await;
    assert_eq!(s["days"], 365);
}

async fn launcher_post(t: &TestApp, uri: &str) -> (StatusCode, axum::http::HeaderMap, Value) {
    let req = Request::post(uri)
        .header("x-pinecone-client", "pinecone/0.9.8")
        .body(Body::empty())
        .unwrap();
    let (status, headers, body) = t.send(req).await;
    (
        status,
        headers,
        serde_json::from_slice(&body).unwrap_or(Value::Null),
    )
}

fn server_test_pack(host: &str, sha512: &str) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(&mut buf);
    let opts = zip::write::SimpleFileOptions::default();
    zip.start_file("modrinth.index.json", opts).unwrap();
    let index = json!({ "formatVersion": 1, "game": "minecraft", "versionId": "6.5.0", "name": "Avalon",
            "files": [
                { "path": "mods/server.jar", "hashes": { "sha1": sha1_hex(b"server-mod"), "sha512": sha512 },
                  "env": { "client": "required", "server": "required" }, "downloads": [format!("{host}/mods/server.jar")] },
                { "path": "mods/client.jar", "hashes": { "sha1": sha1_hex(b"client-mod") },
                  "env": { "client": "required", "server": "unsupported" }, "downloads": [format!("{host}/mods/client.jar")] },
            ],
            "dependencies": { "minecraft": "1.21.1", "fabric-loader": "0.19.5" } });
    zip.write_all(index.to_string().as_bytes()).unwrap();
    for (name, body) in [
        ("overrides/config/twogg.json", "client"),
        ("overrides/server.properties", "motd=Avalon"),
        ("server-overrides/config/twogg.json", "server"),
        ("client-overrides/options.txt", "fov:90"),
    ] {
        zip.start_file(name, opts).unwrap();
        zip.write_all(body.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    buf.into_inner()
}

/// Nachgebaute Download-Hosts und Fabric-Meta für Server-Packs.
async fn mock_downloads() -> String {
    use axum::routing::get;
    let app = axum::Router::new()
        .route("/mods/server.jar", get(|| async { "server-mod" }))
        .route("/mods/client.jar", get(|| async { "client-mod" }))
        .route(
            "/v2/versions/installer",
            get(|| async {
                r#"[{"version":"1.1.0","stable":false},{"version":"1.0.3","stable":true}]"#
            }),
        )
        .route(
            "/v2/versions/loader/1.21.1/0.19.5/1.0.3/server/jar",
            get(|| async { "fabric-launcher" }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn sha512_hex(data: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha512::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn sha1_hex(data: &[u8]) -> String {
    use sha1::Digest;
    sha1::Sha1::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[tokio::test]
async fn server_pack_leaves_out_client_mods() {
    let host = mock_downloads().await;
    let app = setup_full(
        "http://127.0.0.1:9".into(),
        None,
        "http://127.0.0.1:9".into(),
        host.clone(),
    )
    .await;
    let token = login(&app).await;
    let t = Some(token.as_str());
    app.json(
        "POST",
        "/api/admin/packs",
        t,
        Some(json!({ "id": "avalon", "title": "Avalon", "visibility": "public" })),
    )
    .await;

    let pack = server_test_pack(&host, &sha512_hex(b"server-mod"));
    let (ct, body) = multipart(&[("packId", "avalon")], Some(("a.mrpack", &pack)));
    let (status, _, bytes) = app
        .send(
            Request::post("/api/admin/versions")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let vid = serde_json::from_slice::<Value>(&bytes).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let uri = format!("/api/admin/versions/{vid}/server-pack");

    let (status, _) = app.json("GET", &uri, None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (_, s) = app.json("GET", &uri, t, None).await;
    assert_eq!(s["state"], "none");
    let (status, _) = app.json("POST", &format!("{uri}/link"), t, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, s) = app.json("POST", &uri, t, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(s["state"], "building");
    let mut s = Value::Null;
    for _ in 0..100 {
        s = app.json("GET", &uri, t, None).await.1;
        if s["state"] != "building" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(s["state"], "ready", "{s}");
    assert_eq!(s["removed"], json!(["client.jar"]));
    assert_eq!(s["loader"], "Fabric 0.19.5");

    let (_, link) = app.json("POST", &format!("{uri}/link"), t, None).await;
    let url = link["url"].as_str().unwrap();
    assert!(url.starts_with("/dl/server/"), "{url}");
    let (status, headers, file) = app
        .send(Request::get(url).body(Body::empty()).unwrap())
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .contains("avalon-6.5.0-server.zip")
    );
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(file)).unwrap();
    let read = |zip: &mut zip::ZipArchive<std::io::Cursor<Vec<u8>>>, name: &str| {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut zip.by_name(name).unwrap(), &mut s).unwrap();
        s
    };
    assert_eq!(read(&mut zip, "mods/server.jar"), "server-mod");
    assert_eq!(
        read(&mut zip, "fabric-server-launch.jar"),
        "fabric-launcher"
    );
    assert_eq!(read(&mut zip, "config/twogg.json"), "server");
    assert_eq!(read(&mut zip, "server.properties"), "motd=Avalon");
    let info = read(&mut zip, "PINECONE-SERVER.txt");
    assert!(
        info.contains("client.jar") && info.contains("eula=true"),
        "{info}"
    );
    assert!(zip.by_name("mods/client.jar").is_err());
    assert!(zip.by_name("options.txt").is_err());
    assert!(
        zip.by_name("eula.txt").is_err(),
        "EULA nimmt der Betreiber selbst an"
    );

    // Falsche Prüfsumme: Erzeugen schlägt fehl.
    let bad = server_test_pack(&host, &"0".repeat(128));
    let (ct, body) = multipart(
        &[("packId", "avalon"), ("version", "6.5.1")],
        Some(("b.mrpack", &bad)),
    );
    let (_, _, bytes) = app
        .send(
            Request::post("/api/admin/versions")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    let bad_id = serde_json::from_slice::<Value>(&bytes).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let bad_uri = format!("/api/admin/versions/{bad_id}/server-pack");
    app.json("POST", &bad_uri, t, None).await;
    let mut s = Value::Null;
    for _ in 0..100 {
        s = app.json("GET", &bad_uri, t, None).await.1;
        if s["state"] != "building" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(s["state"], "failed", "{s}");
    assert!(s["error"].as_str().unwrap().contains("Prüfsumme"), "{s}");

    // Gefälschter Link und gelöschte Version.
    let (status, _, _) = app
        .send(
            Request::get(format!("/dl/{}", &url[11..]))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_ne!(status, StatusCode::OK, "Server-Link öffnet kein Modpack");
    app.json("DELETE", &format!("/api/admin/versions/{vid}"), t, None)
        .await;
    let (status, _, _) = app
        .send(Request::get(url).body(Body::empty()).unwrap())
        .await;
    assert_ne!(status, StatusCode::OK);
}
