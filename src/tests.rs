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
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        addr: "127.0.0.1:0".parse().unwrap(),
        data_dir: dir.path().join("data"),
        public_url: "https://pinecone.test".into(),
        admin_user: "admin".into(),
        admin_password_hash: crate::auth::hash_password("geheimes-passwort").unwrap(),
        admin_dir: dir.path().join("admin"),
        download_ttl: 600,
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
