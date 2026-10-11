//! Dateiablage (Modpacks, Bilder), signierte Download-Links und das Speichern von Uploads.

use std::path::{Path, PathBuf};

use axum::extract::multipart::Field;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use sha1::Digest;
use sha2::Sha256;
use tokio::io::AsyncWriteExt;

use crate::{ApiError, ApiResult};

/// Höchstgröße für Bilder (Pack-Icons, Bilder in Neuigkeiten).
pub const MAX_IMAGE: usize = 5 * 1024 * 1024;

pub fn packs_dir(data: &Path) -> PathBuf {
    data.join("packs")
}

pub fn media_dir(data: &Path) -> PathBuf {
    data.join("media")
}

/// Schlüssel für Download-Links; wird beim ersten Start zufällig erzeugt.
pub fn load_or_create_secret(path: &Path) -> anyhow::Result<Vec<u8>> {
    if let Ok(bytes) = std::fs::read(path)
        && bytes.len() >= 32
    {
        return Ok(bytes);
    }
    let mut key = vec![0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut key);
    std::fs::write(path, &key)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(key)
}

type HmacSha256 = Hmac<Sha256>;

/// Download-Token: `<version-id>.<ablauf>.<signatur>` (URL-sicher).
pub fn sign_download(secret: &[u8], version_id: &str, expires: i64) -> String {
    let payload = format!("{version_id}.{expires}");
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC nimmt jede Schlüssellänge");
    mac.update(payload.as_bytes());
    let sig = URL_SAFE_NO_PAD.encode(&mac.finalize().into_bytes()[..18]);
    format!("{payload}.{sig}")
}

/// Prüft ein Token und liefert die Versions-ID, wenn es echt und nicht abgelaufen ist.
pub fn verify_download(secret: &[u8], token: &str, now: i64) -> Option<String> {
    let (payload, sig) = token.rsplit_once('.')?;
    let (version_id, expires) = payload.rsplit_once('.')?;
    let expires: i64 = expires.parse().ok()?;
    if expires < now {
        return None;
    }
    let sig = URL_SAFE_NO_PAD.decode(sig).ok()?;
    let mut mac = HmacSha256::new_from_slice(secret).ok()?;
    mac.update(payload.as_bytes());
    // Vergleich in konstanter Zeit über die gekürzte Signatur.
    mac.verify_truncated_left(&sig).ok()?;
    Some(version_id.to_string())
}

pub struct StoredFile {
    pub path: PathBuf,
    pub size: u64,
    pub sha1: String,
    pub sha512: String,
}

/// Schreibt ein hochgeladenes Feld in `dir/<name>` und berechnet dabei die Hashes.
pub async fn store_field(
    mut field: Field<'_>,
    dir: &Path,
    name: &str,
    max: usize,
) -> ApiResult<StoredFile> {
    tokio::fs::create_dir_all(dir).await?;
    let path = dir.join(name);
    let tmp = dir.join(format!(".{name}.upload"));
    let mut out = tokio::fs::File::create(&tmp).await?;
    let mut sha1 = sha1::Sha1::new();
    let mut sha512 = sha2::Sha512::new();
    let mut size = 0usize;
    let result: ApiResult<()> = async {
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|e| ApiError::bad_request(format!("Upload abgebrochen: {e}")))?
        {
            size += chunk.len();
            if size > max {
                return Err(ApiError::bad_request("Datei ist zu groß"));
            }
            sha1.update(&chunk);
            sha512.update(&chunk);
            out.write_all(&chunk).await?;
        }
        out.flush().await?;
        Ok(())
    }
    .await;
    if let Err(e) = result {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(e);
    }
    tokio::fs::rename(&tmp, &path).await?;
    Ok(StoredFile {
        path,
        size: size as u64,
        sha1: format!("{:x}", sha1.finalize()),
        sha512: format!("{:x}", sha512.finalize()),
    })
}

/// Bild-Upload: nur PNG, JPEG, WebP und GIF (am Inhalt erkannt), mit zufälligem Namen.
pub async fn store_image(field: Field<'_>, data: &Path) -> ApiResult<String> {
    let dir = media_dir(data);
    let tmp_name = format!("{}.upload", uuid::Uuid::new_v4().simple());
    let stored = store_field(field, &dir, &tmp_name, MAX_IMAGE).await?;
    let head = {
        use std::io::Read;
        let mut buf = [0u8; 12];
        let n = std::fs::File::open(&stored.path)?.read(&mut buf)?;
        buf[..n].to_vec()
    };
    let ext = if head.starts_with(&[0x89, b'P', b'N', b'G']) {
        "png"
    } else if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "jpg"
    } else if head.len() >= 12 && &head[0..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        "webp"
    } else if head.starts_with(b"GIF8") {
        "gif"
    } else {
        let _ = std::fs::remove_file(&stored.path);
        return Err(ApiError::bad_request("Nur PNG, JPEG, WebP oder GIF"));
    };
    let name = format!("{}.{ext}", &stored.sha1[..20]);
    std::fs::rename(&stored.path, dir.join(&name))?;
    Ok(name)
}

/// Öffentliche URL eines Bilds aus `media/`.
pub fn media_url(public_url: &str, name: &str) -> String {
    format!("{public_url}/media/{name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_tokens() {
        let key = b"0123456789abcdef0123456789abcdef";
        let token = sign_download(key, "v-123", 1000);
        assert_eq!(verify_download(key, &token, 999).as_deref(), Some("v-123"));
        assert_eq!(verify_download(key, &token, 1001), None, "abgelaufen");
        assert_eq!(
            verify_download(b"anderer-schluessel-anderer-schl", &token, 999),
            None
        );
        let forged = token.replace("v-123.1000", "v-999.1000");
        assert_eq!(verify_download(key, &forged, 999), None);
        let extended = token.replace(".1000.", ".9999.");
        assert_eq!(verify_download(key, &extended, 999), None);
        assert_eq!(verify_download(key, "kaputt", 0), None);
    }
}
