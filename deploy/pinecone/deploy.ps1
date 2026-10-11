<#
  Richtet den Pinecone-Server auf core01 ein (Stack /opt/stacks/pinecone) bzw. aktualisiert die
  Stack-Dateien. Den Server selbst aktualisiert danach core01 automatisch: GitHub baut bei jedem
  Merge auf main ein Image (ghcr.io), auto-update.sh holt es per Cron alle 5 Minuten.

      .\deploy\pinecone\deploy.ps1

  Beim ersten Mal fragt das Skript nach dem Admin-Passwort (für .env) und nach einem GitHub-Token
  mit dem Recht read:packages (für das Image aus der privaten Registry).
  Voraussetzungen: VPN aktiv, SSH-Key wie in AGENTS.md, OpenSSH unter Windows.
  Host, Benutzer und Key stehen in deploy.local.psd1 neben diesem Skript (nicht im Repo):

      @{ Host = "<VPN-IP>"; User = "<ssh-user>"; Key = "C:\Users\<name>\.ssh\<key>" }
#>
param(
  [string]$HostIp,
  [string]$User,
  [string]$Key,
  [string]$StackDir = "/opt/stacks/pinecone"
)
$ErrorActionPreference = "Stop"
$local = Join-Path $PSScriptRoot "deploy.local.psd1"
if (Test-Path $local) {
  $cfg = Import-PowerShellDataFile $local
  if (-not $HostIp) { $HostIp = $cfg.Host }
  if (-not $User) { $User = $cfg.User }
  if (-not $Key) { $Key = $cfg.Key }
}
if (-not $HostIp) { $HostIp = Read-Host "VPN-IP von core01" }
if (-not $User) { $User = Read-Host "SSH-Benutzer" }
if (-not $Key) { $Key = Read-Host "Pfad zum SSH-Key" }
$here = $PSScriptRoot
$ssh = @("-o", "BatchMode=yes", "-i", $Key)
$target = "$User@$HostIp"

Write-Host "Stack-Dateien hochladen ..."
scp @ssh (Join-Path $here "docker-compose.yml") "${target}:/tmp/pinecone-compose.yml"
scp @ssh (Join-Path $here ".env.example") "${target}:/tmp/pinecone.env.example"
scp @ssh (Join-Path $here "auto-update.sh") "${target}:/tmp/pinecone-auto-update.sh"

# .env nur beim ersten Deploy anlegen (Passwort lokal abfragen, nie im Repo).
$hasEnv = ssh @ssh $target "test -f $StackDir/.env && echo yes || echo no"
$password = ""
if ($hasEnv.Trim() -ne "yes") {
  $secure = Read-Host "Admin-Passwort für https://pinecone.twogg.app/admin (mind. 10 Zeichen)" -AsSecureString
  $password = [Runtime.InteropServices.Marshal]::PtrToStringAuto([Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure))
  if ($password.Length -lt 10) { throw "Passwort zu kurz" }
}

# Anmeldung an ghcr.io nur, wenn core01 noch nicht angemeldet ist.
$hasLogin = ssh @ssh $target "grep -q ghcr.io ~/.docker/config.json 2>/dev/null && echo yes || echo no"
$token = ""
if ($hasLogin.Trim() -ne "yes") {
  Write-Host "core01 braucht Lesezugriff auf das Server-Image (ghcr.io/wheatley2056/twogg-pinecone-server)."
  Write-Host "GitHub → Settings → Developer settings → Personal access tokens (classic), Recht: read:packages"
  $secureToken = Read-Host "GitHub-Token" -AsSecureString
  $token = [Runtime.InteropServices.Marshal]::PtrToStringAuto([Runtime.InteropServices.Marshal]::SecureStringToBSTR($secureToken))
  if (-not $token) { throw "Ohne Token kann core01 das Image nicht laden" }
}

# Stack-Ordner einmalig anlegen (braucht sudo; fragt interaktiv nach dem sudo-Passwort).
$writable = ssh @ssh $target "test -w $StackDir && echo yes || echo no"
if ($writable.Trim() -ne "yes") {
  Write-Host "Lege $StackDir an (sudo-Passwort von core01 eingeben) ..."
  ssh -t -i $Key $target "sudo mkdir -p $StackDir && sudo chown `$(id -u):`$(id -g) $StackDir"
}

$remote = @"
set -e
cd $StackDir
cp /tmp/pinecone-compose.yml docker-compose.yml
tr -d '\r' < /tmp/pinecone-auto-update.sh > auto-update.sh
chmod +x auto-update.sh
if [ ! -f .env ]; then
  cp /tmp/pinecone.env.example .env
  grep -v '^PINECONE_ADMIN_PASSWORD=' .env > .env.tmp
  grep -v '^BIND_IP=' .env.tmp > .env.tmp2 && mv .env.tmp2 .env.tmp
  printf 'BIND_IP=%s\n' '$HostIp' >> .env.tmp
  printf 'PINECONE_ADMIN_PASSWORD=%s\n' "`$PINECONE_PW" >> .env.tmp
  mv .env.tmp .env
  chmod 600 .env
fi
if [ -n "`$GHCR_TOKEN" ]; then
  printf '%s' "`$GHCR_TOKEN" | docker login ghcr.io -u wheatley2056 --password-stdin
fi
# Früherer Quellcode-Deploy wird nicht mehr gebraucht.
rm -rf src
# Cronjob für das automatische Update (einmalig).
line="*/5 * * * * $StackDir/auto-update.sh >> $StackDir/auto-update.log 2>&1"
( crontab -l 2>/dev/null | grep -v 'pinecone.*auto-update.sh' ; echo "`$line" ) | crontab -
./auto-update.sh || true
docker compose ps
BIND_IP=`$(sed -n 's/^BIND_IP=//p' .env); PORT=`$(sed -n 's/^PORT=//p' .env)
curl -fsS http://`${BIND_IP}:`${PORT:-8095}/api/v1/info && echo
rm -f /tmp/pinecone-compose.yml /tmp/pinecone.env.example /tmp/pinecone-auto-update.sh
"@
# Geheimnisse über stdin statt in der Kommandozeile (sonst kurz in `ps` auf core01 sichtbar).
$secrets = "export PINECONE_PW='$($password.Replace("'", "'\''"))'`nexport GHCR_TOKEN='$($token.Replace("'", "'\''"))'`n"
$remote = ($secrets + $remote).Replace("`r`n", "`n")
Write-Host "Einrichten und starten ..."
$remote | ssh @ssh $target "bash -s"
Write-Host "Fertig. Updates holt core01 ab jetzt selbst (Log: $StackDir/auto-update.log)."
Write-Host "Nginx Proxy Manager: pinecone.twogg.app -> http://${HostIp}:8095 (siehe deploy/pinecone/README.md)"
