//! Omarchy theme sync: the shell reads the active Omarchy theme and hands it
//! to the webview, which maps it onto Bridge's tokens.
//!
//! Omarchy keeps the active theme at `~/.local/state/omarchy/current/theme/`
//! and replaces that directory on every switch, so the shell polls a cheap
//! stamp (the theme name plus `colors.toml`'s size and mtime) rather than
//! holding an inotify watch on a directory that is about to be swapped out.
//!
//! This is shell-to-webview transport, like `agent-event-batch`: desktop-local
//! state with no daemon involvement, so it is not a protocol method. The
//! palette is sent raw; `src/lib/omarchyTheme.ts` owns the token table.
//!
//! Parsing and detection are platform-neutral so they are tested everywhere.
//! Only `install` does anything, and only on Linux.

use serde::Serialize;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Shell to webview: the current theme, sent on request and on every switch.
pub const THEME_EVENT: &str = "bridge-omarchy-theme";
/// Webview to shell: "send me the current theme" (the webview boots after us).
pub const THEME_REQUEST_EVENT: &str = "bridge-omarchy-theme-request";
/// Short enough that a switch retints within a second.
pub const POLL_INTERVAL: Duration = Duration::from_millis(400);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemePayload {
    pub available: bool,
    pub theme_name: Option<String>,
    pub mode: &'static str,
    pub colors: BTreeMap<String, String>,
    pub font: Option<String>,
}

/// `~/.local/state/omarchy/current`, the parent of the swapped theme dir.
pub fn current_dir(home: &Path) -> PathBuf {
    home.join(".local/state/omarchy/current")
}

/// On when the theme state exists, or when `omarchy` is installed and we are
/// inside Hyprland. The second rule covers a fresh install before its first
/// theme switch has written any state.
pub fn detect(home: &Path, path_var: Option<&OsStr>, in_hyprland: bool) -> bool {
    if current_dir(home).join("theme").is_dir() {
        return true;
    }
    in_hyprland
        && path_var
            .map(|paths| std::env::split_paths(paths).any(|dir| dir.join("omarchy").is_file()))
            .unwrap_or(false)
}

fn is_hex_color(value: &str) -> bool {
    let Some(digits) = value.strip_prefix('#') else { return false };
    matches!(digits.len(), 3 | 6 | 8) && digits.chars().all(|c| c.is_ascii_hexdigit())
}

/// The hex colours a `colors.toml` names, and its mode, resolved with the same
/// precedence as Omarchy's `omarchy-theme-color`: the `mode` key, the legacy
/// `theme_type` key, a `light.mode` file beside it, background luminance, dark.
pub fn parse_colors(text: &str, light_marker: bool) -> Option<(&'static str, BTreeMap<String, String>)> {
    let table: toml::Table = toml::from_str(text).ok()?;
    let string = |key: &str| table.get(key).and_then(toml::Value::as_str);
    let colors: BTreeMap<String, String> = table
        .iter()
        .filter_map(|(key, value)| {
            let value = value.as_str()?;
            is_hex_color(value).then(|| (key.clone(), value.to_ascii_lowercase()))
        })
        .collect();
    let declared = string("mode").or_else(|| string("theme_type"));
    let mode = match declared {
        Some("light") => "light",
        Some(_) => "dark",
        None if light_marker => "light",
        None => match colors.get("background").map(String::as_str) {
            Some(bg) if bg.len() == 7 => {
                let channel = |i: usize| u32::from_str_radix(&bg[i..i + 2], 16).unwrap_or(0);
                if channel(1) + channel(3) + channel(5) > 382 { "light" } else { "dark" }
            }
            _ => "dark",
        },
    };
    Some((mode, colors))
}

/// `fc-match` prints an alias list (`JetBrainsMono Nerd Font,JetBrainsMono NF`);
/// Omarchy's `omarchy-font-current` takes the first entry, and so do we.
pub fn first_font_alias(output: &str) -> Option<String> {
    let first = output.lines().next()?.split(',').next()?.trim();
    (!first.is_empty()).then(|| first.to_string())
}

/// The family Omarchy's `font set` configured. fontconfig is its source of
/// truth; `shell.toml`'s `[font]` only holds sizes.
pub fn system_font() -> Option<String> {
    let output = std::process::Command::new("fc-match")
        .args(["monospace", "-f", "%{family}\n"])
        .output()
        .ok()?;
    output.status.success().then(|| first_font_alias(&String::from_utf8_lossy(&output.stdout)))?
}

/// The active theme, or `None` when it is missing or unreadable (the previous
/// palette stays on screen rather than flashing back to Graphite).
pub fn read_theme(current: &Path, font: Option<String>) -> Option<ThemePayload> {
    let theme = current.join("theme");
    let text = std::fs::read_to_string(theme.join("colors.toml")).ok()?;
    let (mode, colors) = parse_colors(&text, theme.join("light.mode").is_file())?;
    let theme_name = std::fs::read_to_string(current.join("theme.name"))
        .ok()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty());
    Some(ThemePayload { available: true, theme_name, mode, colors, font })
}

/// What a switch changes. Equal stamps mean nothing to re-send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamp {
    name: Option<String>,
    colors_len: u64,
    colors_mtime: Option<SystemTime>,
}

pub fn stamp(current: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(current.join("theme/colors.toml")).ok()?;
    Some(Stamp {
        name: std::fs::read_to_string(current.join("theme.name")).ok(),
        colors_len: meta.len(),
        colors_mtime: meta.modified().ok(),
    })
}

/// Answer the webview's request and follow theme switches. A no-op unless
/// this is Linux and Omarchy is detected.
pub fn install<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if !cfg!(target_os = "linux") {
        return;
    }
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return };
    let in_hyprland = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some();
    if detect(&home, std::env::var_os("PATH").as_deref(), in_hyprland) {
        follow(app, current_dir(&home));
    }
}

/// Compiled on every platform so it type-checks on macOS CI too; only
/// `install` decides whether it runs.
fn follow<R: tauri::Runtime>(app: &tauri::AppHandle<R>, current: PathBuf) {
    use tauri::{Emitter, Listener};

    let request_app = app.clone();
    let request_dir = current.clone();
    let _ = app.listen(THEME_REQUEST_EVENT, move |_| {
        if let Some(payload) = read_theme(&request_dir, system_font()) {
            let _ = request_app.emit(THEME_EVENT, payload);
        }
    });

    let poll_app = app.clone();
    let _ = std::thread::Builder::new().name("omarchy-theme".into()).spawn(move || {
        let mut last = stamp(&current);
        loop {
            std::thread::sleep(POLL_INTERVAL);
            let next = stamp(&current);
            if next.is_none() || next == last {
                continue;
            }
            last = next;
            if let Some(payload) = read_theme(&current, system_font()) {
                let _ = poll_app.emit(THEME_EVENT, payload);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../testing/fixtures/omarchy/themes")
    }

    #[test]
    fn parses_every_shipped_theme() {
        let mut count = 0;
        for entry in std::fs::read_dir(fixtures()).unwrap() {
            let path = entry.unwrap().path();
            let text = std::fs::read_to_string(&path).unwrap();
            let name = path.file_stem().unwrap().to_string_lossy().to_string();
            let (mode, colors) = parse_colors(&text, false).unwrap_or_else(|| panic!("{name} parses"));
            for key in ["background", "foreground", "accent"] {
                assert!(colors.contains_key(key), "{name} has {key}");
            }
            let declared = if text.contains("mode = \"light\"") { "light" } else { "dark" };
            assert_eq!(mode, declared, "{name}");
            count += 1;
        }
        assert_eq!(count, 23);
    }

    #[test]
    fn drops_non_hex_values_and_comments() {
        let (_, colors) = parse_colors(
            "# a comment\nbackground = \"#1A1B26\" # inline\nfont = \"JetBrains\"\nborder = \"rgb(1,2,3)\"\nshort = \"#fff\"\nalpha = \"#11223344\"\nbad = \"#12345\"\nsize = 9\n",
            false,
        )
        .unwrap();
        assert_eq!(
            colors.into_iter().collect::<Vec<_>>(),
            vec![
                ("alpha".to_string(), "#11223344".to_string()),
                ("background".to_string(), "#1a1b26".to_string()),
                ("short".to_string(), "#fff".to_string()),
            ]
        );
        assert!(parse_colors("not = [valid", false).is_none());
    }

    #[test]
    fn mode_follows_omarchy_precedence() {
        let mode = |text: &str, marker: bool| parse_colors(text, marker).unwrap().0;
        assert_eq!(mode("mode = \"light\"\nbackground = \"#000000\"", false), "light");
        assert_eq!(mode("mode = \"dark\"\nbackground = \"#ffffff\"", true), "dark");
        assert_eq!(mode("theme_type = \"light\"\nbackground = \"#000000\"", false), "light");
        assert_eq!(mode("background = \"#000000\"", true), "light");
        assert_eq!(mode("background = \"#faf4ed\"", false), "light");
        assert_eq!(mode("background = \"#1a1b26\"", false), "dark");
        assert_eq!(mode("background = \"#808080\"", false), "light");
        assert_eq!(mode("background = \"#7f7f7f\"", false), "dark");
        assert_eq!(mode("accent = \"#ffffff\"", false), "dark");
    }

    #[test]
    fn detection_rules() {
        let home = tempfile::tempdir().unwrap();
        let bin = tempfile::tempdir().unwrap();
        let path = std::env::join_paths([bin.path()]).unwrap();
        assert!(!detect(home.path(), Some(&path), true));

        std::fs::write(bin.path().join("omarchy"), "#!/bin/sh\n").unwrap();
        assert!(detect(home.path(), Some(&path), true));
        assert!(!detect(home.path(), Some(&path), false), "omarchy on PATH outside Hyprland is not enough");

        std::fs::create_dir_all(current_dir(home.path()).join("theme")).unwrap();
        assert!(detect(home.path(), None, false), "theme state alone is enough");
    }

    #[test]
    fn font_family_takes_first_fontconfig_alias() {
        assert_eq!(
            first_font_alias("JetBrainsMono Nerd Font,JetBrainsMono NF\n").as_deref(),
            Some("JetBrainsMono Nerd Font")
        );
        assert_eq!(first_font_alias("Iosevka\n").as_deref(), Some("Iosevka"));
        assert_eq!(first_font_alias(""), None);
        assert_eq!(first_font_alias("\n"), None);
    }

    #[test]
    fn reads_the_active_theme_and_its_name() {
        let home = tempfile::tempdir().unwrap();
        let current = current_dir(home.path());
        assert!(read_theme(&current, None).is_none(), "missing theme sends nothing");
        std::fs::create_dir_all(current.join("theme")).unwrap();
        std::fs::copy(fixtures().join("rose-pine.toml"), current.join("theme/colors.toml")).unwrap();
        std::fs::write(current.join("theme.name"), "rose-pine\n").unwrap();
        let payload = read_theme(&current, Some("Iosevka".into())).unwrap();
        assert!(payload.available);
        assert_eq!(payload.theme_name.as_deref(), Some("rose-pine"));
        assert_eq!(payload.mode, "light");
        assert_eq!(payload.colors.get("background").map(String::as_str), Some("#faf4ed"));
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["themeName"], "rose-pine");
        assert_eq!(json["font"], "Iosevka");
    }

    #[test]
    fn stamp_changes_when_theme_switches() {
        let home = tempfile::tempdir().unwrap();
        let current = current_dir(home.path());
        assert!(stamp(&current).is_none());
        std::fs::create_dir_all(current.join("theme")).unwrap();
        std::fs::write(current.join("theme/colors.toml"), "background = \"#000000\"\n").unwrap();
        std::fs::write(current.join("theme.name"), "matte-black").unwrap();
        let first = stamp(&current).unwrap();
        assert_eq!(stamp(&current).unwrap(), first, "an unchanged dir keeps its stamp");

        std::fs::write(current.join("theme.name"), "gruvbox").unwrap();
        let renamed = stamp(&current).unwrap();
        assert_ne!(renamed, first);

        // Omarchy swaps the whole dir; a different palette changes the size.
        std::fs::remove_dir_all(current.join("theme")).unwrap();
        std::fs::create_dir_all(current.join("theme")).unwrap();
        std::fs::write(current.join("theme/colors.toml"), "background = \"#282828\"\naccent = \"#fabd2f\"\n").unwrap();
        assert_ne!(stamp(&current).unwrap(), renamed);
    }
}
