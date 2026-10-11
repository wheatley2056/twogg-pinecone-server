# TwoGG Pinecone Server

Server hinter `https://pinecone.twogg.app` für den [Pinecone-Launcher](https://github.com/wheatley2056/twogg-pinecone):
Neuigkeiten, TwoGG-Modpacks (nur über kurzlebige, signierte Links), die TwoGG-Auswahl für „Entdecken“,
den Community-Server und die Pinecone-Freundesliste. Verwaltet wird alles unter `/admin`
(React-Admin, Quellcode in `admin/`).

```
src/          # Rust-Server (axum, SQLite über sqlx)
admin/        # Admin-Oberfläche (React-Admin, Vite)
deploy/       # Docker-Stack und Deploy-Skript für core01
Dockerfile    # ein Image: Server + gebaute Admin-Oberfläche
```

## Entwicklung

```sh
cd admin && npm ci && npm run build && cd ..
PINECONE_ADMIN_PASSWORD=ein-langes-passwort PINECONE_ADMIN_DIR=admin/dist PINECONE_DATA_DIR=data cargo run
# → http://localhost:8080/admin ; der Launcher nutzt ihn mit PINECONE_SERVER_URL=http://localhost:8080
```

Prüfen: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`,
`cd admin && npm run check`. Die Admin-Oberfläche im Entwicklungsmodus: `cd admin && npm run dev`
(leitet `/api` an `localhost:8080` weiter).

## Konfiguration

| Variable | Bedeutung |
|---|---|
| `PINECONE_ADDR` | Lauschadresse (Standard `0.0.0.0:8080`) |
| `PINECONE_DATA_DIR` | Daten: `pinecone.db`, `packs/`, `media/`, `secret.key` |
| `PINECONE_PUBLIC_URL` | Öffentliche Adresse (für Links in Antworten) |
| `PINECONE_ADMIN_USER` | Admin-Benutzername (Standard `admin`) |
| `PINECONE_ADMIN_PASSWORD` / `_HASH` | Passwort im Klartext oder als Argon2-Hash (`pinecone-server hash-password '…'`) |
| `PINECONE_ADMIN_DIR` | Gebaute Admin-Oberfläche (`admin/dist`) |
| `PINECONE_DOWNLOAD_TTL` | Gültigkeit der Download-Links in Sekunden (Standard 600) |
| `PINECONE_SESSION_SERVER` | Mojang-Session-Server für die Freunde-Anmeldung (Standard `https://sessionserver.mojang.com`) |

Datenbank-Migrationen laufen beim Start automatisch.

## API für den Launcher (`/api/v1`)

| Endpunkt | |
|---|---|
| `GET /info` | Server-Infos, Community-Server (Name, Adresse) |
| `GET /news?limit=10` | sichtbare Neuigkeiten (Markdown als HTML) |
| `GET /packs`, `GET /packs/{id}` | öffentliche Modpacks mit Versionen, optional `eventId` (twogg.app-Event) |
| `POST /packs/{id}/versions/{version}/download` | signierter Download-Link (Header `X-Pinecone-Client: pinecone/<version>`) |
| `GET /recommendations?type=modpack\|mod\|resourcepack\|shader\|datapack` | TwoGG-Auswahl |
| `POST /auth/challenge`, `POST /auth/verify` | Freunde-Anmeldung mit dem Minecraft-Konto (Mojang `join` / `hasJoined`), Sitzung 30 Tage |
| `GET /me`, `POST /presence` | eigenes Profil, Präsenz mit Aktivität (online = letzte 90 s) |
| `GET /friends`, `POST /friends/requests`, `POST /friends/requests/{uuid}/accept`, `DELETE /friends/requests/{uuid}`, `DELETE /friends/{uuid}` | Freunde und Anfragen (per Spielername) |

`GET /dl/{token}` liefert die `.mrpack`-Datei. Die Admin-API liegt unter `/api/admin` (Sitzung per
Cookie, Sperre nach Fehlversuchen).

Die Verträge sind mit dem Launcher abgestimmt (`crates/core/src/twogg.rs` dort). Änderungen an der
API bitte rückwärtskompatibel halten, da ältere Launcher-Versionen im Umlauf sind.

## Deploy

Docker-Stack auf core01 mit Nginx Proxy Manager und Backup: [deploy/pinecone/README.md](deploy/pinecone/README.md).
