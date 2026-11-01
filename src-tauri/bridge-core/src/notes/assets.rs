//! Page assets (images). Uploaded in parts, since a request frame is capped
//! below most photos; stored once per space by content hash in `notes.db`;
//! mirrored as `<root>/assets/<hash>.<ext>` beside the page files, so the
//! `![](assets/…)` a page holds shows in any Markdown editor.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine;
use bridge_protocol::messages::{NoteAsset, NoteAssetData, NoteAssetPart};
use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};

use super::ids::new_id;
use super::service::NotesService;
use crate::BridgeError;

pub const MAX_BYTES: usize = 20 * 1024 * 1024;
pub const DIR: &str = "assets";
const UPLOAD_TTL: Duration = Duration::from_secs(600);
const TYPES: [(&str, &str); 5] = [("image/png", "png"), ("image/jpeg", "jpg"), ("image/gif", "gif"), ("image/webp", "webp"), ("image/svg+xml", "svg")];

/// Uploads still arriving, by id. In memory: an upload a restart interrupts
/// is simply sent again.
#[derive(Default)]
pub struct Uploads(Mutex<HashMap<String, Upload>>);

struct Upload {
    bytes: Vec<u8>,
    touched: Instant,
}

fn invalid(message: impl Into<String>) -> BridgeError {
    BridgeError::Invalid(message.into())
}

/// `assets/<16 hex>.<ext>`, split into hash and extension.
fn parse_path(path: &str) -> Option<(&str, &str)> {
    let (hash, ext) = path.strip_prefix("assets/")?.split_once('.')?;
    let hex = hash.len() == 16 && hash.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    (hex && TYPES.iter().any(|(_, known)| *known == ext)).then_some((hash, ext))
}

impl NotesService {
    pub fn put_asset_part(&self, upload_id: Option<&str>, data: &str) -> Result<NoteAssetPart, BridgeError> {
        let part = base64::engine::general_purpose::STANDARD.decode(data.trim()).map_err(|_| invalid("an asset part must be base64"))?;
        let mut uploads = self.uploads().0.lock().unwrap();
        uploads.retain(|_, upload| upload.touched.elapsed() < UPLOAD_TTL);
        let id = match upload_id {
            Some(id) if uploads.contains_key(id) => id.to_owned(),
            Some(_) => return Err(invalid("that upload expired or never started; send it again")),
            None => {
                let id = new_id();
                uploads.insert(id.clone(), Upload { bytes: Vec::new(), touched: Instant::now() });
                id
            }
        };
        let upload = uploads.get_mut(&id).expect("present");
        if upload.bytes.len() + part.len() > MAX_BYTES {
            uploads.remove(&id);
            return Err(invalid("an image can be at most 20 MB"));
        }
        upload.bytes.extend_from_slice(&part);
        upload.touched = Instant::now();
        Ok(NoteAssetPart { upload_id: id, size: upload.bytes.len() as u64 })
    }

    pub fn finish_asset(&self, upload_id: &str, mime: &str) -> Result<NoteAsset, BridgeError> {
        let ext = TYPES.iter().find(|(known, _)| *known == mime).map(|(_, ext)| *ext).ok_or_else(|| invalid(format!("{mime} is not an image Bridge stores (PNG, JPEG, GIF, WebP or SVG)")))?;
        let bytes = self.uploads().0.lock().unwrap().remove(upload_id).map(|upload| upload.bytes).ok_or_else(|| invalid("that upload expired or never started; send it again"))?;
        if bytes.is_empty() {
            return Err(invalid("the image is empty"));
        }
        let hash = format!("{:x}", Sha256::digest(&bytes))[..16].to_owned();
        let size = bytes.len() as u64;
        self.with_db(|db| {
            db.execute(
                "INSERT OR IGNORE INTO assets(space_id, hash, mime, ext, size, bytes, created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![self.space(), hash, mime, ext, size as i64, bytes, Utc::now().to_rfc3339()],
            )?;
            Ok(())
        })?;
        let path = format!("{DIR}/{hash}.{ext}");
        write_file(&self.root(), &path, &bytes);
        Ok(NoteAsset { path, mime: mime.to_owned(), size })
    }

    pub fn read_asset(&self, path: &str) -> Result<NoteAssetData, BridgeError> {
        let (mime, bytes) = self.asset_bytes(path)?.ok_or_else(|| invalid(format!("no asset at {path}")))?;
        Ok(NoteAssetData { path: path.to_owned(), mime, data: base64::engine::general_purpose::STANDARD.encode(bytes) })
    }

    fn asset_bytes(&self, path: &str) -> Result<Option<(String, Vec<u8>)>, BridgeError> {
        let (hash, _) = parse_path(path).ok_or_else(|| invalid(format!("{path} is not an asset path")))?;
        self.with_db(|db| {
            Ok(db
                .query_row("SELECT mime, bytes FROM assets WHERE space_id=?1 AND hash=?2", params![self.space(), hash], |row| Ok((row.get(0)?, row.get(1)?)))
                .optional()?)
        })
    }
}

fn write_file(root: &Path, path: &str, bytes: &[u8]) {
    let target = root.join(path);
    if target.exists() {
        return;
    }
    let written = target.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| {
        let temp = target.with_extension(format!("tmp-{}", uuid::Uuid::new_v4().simple()));
        std::fs::write(&temp, bytes)?;
        std::fs::rename(&temp, &target)
    });
    if let Err(error) = written {
        eprintln!("[notes] could not write {}: {error}", target.display());
    }
}

/// Write any asset `body` links to that the notes root lacks (a new root, a
/// deleted file). Best effort: the database keeps every asset either way.
pub(crate) fn mirror_referenced(notes: &NotesService, body: &str, root: &Path) {
    for (index, _) in body.match_indices("assets/") {
        let candidate: String = body[index..].chars().take_while(|char| char.is_ascii_alphanumeric() || matches!(char, '/' | '.')).collect();
        let Some((hash, ext)) = parse_path(candidate.trim_end_matches('.')) else { continue };
        let path = format!("{DIR}/{hash}.{ext}");
        if root.join(&path).exists() {
            continue;
        }
        if let Ok(Some((_, bytes))) = notes.asset_bytes(&path) {
            write_file(root, &path, &bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::service::tests::service;

    const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

    #[test]
    fn assets_round_trip_by_hash_and_dedupe() {
        let (_dir, notes) = service();
        let bytes: Vec<u8> = (0..1500u32).map(|n| (n % 251) as u8).collect();
        let first = notes.put_asset_part(None, &B64.encode(&bytes[..1000])).unwrap();
        let second = notes.put_asset_part(Some(&first.upload_id), &B64.encode(&bytes[1000..])).unwrap();
        assert_eq!(second.size, 1500);
        let asset = notes.finish_asset(&first.upload_id, "image/png").unwrap();
        assert!(asset.path.starts_with("assets/") && asset.path.ends_with(".png"), "{}", asset.path);
        assert_eq!(asset.size, 1500);
        let read = notes.read_asset(&asset.path).unwrap();
        assert_eq!(B64.decode(read.data).unwrap(), bytes);
        assert_eq!(read.mime, "image/png");

        let again = notes.put_asset_part(None, &B64.encode(&bytes)).unwrap();
        assert_eq!(notes.finish_asset(&again.upload_id, "image/png").unwrap().path, asset.path);
        let rows: i64 = notes.with_db(|db| Ok(db.query_row("SELECT COUNT(*) FROM assets", [], |row| row.get(0))?)).unwrap();
        assert_eq!(rows, 1);
        // A finished upload is gone.
        assert!(notes.put_asset_part(Some(&first.upload_id), "AA==").is_err());
    }

    #[test]
    fn asset_uploads_refuse_unknown_types_oversize_and_bad_paths() {
        let (_dir, notes) = service();
        assert!(notes.put_asset_part(None, "not base64!").is_err());
        assert!(notes.put_asset_part(Some("nope"), "AA==").is_err());
        let part = notes.put_asset_part(None, "AA==").unwrap();
        assert!(notes.finish_asset(&part.upload_id, "application/pdf").is_err());

        let chunk = B64.encode(vec![0u8; 4 * 1024 * 1024]);
        let big = notes.put_asset_part(None, &chunk).unwrap();
        for _ in 0..4 {
            notes.put_asset_part(Some(&big.upload_id), &chunk).unwrap();
        }
        assert!(notes.put_asset_part(Some(&big.upload_id), &chunk).is_err(), "over 20 MB");
        assert!(notes.put_asset_part(Some(&big.upload_id), "AA==").is_err(), "the oversize upload is dropped");

        for path in ["assets/../notes.db", "assets/0123456789abcdef.exe", "elsewhere/0123456789abcdef.png", "assets/0123456789abcdef.png"] {
            assert!(notes.read_asset(path).is_err(), "{path}");
        }
    }

    #[test]
    fn finishing_an_asset_writes_the_file_and_projection_restores_it() {
        let (_dir, notes) = service();
        let part = notes.put_asset_part(None, &B64.encode(b"GIF89a-not-really")).unwrap();
        let asset = notes.finish_asset(&part.upload_id, "image/gif").unwrap();
        let file = notes.root().join(&asset.path);
        assert_eq!(std::fs::read(&file).unwrap(), b"GIF89a-not-really");

        std::fs::remove_file(&file).unwrap();
        let page = notes.create(crate::notes::service::CreatePage { content: Some(format!("See ![shot]({}).\n", asset.path)), ..Default::default() }, crate::notes::service::PERSON).unwrap();
        assert!(notes.read(&page.id).unwrap().note.path.is_some());
        assert_eq!(std::fs::read(&file).unwrap(), b"GIF89a-not-really");
    }
}
