//! Explicit, approved-domain cookie import from a user's Chrome or Brave profile.
//! The caller must obtain approval before invoking this short-lived operation.

use crate::browser_clone::{CookieSpec, SameSite};
use rusqlite::{params_from_iter, Connection, OpenFlags};
use security_framework::passwords::{generic_password, PasswordOptions};
use sha2::{Digest, Sha256};
use std::{env, ffi::c_void, path::PathBuf};
use zeroize::{Zeroize, Zeroizing};

#[derive(Debug, Clone, Copy)]
pub enum Browser {
    Chrome,
    Brave,
}

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("invalid approved domain")]
    Domain,
    #[error("browser cookie store is unavailable")]
    Store,
    #[error("browser Safe Storage permission was denied or unavailable")]
    Keychain,
    #[error("browser cookie could not be decrypted")]
    Decrypt,
}

impl Browser {
    fn base(self) -> &'static str {
        match self {
            Self::Chrome => "Google/Chrome",
            Self::Brave => "BraveSoftware/Brave-Browser",
        }
    }
    fn safe_storage(self) -> &'static str {
        match self {
            Self::Chrome => "Chrome Safe Storage",
            Self::Brave => "Brave Safe Storage",
        }
    }
}

fn approved_domain(domain: &str) -> Result<String, ImportError> {
    let domain = domain.trim().to_ascii_lowercase();
    if domain.len() > 253
        || !domain.contains('.')
        || domain.starts_with('.')
        || domain.ends_with('.')
        || domain.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
    {
        return Err(ImportError::Domain);
    }
    Ok(domain)
}

fn matches_domain(host_key: &str, approved: &str) -> bool {
    let host = host_key.trim_start_matches('.').to_ascii_lowercase();
    host == approved || host.ends_with(&format!(".{approved}"))
}

fn approved_domains(domains: &[String]) -> Result<Vec<String>, ImportError> {
    if domains.is_empty() {
        return Err(ImportError::Domain);
    }
    let mut domains = domains
        .iter()
        .map(|domain| approved_domain(domain))
        .collect::<Result<Vec<_>, _>>()?;
    domains.sort();
    domains.dedup();
    Ok(domains)
}

/// Reads only cookie rows for the explicitly approved domains and their
/// subdomains. Parent domains require their own approval. `profile` is a Chrome
/// profile directory name (for example, `Default`), never a path.
pub fn import_cookies(
    browser: Browser,
    profile: &str,
    domains: &[String],
) -> Result<Vec<CookieSpec>, ImportError> {
    // Validate the entire scope before opening the store or touching Keychain.
    let domains = approved_domains(domains)?;
    if profile.is_empty()
        || profile == "."
        || profile == ".."
        || !profile
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b' ' || b == b'-' || b == b'_')
    {
        return Err(ImportError::Store);
    }
    let home = env::var_os("HOME").ok_or(ImportError::Store)?;
    let db_path = PathBuf::from(home)
        .join("Library/Application Support")
        .join(browser.base())
        .join(profile)
        .join("Cookies");
    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| ImportError::Store)?;
    let mut options = PasswordOptions::new_generic_password(browser.safe_storage(), "");
    // Chrome's item may use a browser-specific account. Query by service only.
    use core_foundation::{base::TCFType, string::CFString};
    use security_framework_sys::item::kSecAttrAccount;
    #[allow(deprecated)]
    unsafe {
        options
            .query
            .retain(|(key, _)| *key != CFString::wrap_under_get_rule(kSecAttrAccount));
    }
    let password = Zeroizing::new(
        generic_password(options)
            .map_err(|_| ImportError::Keychain)?
            .to_vec(),
    );
    let key = Zeroizing::new(pbkdf2_sha1(&password, b"saltysalt", 1003, 16)?);
    read_cookies(&conn, &domains, &key)
}

fn read_cookies(
    conn: &Connection,
    domains: &[String],
    key: &[u8],
) -> Result<Vec<CookieSpec>, ImportError> {
    let domains = approved_domains(domains)?;
    // One query selects overlapping scopes only once. Values outside the
    // approved scope never reach row decoding or decryption.
    let conditions =
        vec!["(host_key = ? OR host_key = ? OR host_key LIKE ? ESCAPE '\\')"; domains.len()]
            .join(" OR ");
    let sql = format!("SELECT host_key, name, value, encrypted_value, path, is_secure, is_httponly, samesite FROM cookies WHERE {conditions}");
    let mut params = Vec::new();
    for domain in &domains {
        params.extend([domain.clone(), format!(".{domain}"), format!("%.{domain}")]);
    }
    let mut statement = conn.prepare(&sql).map_err(|_| ImportError::Store)?;
    let rows = statement
        .query_map(params_from_iter(params), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Vec<u8>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, bool>(5)?,
                row.get::<_, bool>(6)?,
                row.get::<_, i64>(7)?,
            ))
        })
        .map_err(|_| ImportError::Store)?;
    let mut cookies = Vec::new();
    for row in rows {
        let (host, name, plain, encrypted, path, secure, http_only, same_site) =
            row.map_err(|_| ImportError::Store)?;
        if !domains.iter().any(|domain| matches_domain(&host, domain)) {
            continue;
        }
        let value = if encrypted.is_empty() {
            plain
        } else {
            decrypt_cookie(&encrypted, key, &host)?
        };
        cookies.push(CookieSpec {
            name,
            value,
            domain: host,
            path,
            secure,
            http_only,
            same_site: match same_site {
                0 => Some(SameSite::None),
                1 => Some(SameSite::Lax),
                2 => Some(SameSite::Strict),
                _ => None,
            },
        });
    }
    Ok(cookies)
}

extern "C" {
    fn CCKeyDerivationPBKDF(
        algorithm: u32,
        password: *const u8,
        password_len: usize,
        salt: *const u8,
        salt_len: usize,
        prf: u32,
        rounds: u32,
        derived: *mut u8,
        derived_len: usize,
    ) -> i32;
    fn CCCrypt(
        operation: u32,
        algorithm: u32,
        options: u32,
        key: *const u8,
        key_len: usize,
        iv: *const u8,
        input: *const u8,
        input_len: usize,
        output: *mut c_void,
        output_available: usize,
        output_len: *mut usize,
    ) -> i32;
}

fn pbkdf2_sha1(
    password: &[u8],
    salt: &[u8],
    rounds: u32,
    len: usize,
) -> Result<Vec<u8>, ImportError> {
    let mut key = vec![0; len];
    // CommonCrypto: kCCPBKDF2=2, kCCPRFHmacAlgSHA1=1.
    let status = unsafe {
        CCKeyDerivationPBKDF(
            2,
            password.as_ptr(),
            password.len(),
            salt.as_ptr(),
            salt.len(),
            1,
            rounds,
            key.as_mut_ptr(),
            key.len(),
        )
    };
    if status != 0 {
        key.zeroize();
        return Err(ImportError::Decrypt);
    }
    Ok(key)
}

fn decrypt_cookie(encrypted: &[u8], key: &[u8], host: &str) -> Result<String, ImportError> {
    let cipher = encrypted.strip_prefix(b"v10").ok_or(ImportError::Decrypt)?;
    let mut output = vec![0u8; cipher.len() + 16];
    let mut used = 0;
    let iv = [b' '; 16];
    // CommonCrypto: decrypt=1, AES=0, PKCS7 padding=1.
    let status = unsafe {
        CCCrypt(
            1,
            0,
            1,
            key.as_ptr(),
            key.len(),
            iv.as_ptr(),
            cipher.as_ptr(),
            cipher.len(),
            output.as_mut_ptr().cast(),
            output.len(),
            &mut used,
        )
    };
    if status != 0 {
        output.zeroize();
        return Err(ImportError::Decrypt);
    }
    output.truncate(used);
    // Chromium's newer v10 payloads prefix the cleartext with SHA-256(host_key).
    if output.starts_with(Sha256::digest(host.as_bytes()).as_slice()) {
        output.drain(..32);
    }
    let result = String::from_utf8(output.clone()).map_err(|_| ImportError::Decrypt);
    output.zeroize();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_filter_excludes_other_sites_and_suffix_tricks() {
        assert!(matches_domain(".login.example.com", "example.com"));
        assert!(!matches_domain("example.com.evil.test", "example.com"));
        assert!(!matches_domain("other.com", "example.com"));
    }

    #[test]
    fn approved_domain_rejects_garbage_and_paths() {
        assert!(approved_domain("example.com").is_ok());
        assert!(approved_domain("login.example.co.uk").is_ok());
        for bad in [
            "",
            "nodot",
            ".leading.com",
            "trailing.com.",
            "a/b.com",
            "-bad.com",
        ] {
            assert!(approved_domain(bad).is_err(), "accepted {bad:?}");
        }
    }

    fn cookie_store() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE cookies (host_key TEXT, name TEXT, value TEXT, encrypted_value BLOB, path TEXT, is_secure INTEGER, is_httponly INTEGER, samesite INTEGER)").unwrap();
        conn
    }

    fn insert_cookie(conn: &Connection, host: &str, encrypted: &[u8]) {
        conn.execute(
            "INSERT INTO cookies VALUES (?1, 'sid', '', ?2, '/', 1, 1, 1)",
            rusqlite::params![host, encrypted],
        )
        .unwrap();
    }

    #[test]
    fn multi_domain_selection_never_reads_unapproved_or_parent_values() {
        let conn = cookie_store();
        let key = pbkdf2_sha1(b"fixture", b"saltysalt", 1003, 16).unwrap();
        for host in [
            "docs.google.com",
            ".accounts.google.com",
            "login.accounts.google.com",
        ] {
            insert_cookie(&conn, host, &seal_v10(b"synthetic-session", &key));
        }
        for host in [
            ".google.com",
            "drive.google.com",
            "docs.google.com.evil.test",
            "unrelated.test",
        ] {
            // Even decoding these values would fail. They must not be read.
            conn.execute(
                "INSERT INTO cookies VALUES (?1, 'poison', 123, 'not-a-blob', '/', 1, 1, 1)",
                [host],
            )
            .unwrap();
        }
        let cookies = read_cookies(
            &conn,
            &["docs.google.com".into(), "accounts.google.com".into()],
            &key,
        )
        .unwrap();
        let mut hosts: Vec<_> = cookies
            .iter()
            .map(|cookie| cookie.domain.as_str())
            .collect();
        hosts.sort();
        assert_eq!(
            hosts,
            [
                ".accounts.google.com",
                "docs.google.com",
                "login.accounts.google.com"
            ]
        );
        assert!(cookies
            .iter()
            .all(|cookie| cookie.value == "synthetic-session"
                && cookie.secure
                && cookie.http_only
                && cookie.same_site == Some(SameSite::Lax)));
    }

    #[test]
    fn parent_cookies_require_explicit_consent_and_overlap_is_selected_once() {
        let conn = cookie_store();
        let key = pbkdf2_sha1(b"fixture", b"saltysalt", 1003, 16).unwrap();
        for host in [".google.com", "docs.google.com", ".accounts.google.com"] {
            insert_cookie(&conn, host, &seal_v10(b"synthetic-session", &key));
        }
        let narrow = read_cookies(&conn, &["docs.google.com".into()], &key).unwrap();
        assert_eq!(narrow.len(), 1);
        assert_eq!(narrow[0].domain, "docs.google.com");
        let cookies = read_cookies(
            &conn,
            &[
                "google.com".into(),
                "docs.google.com".into(),
                " GOOGLE.COM ".into(),
            ],
            &key,
        )
        .unwrap();
        assert_eq!(cookies.len(), 3);
        assert!(cookies.iter().any(|cookie| cookie.domain == ".google.com"));
    }

    #[test]
    fn invalid_scope_is_refused_before_the_cookie_store_is_opened() {
        for domains in [
            vec![],
            vec!["docs.google.com".into(), "invalid/path.test".into()],
        ] {
            assert!(matches!(
                import_cookies(Browser::Chrome, "Default", &domains),
                Err(ImportError::Domain)
            ));
        }
    }

    /// Encrypt like Chromium's macOS v10 so `decrypt_cookie` can be exercised
    /// end to end — proving the CommonCrypto key derivation and AES both link
    /// and round-trip, without ever touching a real Keychain or real cookie.
    fn seal_v10(plaintext: &[u8], key: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; plaintext.len() + 32];
        let mut used = 0usize;
        let iv = [b' '; 16];
        // kCCEncrypt=0, AES=0, PKCS7 padding=1.
        let status = unsafe {
            CCCrypt(
                0,
                0,
                1,
                key.as_ptr(),
                key.len(),
                iv.as_ptr(),
                plaintext.as_ptr(),
                plaintext.len(),
                out.as_mut_ptr().cast(),
                out.len(),
                &mut used,
            )
        };
        assert_eq!(status, 0, "CCCrypt encrypt failed to link or run");
        out.truncate(used);
        let mut sealed = b"v10".to_vec();
        sealed.extend_from_slice(&out);
        sealed
    }

    #[test]
    fn decrypt_round_trips_a_v10_cookie() {
        let key = pbkdf2_sha1(b"test-passphrase", b"saltysalt", 1003, 16).unwrap();
        let sealed = seal_v10(b"session=abc123; theme=dark", &key);
        let plain = decrypt_cookie(&sealed, &key, "login.example.com").unwrap();
        assert_eq!(plain, "session=abc123; theme=dark");
    }

    #[test]
    fn decrypt_strips_a_host_hash_prefix() {
        let key = pbkdf2_sha1(b"pw", b"saltysalt", 1003, 16).unwrap();
        let host = "login.example.com";
        let mut payload = Sha256::digest(host.as_bytes()).to_vec();
        payload.extend_from_slice(b"realvalue");
        let sealed = seal_v10(&payload, &key);
        assert_eq!(decrypt_cookie(&sealed, &key, host).unwrap(), "realvalue");
    }

    #[test]
    fn decrypt_rejects_a_non_v10_blob() {
        let key = pbkdf2_sha1(b"pw", b"saltysalt", 1003, 16).unwrap();
        assert!(matches!(
            decrypt_cookie(b"v11garbage", &key, "example.com"),
            Err(ImportError::Decrypt)
        ));
    }
}
