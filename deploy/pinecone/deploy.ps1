<#
  Deployt den Pinecone-Server auf core01 (Stack /opt/stacks/pinecone).
  Aufruf aus dem Server-Repo in PowerShell:

      .\deploy\pinecone\deploy.ps1

  Beim ersten Mal wird nach dem Admin-Passwort gefragt und /opt/stacks/pinecone/.env angelegt.
  Voraussetzungen: VPN aktiv, SSH-Key wie in AGENTS.md, git und OpenSSH unter Windows.
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
$repo = (git rev-parse --show-toplevel).Trim()
$ssh = @("-o", "BatchMode=yes", "-i", $Key)
$target = "$User@$HostIp"

Write-Host "Quellcode packen ($(git -C $repo rev-parse --short HEAD)) ..."
$tar = Join-Path $env:TEMP "pinecone-src.tar.gz"
git -C $repo archive --format=tar.gz -o $tar HEAD

Write-Host "Hochladen nach core01 ..."
scp @ssh $tar "${target}:/tmp/pinecone-src.tar.gz"
scp @ssh (Join-Path $repo "deploy/pinecone/docker-compose.yml") "${target}:/tmp/pinecone-compose.yml"
scp @ssh (Join-Path $repo "deploy/pinecone/.env.example") "${target}:/tmp/pinecone.env.example"

# .env nur beim ersten Deploy anlegen (Passwort lokal abfragen, nie im Repo).
$hasEnv = ssh @ssh $target "test -f $StackDir/.env && echo yes || echo no"
$password = ""
if ($hasEnv.Trim() -ne "yes") {
  $secure = Read-Host "Admin-Passwort für https://pinecone.twogg.app/admin (mind. 10 Zeichen)" -AsSecureString
  $password = [Runtime.InteropServices.Marshal]::PtrToStringAuto([Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure))
  if ($password.Length -lt 10) { throw "Passwort zu kurz" }
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
rm -rf src && mkdir src && tar -xzf /tmp/pinecone-src.tar.gz -C src
cp /tmp/pinecone-compose.yml docker-compose.yml
if [ ! -f .env ]; then
  cp /tmp/pinecone.env.example .env
  grep -v '^PINECONE_ADMIN_PASSWORD=' .env > .env.tmp
  grep -v '^BIND_IP=' .env.tmp > .env.tmp2 && mv .env.tmp2 .env.tmp
  printf 'BIND_IP=%s\n' '$HostIp' >> .env.tmp
  printf 'PINECONE_ADMIN_PASSWORD=%s\n' "`$PINECONE_PW" >> .env.tmp
  mv .env.tmp .env
  chmod 600 .env
fi
DOCKER_BUILDKIT=1 docker compose build
docker compose up -d
for i in `$(seq 1 30); do
  docker compose ps --format '{{.Health}}' | grep -q healthy && break; sleep 2
done
docker compose ps
BIND_IP=`$(sed -n 's/^BIND_IP=//p' .env); PORT=`$(sed -n 's/^PORT=//p' .env)
curl -fsS http://`${BIND_IP}:`${PORT:-8095}/api/v1/info && echo
rm -f /tmp/pinecone-src.tar.gz /tmp/pinecone-compose.yml /tmp/pinecone.env.example
"@
$remote = $remote.Replace("`r`n", "`n")
Write-Host "Bauen und starten (der erste Build dauert einige Minuten) ..."
$remote | ssh @ssh $target "PINECONE_PW='$($password.Replace("'", "'\''"))' bash -s"
Write-Host "Fertig. Nginx Proxy Manager: pinecone.twogg.app -> http://${HostIp}:8095 (siehe deploy/pinecone/README.md)"
