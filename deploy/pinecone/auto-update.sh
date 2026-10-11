#!/usr/bin/env bash
# Automatisches Update des Pinecone-Servers auf core01 (läuft per Cron alle 5 Minuten, siehe
# deploy.ps1). Holt das neueste Image aus der GitHub Container Registry; ist es neu:
#   1. Server anhalten und Datenbank + Schlüssel sichern (backups/, die letzten 10 bleiben),
#   2. mit dem neuen Image starten und warten, bis der Healthcheck „healthy“ meldet,
#   3. sonst Datenbank zurückspielen und das vorige Image wieder starten. Ein Image, das so
#      gescheitert ist, wird nicht noch einmal versucht.
# Modpacks und Bilder (packs/, media/) werden nur ergänzt, nie umgeschrieben; sie sichert das
# reguläre Backup (siehe README).
set -euo pipefail
cd "$(dirname "$0")"

exec 9>.auto-update.lock
flock -n 9 || exit 0

log() { echo "$(date '+%F %T') $*"; }

CONTAINER=pinecone-server
VOLUME=pinecone_data
HELPER=debian:bookworm-slim
FILES="pinecone.db pinecone.db-wal pinecone.db-shm secret.key"

image=$(docker compose config --images | head -n1)
docker compose pull --quiet pinecone
new=$(docker image inspect --format '{{.Id}}' "$image")
running=$(docker inspect --format '{{.Image}}' "$CONTAINER" 2>/dev/null || true)

[ "$new" = "$running" ] && exit 0
if [ -f .failed-image ] && [ "$(cat .failed-image)" = "$new" ]; then
  exit 0
fi

log "Neues Image ${new:7:12} (bisher ${running:7:12})"
mkdir -p backups
backup=""
if [ -n "$running" ]; then
  docker tag "$running" pinecone-server:previous
  docker compose stop pinecone
  backup="pinecone-db-$(date +%F-%H%M%S).tgz"
  if ! docker run --rm -v "$VOLUME":/data:ro -v "$PWD/backups":/backup "$HELPER" \
    sh -c "cd /data && tar czf /backup/$backup \$(ls $FILES 2>/dev/null)"; then
    # Ohne Sicherung kein Update: alten Stand wieder starten.
    log "Sicherung fehlgeschlagen, Update abgebrochen"
    PINECONE_IMAGE=pinecone-server:previous docker compose up -d pinecone
    exit 1
  fi
  ls -1t backups/pinecone-db-*.tgz | tail -n +11 | xargs -r rm --
  log "Gesichert: backups/$backup"
fi

docker compose up -d pinecone
for _ in $(seq 1 40); do
  status=$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}' "$CONTAINER" 2>/dev/null || echo missing)
  if [ "$status" = healthy ]; then
    log "Update erfolgreich"
    rm -f .failed-image
    docker image prune -f >/dev/null
    exit 0
  fi
  sleep 3
done

log "Neuer Server wird nicht gesund (Status: $status); Logs:"
docker compose logs --tail 30 pinecone || true
echo "$new" > .failed-image
if [ -n "$running" ]; then
  log "Zurück auf das vorige Image"
  docker compose stop pinecone
  docker run --rm -v "$VOLUME":/data -v "$PWD/backups":/backup "$HELPER" \
    sh -c "cd /data && rm -f $FILES && tar xzf /backup/$backup"
  PINECONE_IMAGE=pinecone-server:previous docker compose up -d pinecone
fi
exit 1
