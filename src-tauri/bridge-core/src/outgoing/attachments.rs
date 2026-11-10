//! Images a draft carries.
//!
//! An agent attaches local files by path, only when the person asked for an
//! image. A path is accepted only if the file's first bytes say it is a PNG,
//! JPEG, GIF or WebP: an extension is a claim, the bytes are the fact. That
//! keeps a draft from becoming a way to lift `~/.ssh/id_rsa` into a message.
//! Files are read again, and checked again, when the card shows them.

use std::io::Read;
use std::path::Path;

use base64::Engine;
use bridge_protocol::messages::{OutgoingAttachment, MAX_OUTGOING_ATTACHMENTS, MAX_OUTGOING_ATTACHMENT_BYTES};

/// The image type a file's first bytes declare.
pub fn sniff(head: &[u8]) -> Option<&'static str> {
    if head.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some("image/png")
    } else if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if head.len() >= 12 && &head[0..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// Read a file through one handle, at most the size limit plus one byte, so
/// the bytes checked are the bytes returned and a growing file cannot slip
/// past the limit.
fn bounded_read(path: &str) -> Result<Vec<u8>, String> {
    let file = Path::new(path);
    if !file.is_absolute() {
        return Err(format!("{path}: attachments must be absolute paths"));
    }
    let handle = std::fs::File::open(file).map_err(|_| format!("{path}: no such file"))?;
    let meta = handle.metadata().map_err(|_| format!("{path}: cannot be read"))?;
    if !meta.is_file() {
        return Err(format!("{path}: not a file"));
    }
    let mut bytes = Vec::new();
    handle
        .take(MAX_OUTGOING_ATTACHMENT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| format!("{path}: cannot be read"))?;
    if bytes.len() as u64 > MAX_OUTGOING_ATTACHMENT_BYTES {
        return Err(format!("{path}: larger than {} MB", MAX_OUTGOING_ATTACHMENT_BYTES / (1024 * 1024)));
    }
    Ok(bytes)
}

fn digest(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

fn one(path: &str) -> Result<OutgoingAttachment, String> {
    let bytes = bounded_read(path)?;
    let mime = sniff(&bytes).ok_or_else(|| format!("{path}: not a PNG, JPEG, GIF or WebP image"))?;
    Ok(OutgoingAttachment {
        path: path.to_owned(),
        name: Path::new(path).file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default(),
        mime: mime.to_owned(),
        bytes: bytes.len() as u64,
        sha256: digest(&bytes),
    })
}

/// Check every path; all of them or none.
pub fn validate(paths: &[String]) -> Result<Vec<OutgoingAttachment>, String> {
    if paths.len() > MAX_OUTGOING_ATTACHMENTS {
        return Err(format!("at most {MAX_OUTGOING_ATTACHMENTS} images per message"));
    }
    paths.iter().map(|path| one(path.trim())).collect()
}

/// The image as base64, if it is still exactly the image that was attached.
pub fn read(attachment: &OutgoingAttachment) -> Result<(String, String), String> {
    let bytes = bounded_read(&attachment.path)?;
    let changed = || format!("{}: the file changed since it was attached", attachment.path);
    let mime = sniff(&bytes).ok_or_else(changed)?;
    if mime != attachment.mime || digest(&bytes) != attachment.sha256 {
        return Err(changed());
    }
    Ok((mime.to_owned(), base64::engine::general_purpose::STANDARD.encode(bytes)))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];

    pub(crate) fn write(dir: &Path, name: &str, bytes: &[u8]) -> String {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn images_are_recognised_by_their_bytes_not_their_names() {
        let dir = tempfile::tempdir().unwrap();
        let png = write(dir.path(), "shot.png", PNG);
        let jpeg_named_txt = write(dir.path(), "photo.txt", &[0xFF, 0xD8, 0xFF, 0xE0, 0, 0]);
        let key_named_png = write(dir.path(), "id_rsa.png", b"-----BEGIN OPENSSH PRIVATE KEY-----");
        let ok = validate(&[png.clone(), jpeg_named_txt]).unwrap();
        assert_eq!((ok[0].mime.as_str(), ok[0].name.as_str(), ok[0].bytes), ("image/png", "shot.png", 12));
        assert_eq!(ok[1].mime, "image/jpeg");
        assert!(validate(&[png, key_named_png]).unwrap_err().contains("not a PNG"));
        assert_eq!(sniff(b"GIF89a.."), Some("image/gif"));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
    }

    #[test]
    fn relative_missing_and_too_many_paths_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let png = write(dir.path(), "a.png", PNG);
        assert!(validate(&["a.png".into()]).unwrap_err().contains("absolute"));
        assert!(validate(&[format!("{png}.gone")]).unwrap_err().contains("no such file"));
        assert!(validate(&[dir.path().to_string_lossy().into_owned()]).unwrap_err().contains("not a file"));
        assert!(validate(&vec![png; MAX_OUTGOING_ATTACHMENTS + 1]).unwrap_err().contains("at most"));
    }

    #[test]
    fn swapping_in_a_different_image_of_the_same_type_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "a.png", PNG);
        let attached = validate(&[path.clone()]).unwrap().remove(0);
        let mut other = PNG.to_vec();
        *other.last_mut().unwrap() = 99;
        std::fs::write(&path, &other).unwrap();
        let error = read(&attached).unwrap_err();
        assert!(error.contains("changed since it was attached"), "{error}");
    }

    #[test]
    fn reading_rechecks_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let png = write(dir.path(), "a.png", PNG);
        let attached = validate(&[png.clone()]).unwrap().remove(0);
        let (mime, data) = read(&attached).unwrap();
        assert_eq!((mime.as_str(), data.as_str()), ("image/png", "iVBORw0KGgoAAAAN"));
        std::fs::write(&png, b"not an image any more").unwrap();
        assert!(read(&attached).is_err());
    }
}
