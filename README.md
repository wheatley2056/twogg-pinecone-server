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
| `PINECONE_LAUNCHER_URL` | Download-Seite des Launchers, verlinkt auf den Modpack-Seiten (Standard: Landing Page auf GitHub Pages) |
| `PINECONE_CURSEFORGE_API_KEY` | CurseForge-API-Schlüssel für den Proxy (siehe unten). Ohne ihn ist CurseForge im Launcher ausgeblendet |
| `PINECONE_CURSEFORGE_API` | CurseForge-API-Adresse (Standard `https://api.curseforge.com`, für Tests) |
| `PINECONE_PLAYER_RETENTION_DAYS` | Spielerprofile der Freundesliste, die so viele Tage nicht gesehen wurden, werden gelöscht (Standard 365, `0` = nie) |

Datenbank-Migrationen laufen beim Start automatisch.

## API für den Launcher (`/api/v1`)

| Endpunkt | |
|---|---|
| `GET /info` | Server-Infos, Community-Server (Name, Adresse), `curseforge` (Proxy verfügbar), `legal` (Impressum, Datenschutz, Kontakt) |
| `GET /news?limit=10` | sichtbare Neuigkeiten (Markdown als HTML) |
| `GET /packs`, `GET /packs/{id}` | öffentliche Modpacks mit Versionen, optional `eventId` (twogg.app-Event) |
| `POST /packs/{id}/versions/{version}/download` | signierter Download-Link (Header `X-Pinecone-Client: pinecone/<version>`) |
| `GET /recommendations?type=modpack\|mod\|resourcepack\|shader\|datapack` | TwoGG-Auswahl |
| `POST /auth/challenge`, `POST /auth/verify` | Freunde-Anmeldung mit dem Minecraft-Konto (Mojang `join` / `hasJoined`), Sitzung 30 Tage |
| `GET /me`, `POST /presence` | eigenes Profil, Präsenz mit Aktivität (online = letzte 90 s) |
| `DELETE /me` | eigenes Spielerprofil mit Sitzungen, Anfragen und Freundschaften löschen |
| `GET /curseforge/v1/mods/search`, `GET /curseforge/v1/mods/{id}/files`, `POST /curseforge/v1/mods/files`, `POST /curseforge/v1/mods` | CurseForge-Proxy (Header `X-Pinecone-Client`, 240 Anfragen/min je IP) |
| `GET /friends`, `POST /friends/requests`, `POST /friends/requests/{uuid}/accept`, `DELETE /friends/requests/{uuid}`, `DELETE /friends/{uuid}` | Freunde, Anfragen (per Spielername `{name}` oder Freundescode `{code}`) und Einladungen (`invites`, `sentInvites`) |
| `GET /friends/code`, `POST /friends/code` | eigener Freundescode `XXXX-XXXX-XX`; `POST` erzeugt einen neuen, der alte gilt sofort nicht mehr. Höchstens 10 falsche Codes je Spieler und Stunde |
| `POST /invites` `{to, server, pack?, instance?}`, `POST /invites/{id}/answer` `{accept}` | Freund auf einen Server einladen (15 min gültig, höchstens 10 je Minute; `pack` nur als `pinecone://`-Link) |
| `POST /sync-shares` `{data}`, `GET /sync-shares/{code}` | Sync-Pack teilen (≤ 64 KB, 30 Tage, höchstens 20 je Spieler); Abruf nur angemeldet |

`GET /packs/{id}` in der API enthält `pageUrl`, die öffentliche Seite des Packs.

## CurseForge

Die [CurseForge-API-Bedingungen](https://support.curseforge.com/en/support/solutions/articles/9000207405)
erlauben nicht, den API-Schlüssel weiterzugeben, und nicht, API-Daten zu speichern oder zu cachen.
Darum liegt der Schlüssel nur auf diesem Server (`PINECONE_CURSEFORGE_API_KEY`), und der Launcher
fragt über `/api/v1/curseforge/...`. Der Proxy reicht nur die vier Abfragen weiter, die der Launcher
braucht, setzt die Spiel-ID fest auf Minecraft (432), verwirft andere Parameter und speichert
nichts (`Cache-Control: no-store`). Den Schlüssel beantragt man über das Formular auf
[docs.curseforge.com](https://docs.curseforge.com/rest-api/) für „TwoGG Pinecone“.

## Datenschutz

- Spielerprofile der Freundesliste (UUID, Name, letzte Aktivität) löscht der Spieler selbst über
  `DELETE /api/v1/me` (Launcher: Einstellungen › Konten). Profile ohne Aktivität seit
  `PINECONE_PLAYER_RETENTION_DAYS` Tagen und abgelaufene Sitzungen löscht der Server täglich.
- IP-Adressen hält der Server nur im Speicher für die Ratenbegrenzung, nicht in der Datenbank.
- Impressum, Datenschutzerklärung und Kontakt-E-Mail pflegt man unter Einstellungen in `/admin`.
  Die Modpack-Seiten zeigen sie zusammen mit dem Hinweis „Kein offizielles Minecraft-Produkt“
  (verlangt von den [Minecraft-Nutzungsrichtlinien](https://www.minecraft.net/en-us/usage-guidelines)).

## Modpack-Seiten

`/packs` listet die öffentlichen TwoGG-Modpacks, `/packs/{id}` ist die Seite eines Packs (auch für
ungelistete Packs, nicht für versteckte), aufgebaut wie eine CurseForge-Projektseite: Kopf mit Logo
und Kennzahlen, Tabs „Beschreibung“ und „Versionen“, Seitenleiste mit Details und Event-Link.
„In Pinecone installieren“ öffnet `pinecone://twogg/<pack>` bzw. je Version
`pinecone://twogg/<pack>/<version>` im Launcher. `/` leitet auf `/packs` weiter. Die Seiten laden
keine Skripte und nur eigene Schriften (`assets/fonts`, OFL), Vorschau-Tags für Discord sind gesetzt.

`GET /dl/{token}` liefert die `.mrpack`-Datei. Die Admin-API liegt unter `/api/admin` (Sitzung per
Bearer-Token, 12 Stunden gültig, Sperre nach Fehlversuchen).

Die Verträge sind mit dem Launcher abgestimmt (`crates/core/src/twogg.rs` dort). Änderungen an der
API bitte rückwärtskompatibel halten, da ältere Launcher-Versionen im Umlauf sind.

## Deploy

Docker-Stack auf core01 mit Nginx Proxy Manager und Backup: [deploy/pinecone/README.md](deploy/pinecone/README.md).
Updates laufen automatisch: Ein Merge auf `main` baut das Image (`ghcr.io`), core01 holt es
innerhalb von 5 Minuten, sichert vorher die Datenbank und fällt bei Problemen zurück.

## Lizenz

MIT oder Apache-2.0, nach Wahl ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)).
