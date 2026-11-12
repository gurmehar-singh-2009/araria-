//! The project format, version 1.
//!
//! A project is a directory:
//!
//! ```text
//! project.json            manifest (this module)
//! scenes/main.scene.json  instance tree (see `scene`)
//! scripts/*.lua           Lua sources referenced by Script instances
//! assets/...              imported models, textures and audio
//! thumbnail.png           optional cover image
//! ```
//!
//! The same layout is used on disk for native builds and in browser storage,
//! and a project can be packed into a single portable `.araria` bundle file
//! to move between the two.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROJECT_FORMAT: &str = "araria.project";
pub const PROJECT_VERSION: u32 = 1;
pub const BUNDLE_FORMAT: &str = "araria.bundle";
pub const BUNDLE_VERSION: u32 = 1;

pub const MANIFEST_FILE: &str = "project.json";
pub const DEFAULT_SCENE: &str = "scenes/main.scene.json";
pub const SCRIPTS_DIR: &str = "scripts";
pub const ASSETS_DIR: &str = "assets";
pub const THUMBNAIL_FILE: &str = "thumbnail.png";

pub const CATEGORIES: &[&str] =
    &["Obby", "Sandbox", "Racing", "Survival", "Platformer", "Arena", "Adventure", "Other"];

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("not an Araria {0} file")]
    WrongFormat(&'static str),
    #[error("{what} version {found} is newer than this build supports ({supported})")]
    TooNew { what: &'static str, found: u32, supported: u32 },
    #[error("bundle entry '{0}' is corrupt")]
    CorruptEntry(String),
    #[error("unsafe path in bundle: '{0}'")]
    UnsafePath(String),
}

/// An asset pack or engine feature the project relies on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dependency {
    pub name: String,
    pub version: String,
}

fn default_scene() -> String {
    DEFAULT_SCENE.to_string()
}
fn default_category() -> String {
    "Other".to_string()
}
fn default_max_players() -> u32 {
    8
}

/// Contents of `project.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectManifest {
    pub format: String,
    pub version: u32,
    /// Stable identifier, unique within a library.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Free-form credit line. There are no accounts, so this is just text.
    #[serde(default)]
    pub author: String,
    #[serde(default = "default_category")]
    pub category: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Scene opened when the project starts.
    #[serde(default = "default_scene")]
    pub scene: String,
    #[serde(default = "default_max_players")]
    pub max_players: u32,
    /// Seconds since the Unix epoch.
    #[serde(default)]
    pub created: u64,
    #[serde(default)]
    pub modified: u64,
    /// Version of the engine that last saved the project.
    #[serde(default)]
    pub engine: String,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
    /// Editorial ordering weight for the built-in catalog (higher first).
    #[serde(default)]
    pub featured: u32,
    /// Short hints shown on the game's detail page.
    #[serde(default)]
    pub controls: Vec<String>,
}

impl ProjectManifest {
    pub fn new(id: &str, name: &str, now: u64) -> Self {
        ProjectManifest {
            format: PROJECT_FORMAT.to_string(),
            version: PROJECT_VERSION,
            id: id.to_string(),
            name: name.to_string(),
            description: String::new(),
            author: String::new(),
            category: default_category(),
            tags: Vec::new(),
            scene: default_scene(),
            max_players: default_max_players(),
            created: now,
            modified: now,
            engine: env!("CARGO_PKG_VERSION").to_string(),
            dependencies: vec![Dependency { name: "araria.core".into(), version: "1".into() }],
            featured: 0,
            controls: Vec::new(),
        }
    }

    pub fn parse(text: &str) -> Result<Self, ProjectError> {
        let manifest: ProjectManifest = serde_json::from_str(text)?;
        if manifest.format != PROJECT_FORMAT {
            return Err(ProjectError::WrongFormat("project"));
        }
        if manifest.version > PROJECT_VERSION {
            return Err(ProjectError::TooNew { what: "project", found: manifest.version, supported: PROJECT_VERSION });
        }
        Ok(manifest)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("manifest is always serializable")
    }
}

/// Turns a display name into a safe identifier / file-name stem.
pub fn slugify(name: &str) -> String {
    let mut slug = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-').to_string();
    if slug.is_empty() { "project".to_string() } else { slug }
}

/// Whether a bundle/project-relative path stays inside its project.
pub fn is_safe_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains(':')
        && path.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
}

#[derive(Serialize, Deserialize)]
struct BundleEntry {
    /// `"text"` for UTF-8 content stored verbatim, `"base64"` otherwise.
    encoding: String,
    data: String,
}

#[derive(Serialize, Deserialize)]
struct BundleDoc {
    format: String,
    version: u32,
    files: BTreeMap<String, BundleEntry>,
}

/// Packs project files (relative path → bytes) into one portable document.
pub fn pack_bundle(files: &BTreeMap<String, Vec<u8>>) -> String {
    let files = files
        .iter()
        .map(|(path, bytes)| {
            let entry = match std::str::from_utf8(bytes) {
                Ok(text) => BundleEntry { encoding: "text".into(), data: text.to_string() },
                Err(_) => BundleEntry { encoding: "base64".into(), data: base64_encode(bytes) },
            };
            (path.clone(), entry)
        })
        .collect();
    let doc = BundleDoc { format: BUNDLE_FORMAT.to_string(), version: BUNDLE_VERSION, files };
    serde_json::to_string(&doc).expect("bundle is always serializable")
}

/// Unpacks a bundle, rejecting paths that would escape the project folder.
pub fn unpack_bundle(text: &str) -> Result<BTreeMap<String, Vec<u8>>, ProjectError> {
    let doc: BundleDoc = serde_json::from_str(text)?;
    if doc.format != BUNDLE_FORMAT {
        return Err(ProjectError::WrongFormat("bundle"));
    }
    if doc.version > BUNDLE_VERSION {
        return Err(ProjectError::TooNew { what: "bundle", found: doc.version, supported: BUNDLE_VERSION });
    }
    let mut files = BTreeMap::new();
    for (path, entry) in doc.files {
        if !is_safe_path(&path) {
            return Err(ProjectError::UnsafePath(path));
        }
        let bytes = match entry.encoding.as_str() {
            "text" => entry.data.into_bytes(),
            "base64" => base64_decode(&entry.data).ok_or_else(|| ProjectError::CorruptEntry(path.clone()))?,
            _ => return Err(ProjectError::CorruptEntry(path)),
        };
        files.insert(path, bytes);
    }
    Ok(files)
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = u32::from(chunk[0]) << 16
            | u32::from(*chunk.get(1).unwrap_or(&0)) << 8
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(BASE64[(n >> 18) as usize & 63] as char);
        out.push(BASE64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { BASE64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { BASE64[n as usize & 63] as char } else { '=' });
    }
    out
}

pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let digits: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace() && *b != b'=').collect();
    let mut out = Vec::with_capacity(digits.len() * 3 / 4);
    for chunk in digits.chunks(4) {
        if chunk.len() == 1 {
            return None;
        }
        let mut n = 0u32;
        for &d in chunk {
            n = n << 6 | BASE64.iter().position(|c| *c == d)? as u32;
        }
        n <<= 6 * (4 - chunk.len());
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_round_trips_and_tolerates_missing_fields() {
        let mut manifest = ProjectManifest::new("sky-run", "Sky Run", 1_700_000_000);
        manifest.tags = vec!["parkour".into()];
        assert_eq!(ProjectManifest::parse(&manifest.to_json()).unwrap(), manifest);

        let minimal = r#"{"format":"araria.project","version":1,"id":"x","name":"X"}"#;
        let parsed = ProjectManifest::parse(minimal).unwrap();
        assert_eq!(parsed.scene, DEFAULT_SCENE);
        assert_eq!(parsed.max_players, 8);

        assert!(matches!(ProjectManifest::parse(r#"{"format":"other","version":1,"id":"x","name":"X"}"#), Err(ProjectError::WrongFormat(_))));
        assert!(matches!(
            ProjectManifest::parse(r#"{"format":"araria.project","version":7,"id":"x","name":"X"}"#),
            Err(ProjectError::TooNew { .. })
        ));
    }

    #[test]
    fn bundles_round_trip_text_and_binary() {
        let mut files = BTreeMap::new();
        files.insert("project.json".to_string(), b"{\"name\":\"x\"}".to_vec());
        files.insert("scripts/main.lua".to_string(), "print('héllo')".as_bytes().to_vec());
        files.insert("assets/blob.bin".to_string(), (0u8..=255).collect());
        files.insert("assets/empty.bin".to_string(), Vec::new());
        let packed = pack_bundle(&files);
        assert_eq!(unpack_bundle(&packed).unwrap(), files);
    }

    #[test]
    fn bundles_cannot_escape_the_project() {
        for path in ["../evil", "/etc/passwd", "a/../../b", "c:\\x", "a//b", ""] {
            assert!(!is_safe_path(path), "{path}");
            let doc = format!(
                r#"{{"format":"araria.bundle","version":1,"files":{{{}:{{"encoding":"text","data":"x"}}}}}}"#,
                serde_json::to_string(path).unwrap()
            );
            assert!(matches!(unpack_bundle(&doc), Err(ProjectError::UnsafePath(_))), "{path}");
        }
        assert!(is_safe_path("scripts/main.lua"));
    }

    #[test]
    fn base64_matches_reference_vectors() {
        for (plain, encoded) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foobar", "Zm9vYmFy")] {
            assert_eq!(base64_encode(plain.as_bytes()), encoded);
            assert_eq!(base64_decode(encoded).unwrap(), plain.as_bytes());
        }
        assert!(base64_decode("!!!!").is_none());
    }

    #[test]
    fn slugs() {
        assert_eq!(slugify("Sky Run: The Sequel!"), "sky-run-the-sequel");
        assert_eq!(slugify("  "), "project");
    }
}
