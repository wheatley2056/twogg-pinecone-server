# Pinecone-Server auf core01

Der Server liefert dem Launcher Neuigkeiten und TwoGG-Modpacks und hat unter `/admin` eine
Verwaltungsoberfläche (React-Admin). Ein Container, Daten (SQLite, Modpacks, Bilder) im Volume
`pinecone_data`.

| | |
|---|---|
| Stack | `/opt/stacks/pinecone/` (`docker-compose.yml`, `.env`, `src/`) |
| Container | `pinecone-server`, lauscht auf `<BIND_IP>:8095` (nur VPN/intern) |
| Öffentlich | `https://pinecone.twogg.app` über Nginx Proxy Manager |
| Admin | `https://pinecone.twogg.app/admin` (Konto aus `.env`) |

## Deploy

Vom Windows-PC aus (VPN an, SSH-Key wie in AGENTS.md), im Repo auf dem Branch mit `crates/server`.
Zugangsdaten gehören nicht ins Repo: Einmalig `deploy/pinecone/deploy.local.psd1` anlegen (ist in
`.gitignore`), sonst fragt das Skript danach:

```powershell
@{ Host = "<VPN-IP von core01>"; User = "<ssh-user>"; Key = "C:\Users\<name>\.ssh\<key>" }
```

Dann:

```powershell
.\deploy\pinecone\deploy.ps1
```

Das Skript packt den aktuellen Commit (`git archive`), lädt ihn nach core01, baut das Image dort
(`docker compose build`, beim ersten Mal einige Minuten) und startet den Stack. Beim ersten Deploy
fragt es nach dem Admin-Passwort und legt `/opt/stacks/pinecone/.env` an (`chmod 600`); spätere
Deploys lassen `.env` und die Daten unangetastet. Der Stack erscheint in Portainer unter „Stacks“
als extern verwalteter Stack `pinecone`.

Manuell auf core01 geht dasselbe mit:

```bash
cd /opt/stacks/pinecone && docker compose up -d --build
docker compose logs -f
```

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
