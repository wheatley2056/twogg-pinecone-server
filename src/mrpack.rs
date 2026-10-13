//! Prüfung hochgeladener Modpacks im Modrinth-Format (`.mrpack`), so streng wie der Launcher:
//! Format, Spiel, Loader, Pfade und Hashes.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Component, Path};

use serde::Deserialize;

const INDEX: &str = "modrinth.index.json";
const MAX_INDEX: u64 = 16 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Index {
    format_version: u32,
    game: String,
    version_id: String,
    #[serde(default)]
    files: Vec<IndexFile>,
    #[serde(default)]
    dependencies: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct IndexFile {
    path: String,
    hashes: Hashes,
    #[serde(default)]
    env: Option<Env>,
    #[serde(default)]
    downloads: Vec<String>,
}

#[derive(Deserialize)]
struct Hashes {
    sha1: String,
}

#[derive(Deserialize)]
struct Env {
    client: String,
}

/// Was der Server aus einem Modpack übernimmt.
#[derive(Debug, PartialEq, Eq)]
pub struct Mrpack {
    pub version: String,
    pub minecraft: String,
    /// Loader in Kleinbuchstaben (`fabric`, `quilt`, `forge`, `neoforge`) und seine Version.
    pub loader: Option<(String, String)>,
}

pub fn safe_relative(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('\\')
        && Path::new(name)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

fn is_sha1(s: &str) -> bool {
    s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Liest und prüft ein `.mrpack`. Fehlertexte gehen so an die Admin-Seite.
pub fn read(path: &Path) -> Result<Mrpack, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("Kein ZIP-Archiv: {e}"))?;
    let index: Index = {
        let entry = zip
            .by_name(INDEX)
            .map_err(|_| "Kein Modrinth-Modpack (modrinth.index.json fehlt)".to_string())?;
        if entry.size() > MAX_INDEX {
            return Err("modrinth.index.json ist zu groß".into());
        }
        let mut text = String::new();
        entry
            .take(MAX_INDEX)
            .read_to_string(&mut text)
            .map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| format!("{INDEX}: {e}"))?
    };
    if index.format_version != 1 || index.game != "minecraft" {
        return Err(format!(
            "Nicht unterstütztes Modpack-Format {} für {}",
            index.format_version, index.game
        ));
    }
    let minecraft = index
        .dependencies
        .get("minecraft")
        .cloned()
        .ok_or("Modpack nennt keine Minecraft-Version")?;
    let deps = &index.dependencies;
    let loader = [
        ("neoforge", "neoforge"),
        ("forge", "forge"),
        ("quilt-loader", "quilt"),
        ("fabric-loader", "fabric"),
    ]
    .into_iter()
    .find_map(|(dep, name)| deps.get(dep).map(|v| (name.to_string(), v.clone())));
    for f in &index.files {
        if f.env.as_ref().is_some_and(|e| e.client == "unsupported") {
            continue;
        }
        if !safe_relative(&f.path) || f.downloads.is_empty() || !is_sha1(&f.hashes.sha1) {
            return Err(format!("Ungültiger Dateieintrag {} im Modpack", f.path));
        }
    }
    for i in 0..zip.len() {
        let entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name();
        let rel = ["overrides/", "client-overrides/", "server-overrides/"]
            .iter()
            .find_map(|p| name.strip_prefix(p));
        if let Some(rel) = rel
            && !entry.is_dir()
            && !rel.is_empty()
            && !safe_relative(rel)
        {
            return Err(format!("Unsicherer Pfad: {rel}"));
        }
    }
    Ok(Mrpack {
        version: index.version_id,
        minecraft,
        loader,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(deps: &str) -> tempfile::NamedTempFile {
        use std::io::Write;
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut zip = zip::ZipWriter::new(file.reopen().unwrap());
        zip.start_file(INDEX, zip::write::SimpleFileOptions::default())
            .unwrap();
        write!(
            zip,
            r#"{{"formatVersion":1,"game":"minecraft","versionId":"1.0","name":"P","files":[],"dependencies":{deps}}}"#
        )
        .unwrap();
        zip.finish().unwrap();
        file
    }

    #[test]
    fn reads_loaders() {
        let forge = pack(r#"{"minecraft":"1.20.1","forge":"47.3.0"}"#);
        assert_eq!(
            read(forge.path()).unwrap().loader,
            Some(("forge".into(), "47.3.0".into()))
        );
        let neo = pack(r#"{"minecraft":"1.21.1","neoforge":"21.1.77"}"#);
        assert_eq!(read(neo.path()).unwrap().loader.unwrap().0, "neoforge");
        let quilt = pack(r#"{"minecraft":"1.21.1","fabric-loader":"0.16","quilt-loader":"0.26"}"#);
        assert_eq!(read(quilt.path()).unwrap().loader.unwrap().0, "quilt");
        let vanilla = pack(r#"{"minecraft":"1.21.1"}"#);
        assert_eq!(read(vanilla.path()).unwrap().loader, None);
    }

    #[test]
    fn rejects_unsafe_paths() {
        assert!(safe_relative("mods/a.jar"));
        assert!(!safe_relative("../evil"));
        assert!(!safe_relative("/etc/passwd"));
        assert!(!safe_relative("mods/../../evil"));
        assert!(!safe_relative("mods\\..\\evil"));
        assert!(!safe_relative(""));
    }
}
