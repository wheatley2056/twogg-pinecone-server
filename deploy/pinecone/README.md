# Pinecone-Server auf core01

Der Server liefert dem Launcher Neuigkeiten und TwoGG-Modpacks und hat unter `/admin` eine
Verwaltungsoberfläche (React-Admin). Ein Container, Daten (SQLite, Modpacks, Bilder) im Volume
`pinecone_data`.

| | |
|---|---|
| Stack | `/opt/stacks/pinecone/` (`docker-compose.yml`, `.env`, `auto-update.sh`, `backups/`) |
| Container | `pinecone-server`, lauscht auf `<BIND_IP>:8095` (nur VPN/intern) |
| Öffentlich | `https://pinecone.twogg.app` über Nginx Proxy Manager |
| Admin | `https://pinecone.twogg.app/admin` (Konto aus `.env`) |

## Deploy und automatische Updates

**Wie Updates laufen:** Jeder Merge auf `main` im Server-Repo baut nach grüner CI ein Image
`ghcr.io/wheatley2056/twogg-pinecone-server:latest` (dazu `:sha-<commit>`). Auf core01 prüft
`auto-update.sh` per Cron alle 5 Minuten, ob es ein neues Image gibt. Wenn ja:

1. Server anhalten, Datenbank und `secret.key` nach `backups/` sichern (die letzten 10 bleiben),
2. mit dem neuen Image starten und auf den Healthcheck warten,
3. wird der Server nicht gesund: Datenbank zurückspielen, vorheriges Image starten und das
   gescheiterte Image nicht noch einmal versuchen (bis ein neueres kommt).

Datenbank-Migrationen laufen beim Start automatisch. Die kurze Pause (wenige Sekunden) fällt nur
bei echten Updates an.

**Einmalig einrichten** (Windows-PC, VPN an, SSH-Key wie in AGENTS.md), im Server-Repo.
Zugangsdaten gehören nicht ins Repo: `deploy/pinecone/deploy.local.psd1` anlegen (ist in
`.gitignore`), sonst fragt das Skript danach:

```powershell
@{ Host = "<VPN-IP von core01>"; User = "<ssh-user>"; Key = "C:\Users\<name>\.ssh\<key>" }
```

```powershell
.\deploy\pinecone\deploy.ps1
```

Das Skript lädt `docker-compose.yml`, `.env.example` und `auto-update.sh` nach
`/opt/stacks/pinecone/`, legt beim ersten Mal `.env` an (fragt nach dem Admin-Passwort, `chmod 600`),
meldet core01 bei `ghcr.io` an (fragt nach einem GitHub-Token, *Personal access token (classic)* mit
nur `read:packages`) und trägt den Cronjob ein. Danach startet es einmal `auto-update.sh`. Erneut
ausführen muss man es nur, wenn sich `docker-compose.yml` oder `auto-update.sh` ändern.

**Auf core01:**

```bash
cd /opt/stacks/pinecone
tail -f auto-update.log            # was das Auto-Update tut
./auto-update.sh                   # sofort nach Updates sehen
docker compose logs -f             # Server-Logs
crontab -l                         # Cronjob prüfen; zum Pausieren die Zeile auskommentieren
```

Eine bestimmte Version festhalten: in `.env` `PINECONE_IMAGE=ghcr.io/wheatley2056/twogg-pinecone-server:sha-<commit>`
setzen und `docker compose up -d`; zurück zum automatischen Stand die Zeile wieder entfernen.
Ein `.failed-image` im Stack-Ordner zeigt ein Image, das beim letzten Versuch nicht gesund wurde.

## Nginx Proxy Manager (einmalig)

Proxy Host anlegen:

- Domain: `pinecone.twogg.app`
- Scheme/Forward: `http` → `<BIND_IP>` : `8095`
- „Block Common Exploits“ an, Websockets nicht nötig
- SSL: Let's Encrypt (oder Cloudflare-Origin-Zertifikat, die Domain läuft über Cloudflare),
  „Force SSL“ an
- **Keine** Access-List für den Host insgesamt: Launcher von überall müssen `/api/v1`, `/dl` und
  `/media` erreichen.
- Optional enger: unter „Custom locations“ `/admin` und `/api/admin` mit Access-List auf
  das VPN-Netz oder Authelia-Forward-Auth. Die Admin-Anmeldung hat zusätzlich
  eigene Sperre nach Fehlversuchen.
- Uploads: Advanced → `client_max_body_size 1024m;`. Achtung: Über Cloudflare (Free) sind
  Uploads auf 100 MB begrenzt; größere Modpacks direkt über `http://<BIND_IP>:8095/admin` (VPN)
  hochladen.

## Backup

Alles Wichtige liegt im Volume `pinecone_data` (`pinecone.db`, `packs/`, `media/`, `secret.key`).

```bash
docker run --rm -v pinecone_data:/data -v "$PWD":/backup debian:bookworm-slim \
  tar czf /backup/pinecone-data-$(date +%F).tgz -C /data .
```

`secret.key` signiert die Download-Links; geht er verloren, entsteht ein neuer (alte Links laufen
ohnehin nach 10 Minuten ab).

## Konfiguration (`.env`)

| Variable | Bedeutung |
|---|---|
| `PINECONE_PUBLIC_URL` | Öffentliche Adresse (für Links in Antworten) |
| `PINECONE_ADMIN_USER` | Admin-Benutzername (Standard `admin`) |
| `PINECONE_ADMIN_PASSWORD` / `_HASH` | Passwort im Klartext oder als Argon2-Hash (`docker compose run --rm pinecone hash-password '…'`) |
| `PINECONE_DOWNLOAD_TTL` | Gültigkeit der Download-Links in Sekunden (Standard 600) |
| `BIND_IP`, `PORT` | Wo der Container auf core01 lauscht (NPM-Ziel) |
| `PINECONE_IMAGE` | Optional: festes Image statt `:latest` (siehe oben) |

## API (für den Launcher)

- `GET /api/v1/news?limit=10`: sichtbare Neuigkeiten (Markdown als HTML)
- `GET /api/v1/packs`: öffentliche Modpacks mit neuester Version
- `GET /api/v1/packs/<id>`: Details und alle Versionen
- `POST /api/v1/packs/<id>/versions/<version>/download` mit Header `X-Pinecone-Client: pinecone/<version>`:
  signierter Download-Link (10 min gültig, 30 Links je IP und 10 min)
- `GET /dl/<token>`: die `.mrpack`-Datei

„Nur über Pinecone“ heißt: Die Dateien sind nicht öffentlich verlinkt, Links laufen schnell ab und
werden nur mit Pinecone-Kennung ausgegeben. Wer es darauf anlegt, kann die Anfrage nachbauen; echter
Kopierschutz ist bei einem Open-Source-Launcher nicht möglich.
