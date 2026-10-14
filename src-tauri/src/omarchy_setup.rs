//! `bridge-deck --omarchy setup|remove`: wire Bridge into an Omarchy desktop.
//!
//! Everything lands under `~/.config`, which Omarchy's own rules require, and
//! all of it is undone by `remove`:
//!
//! - `omarchy/hooks/theme-set.d/bridge`: retints a running Bridge on a theme
//!   switch (belt and braces next to the shell's own poll).
//! - `omarchy/extensions/omarchy-menu.jsonc`: a `bridge` submenu, merged into
//!   the user's file between marker comments.
//! - `omarchy/plugins/app.bridge.status/`: the Quickshell bar widget.
//! - with `--hyprland`, `hypr/bridge.lua` plus one `require` line in the
//!   user's `hypr/hyprland.lua`: the quick-prompt window rules and binding.
//!
//! A user file is backed up before it is first modified, and the manifest
//! records every path created. `remove` restores backups byte for byte and
//! deletes what it created, so a setup/remove round trip leaves the user's
//! files exactly as they were. Running setup twice changes nothing.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const PLUGIN_ID: &str = "app.bridge.status";
const MENU_BEGIN: &str = "  // >>> bridge: added by bridge-deck --omarchy setup; undo with bridge-deck --omarchy remove";
const MENU_END: &str = "  // <<< bridge";
const HYPR_REQUIRE: &str = "require(\"hypr.bridge\") -- added by bridge-deck --omarchy setup";
/// `SUPER + SHIFT + CTRL + B` mirrors Omarchy's agent picker on `... + A`.
pub const QUICK_PROMPT_KEYS: &str = "SUPER + SHIFT + CTRL + B";

const PLUGIN_FILES: &[(&str, &str)] = &[
    ("manifest.json", include_str!("../../integrations/omarchy/plugin/manifest.json")),
    ("BarWidget.qml", include_str!("../../integrations/omarchy/plugin/BarWidget.qml")),
    ("README.md", include_str!("../../integrations/omarchy/plugin/README.md")),
];

#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    /// Files and directories setup created, deepest last.
    created: Vec<PathBuf>,
    /// User files setup modified, each with the pristine copy it saved.
    modified: Vec<Modified>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct Modified {
    path: PathBuf,
    backup: PathBuf,
}

/// What a setup or remove did, for the CLI to print.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub written: Vec<PathBuf>,
    pub restored: Vec<PathBuf>,
    pub removed: Vec<PathBuf>,
}

struct Paths {
    config: PathBuf,
    manifest: PathBuf,
    backups: PathBuf,
}

impl Paths {
    fn new(home: &Path) -> Self {
        let config = home.join(".config");
        let bridge = config.join("bridge");
        Self { manifest: bridge.join("omarchy-setup.json"), backups: bridge.join("omarchy-backup"), config }
    }
}

fn io(context: &Path, error: std::io::Error) -> String {
    format!("{}: {error}", context.display())
}

fn load_manifest(paths: &Paths) -> Result<Manifest, String> {
    match std::fs::read(&paths.manifest) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", paths.manifest.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Manifest::default()),
        Err(error) => Err(io(&paths.manifest, error)),
    }
}

fn save_manifest(paths: &Paths, manifest: &Manifest) -> Result<(), String> {
    let parent = paths.manifest.parent().expect("manifest has a parent");
    std::fs::create_dir_all(parent).map_err(|error| io(parent, error))?;
    let json = serde_json::to_vec_pretty(manifest).expect("manifest serializes");
    std::fs::write(&paths.manifest, json).map_err(|error| io(&paths.manifest, error))
}

/// Create `dir` and every missing parent, recording each one created.
fn ensure_dir(dir: &Path, manifest: &mut Manifest) -> Result<(), String> {
    let mut missing = Vec::new();
    let mut cursor = Some(dir);
    while let Some(path) = cursor {
        if path.exists() {
            break;
        }
        missing.push(path.to_path_buf());
        cursor = path.parent();
    }
    for path in missing.into_iter().rev() {
        std::fs::create_dir(&path).map_err(|error| io(&path, error))?;
        if !manifest.created.contains(&path) {
            manifest.created.push(path);
        }
    }
    Ok(())
}

/// Write a file Bridge owns outright (created by setup, deleted by remove).
fn write_owned(path: &Path, contents: &str, executable: bool, manifest: &mut Manifest, report: &mut Report) -> Result<(), String> {
    ensure_dir(path.parent().expect("owned file has a parent"), manifest)?;
    let existed_before = path.exists() && !manifest.created.contains(&path.to_path_buf());
    if existed_before {
        // A file of ours from an older setup, or the user's: never clobber
        // the user's. Back it up like any other modified file.
        back_up(path, manifest)?;
    }
    if std::fs::read_to_string(path).ok().as_deref() != Some(contents) {
        std::fs::write(path, contents).map_err(|error| io(path, error))?;
        report.written.push(path.to_path_buf());
    }
    #[cfg(unix)]
    if executable {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).map_err(|error| io(path, error))?;
    }
    #[cfg(not(unix))]
    let _ = executable;
    if !existed_before && !manifest.created.contains(&path.to_path_buf()) {
        manifest.created.push(path.to_path_buf());
    }
    Ok(())
}

/// Save the pristine copy of a user file, once.
fn back_up(path: &Path, manifest: &mut Manifest) -> Result<(), String> {
    if manifest.modified.iter().any(|entry| entry.path == path) {
        return Ok(());
    }
    let paths = Paths::new(home_of(path));
    let relative = path.strip_prefix(&paths.config).unwrap_or(path);
    let backup = paths.backups.join(relative);
    std::fs::create_dir_all(backup.parent().expect("backup has a parent")).map_err(|error| io(&backup, error))?;
    std::fs::copy(path, &backup).map_err(|error| io(path, error))?;
    manifest.modified.push(Modified { path: path.to_path_buf(), backup });
    Ok(())
}

/// `~/.config/x/y` → `~`.
fn home_of(path: &Path) -> &Path {
    path.ancestors().find(|ancestor| ancestor.ends_with(".config")).and_then(Path::parent).unwrap_or(Path::new("/"))
}

fn menu_block(exe: &str) -> String {
    let entry = |id: &str, icon: &str, label: &str, args: &str, description: &str| {
        format!(
            "  {}: {},",
            serde_json::to_string(id).unwrap(),
            serde_json::json!({ "icon": icon, "label": label, "action": format!("{exe} {args}"), "description": description })
        )
    };
    [
        MENU_BEGIN.to_string(),
        format!("  \"bridge\": {},", serde_json::json!({ "icon": "\u{f06a9}", "label": "Bridge", "description": "Supervise coding agents" })),
        entry("bridge.new-chat", "\u{f0eb}", "New chat", "--new-chat", "Start a Bridge chat"),
        entry("bridge.quick", "\u{f0e7}", "Quick prompt", "--quick", "Float a Bridge prompt over your work"),
        entry("bridge.approvals", "\u{f00c}", "Review approvals", "--approvals", "Agents waiting for you"),
        entry("bridge.mission-control", "\u{f009}", "Mission Control", "--mission-control", "Every running agent at once"),
        MENU_END.to_string(),
    ]
    .join("\n")
}

/// Insert (or refresh) the marked block right after the file's opening brace.
fn merge_menu(existing: Option<&str>, block: &str) -> String {
    let Some(text) = existing else {
        return format!("{{\n{block}\n}}\n");
    };
    let without = strip_menu_block(text);
    match without.find('{') {
        Some(open) => format!("{}\n{block}{}", &without[..=open], &without[open + 1..]),
        // Not an object at all; Omarchy would ignore it anyway. Leave theirs
        // in place and start a valid file around our entries.
        None => format!("{{\n{block}\n}}\n"),
    }
}

/// The user's file without our block, byte for byte as it was.
fn strip_menu_block(text: &str) -> String {
    let (Some(start), Some(end)) = (text.find(MENU_BEGIN), text.find(MENU_END)) else { return text.to_string() };
    // The block was inserted as "\n<block>" after the brace.
    let start = if start > 0 && text.as_bytes()[start - 1] == b'\n' { start - 1 } else { start };
    format!("{}{}", &text[..start], &text[end + MENU_END.len()..])
}

fn hypr_module(exe: &str) -> String {
    format!(
        "-- Bridge on Hyprland. Added by bridge-deck --omarchy setup --hyprland;\n\
         -- undo with bridge-deck --omarchy remove.\n\
         \n\
         -- The quick prompt floats over whatever you are doing, on its own special\n\
         -- workspace, and Esc hides it without ending the session.\n\
         o.window({{ class = \"^bridge$\", title = \"^Bridge Quick$\" }}, {{ float = true, center = true, size = {{ 864, 470 }}, workspace = \"special:bridge\" }})\n\
         \n\
         o.bind(\"{QUICK_PROMPT_KEYS}\", \"Bridge quick prompt\", \"{exe} --quick\")\n"
    )
}

/// Wire Bridge into Omarchy under `home`. `exe` is how menus, hooks and
/// bindings invoke Bridge (a bare name when it is on PATH).
pub fn setup(home: &Path, exe: &str, hyprland: bool) -> Result<Report, String> {
    let paths = Paths::new(home);
    let mut manifest = load_manifest(&paths)?;
    let mut report = Report::default();

    let hook = paths.config.join("omarchy/hooks/theme-set.d/bridge");
    // omarchy-hook runs each hook synchronously with bash; background the
    // retint so a theme switch never waits on Bridge.
    let hook_body = format!(
        "#!/bin/bash\n# Retint Bridge on a theme switch. Added by bridge-deck --omarchy setup.\n{exe} --omarchy retint >/dev/null 2>&1 &\n"
    );
    write_owned(&hook, &hook_body, true, &mut manifest, &mut report)?;

    let plugin_dir = paths.config.join("omarchy/plugins").join(PLUGIN_ID);
    for (name, contents) in PLUGIN_FILES {
        write_owned(&plugin_dir.join(name), contents, false, &mut manifest, &mut report)?;
    }

    let menu = paths.config.join("omarchy/extensions/omarchy-menu.jsonc");
    let existing = std::fs::read_to_string(&menu).ok();
    let merged = merge_menu(existing.as_deref(), &menu_block(exe));
    if existing.as_deref() != Some(merged.as_str()) {
        if existing.is_some() {
            back_up(&menu, &mut manifest)?;
        } else {
            ensure_dir(menu.parent().expect("menu has a parent"), &mut manifest)?;
            manifest.created.push(menu.clone());
        }
        std::fs::write(&menu, &merged).map_err(|error| io(&menu, error))?;
        report.written.push(menu);
    }

    if hyprland {
        let config = paths.config.join("hypr/hyprland.lua");
        let user = std::fs::read_to_string(&config)
            .map_err(|_| format!("{} not found; Omarchy's Hyprland config is required for --hyprland", config.display()))?;
        write_owned(&paths.config.join("hypr/bridge.lua"), &hypr_module(exe), false, &mut manifest, &mut report)?;
        if !user.contains(HYPR_REQUIRE) {
            back_up(&config, &mut manifest)?;
            let separator = if user.ends_with('\n') { "" } else { "\n" };
            std::fs::write(&config, format!("{user}{separator}{HYPR_REQUIRE}\n")).map_err(|error| io(&config, error))?;
            report.written.push(config);
        }
    }

    save_manifest(&paths, &manifest)?;
    Ok(report)
}

/// Undo every setup under `home`.
pub fn remove(home: &Path) -> Result<Report, String> {
    let paths = Paths::new(home);
    let manifest = load_manifest(&paths)?;
    let mut report = Report::default();
    for entry in &manifest.modified {
        std::fs::copy(&entry.backup, &entry.path).map_err(|error| io(&entry.backup, error))?;
        report.restored.push(entry.path.clone());
    }
    // Files first, then directories deepest first, skipping any directory
    // the user has since put their own files into.
    let (dirs, files): (Vec<_>, Vec<_>) = manifest.created.iter().partition(|path| path.is_dir());
    for file in files {
        if std::fs::remove_file(file).is_ok() {
            report.removed.push(file.clone());
        }
    }
    let mut dirs: Vec<_> = dirs.into_iter().collect();
    dirs.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for dir in dirs {
        if std::fs::remove_dir(dir).is_ok() {
            report.removed.push(dir.clone());
        }
    }
    let _ = std::fs::remove_dir_all(&paths.backups);
    let _ = std::fs::remove_file(&paths.manifest);
    let _ = std::fs::remove_dir(paths.manifest.parent().expect("manifest has a parent"));
    Ok(report)
}

/// How Omarchy files should invoke Bridge: the AppImage itself, the packaged
/// command when it is on PATH, else this executable's absolute path.
pub fn invocation() -> String {
    if let Some(appimage) = std::env::var_os("APPIMAGE") {
        return shell_quote(&appimage.to_string_lossy());
    }
    let on_path = std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join("bridge-deck").is_file()))
        .unwrap_or(false);
    if on_path {
        return "bridge-deck".into();
    }
    std::env::current_exe()
        .map(|exe| shell_quote(&exe.to_string_lossy()))
        .unwrap_or_else(|_| "bridge-deck".into())
}

fn shell_quote(value: &str) -> String {
    if value.chars().all(|c| c.is_ascii_alphanumeric() || "/._-".contains(c)) {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

/// Best-effort: tell the running Omarchy shell about the plugin. Failure is
/// fine; the plugin is picked up on the next shell start.
fn run_quietly(program: &str, args: &[&str]) {
    let _ = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// The `bridge-deck --omarchy setup|remove` entry point. Returns the exit code.
pub fn run_cli(action: crate::launch::OmarchyAction, hyprland: bool) -> i32 {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        eprintln!("bridge-deck: HOME is not set");
        return 1;
    };
    let result = match action {
        crate::launch::OmarchyAction::Setup => setup(&home, &invocation(), hyprland).inspect(|_| {
            run_quietly("omarchy-shell", &["shell", "rescanPlugins"]);
            run_quietly("omarchy", &["plugin", "enable", PLUGIN_ID, "--section", "right"]);
        }),
        crate::launch::OmarchyAction::Remove => {
            run_quietly("omarchy", &["plugin", "disable", PLUGIN_ID]);
            remove(&home)
        }
        crate::launch::OmarchyAction::Retint => return 0,
    };
    match result {
        Ok(report) => {
            for path in &report.written {
                println!("wrote    {}", path.display());
            }
            for path in &report.restored {
                println!("restored {}", path.display());
            }
            for path in &report.removed {
                println!("removed  {}", path.display());
            }
            0
        }
        Err(error) => {
            eprintln!("bridge-deck: {error}");
            1
        }
    }
}

#[cfg(test)]
pub(crate) mod tests_support {
    pub fn hypr_module_for_test() -> String {
        super::hypr_module("bridge-deck")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER_MENU: &str = "{\n  // my menu\n  \"personal\": {\"icon\":\"\",\"label\":\"Personal\"},\n  \"personal.notes\": {\"label\":\"Notes\",\"action\":\"nvim ~/notes\"},\n}\n";
    const USER_HYPR: &str = "-- my hyprland\nrequire(\"default.hypr.omarchy\")\nrequire(\"hypr.bindings\")\n";

    fn home_with_user_files() -> tempfile::TempDir {
        let home = tempfile::tempdir().unwrap();
        let config = home.path().join(".config");
        std::fs::create_dir_all(config.join("omarchy/extensions")).unwrap();
        std::fs::create_dir_all(config.join("hypr")).unwrap();
        std::fs::write(config.join("omarchy/extensions/omarchy-menu.jsonc"), USER_MENU).unwrap();
        std::fs::write(config.join("hypr/hyprland.lua"), USER_HYPR).unwrap();
        home
    }

    /// Every file under `root`, relative path to bytes.
    fn snapshot(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
        fn walk(dir: &Path, root: &Path, out: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(&path, root, out);
                } else {
                    out.insert(path.strip_prefix(root).unwrap().to_path_buf(), std::fs::read(&path).unwrap());
                }
            }
        }
        let mut out = Default::default();
        walk(root, root, &mut out);
        out
    }

    #[test]
    fn setup_then_remove_leaves_the_users_files_byte_identical() {
        let home = home_with_user_files();
        let before = snapshot(home.path());
        setup(home.path(), "bridge-deck", true).unwrap();
        let config = home.path().join(".config");
        assert!(config.join("omarchy/hooks/theme-set.d/bridge").is_file());
        assert!(config.join("omarchy/plugins/app.bridge.status/manifest.json").is_file());
        assert!(config.join("hypr/bridge.lua").is_file());
        let menu = std::fs::read_to_string(config.join("omarchy/extensions/omarchy-menu.jsonc")).unwrap();
        assert!(menu.contains("\"bridge.new-chat\"") && menu.contains("\"personal.notes\""));
        assert!(std::fs::read_to_string(config.join("hypr/hyprland.lua")).unwrap().ends_with(&format!("{HYPR_REQUIRE}\n")));

        remove(home.path()).unwrap();
        assert_eq!(snapshot(home.path()), before);
        assert!(!config.join("omarchy/hooks").exists(), "directories setup created are gone");
        assert!(!config.join("bridge").exists());
    }

    #[test]
    fn setup_twice_changes_nothing_the_second_time() {
        let home = home_with_user_files();
        setup(home.path(), "bridge-deck", true).unwrap();
        let first = snapshot(home.path());
        let report = setup(home.path(), "bridge-deck", true).unwrap();
        assert!(report.written.is_empty(), "nothing rewritten: {:?}", report.written);
        assert_eq!(snapshot(home.path()), first);
        // And remove after a double setup still restores the originals.
        remove(home.path()).unwrap();
        let config = home.path().join(".config");
        assert_eq!(std::fs::read_to_string(config.join("omarchy/extensions/omarchy-menu.jsonc")).unwrap(), USER_MENU);
        assert_eq!(std::fs::read_to_string(config.join("hypr/hyprland.lua")).unwrap(), USER_HYPR);
    }

    #[test]
    fn nothing_is_written_outside_dot_config() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".config/hypr")).unwrap();
        std::fs::write(home.path().join(".config/hypr/hyprland.lua"), USER_HYPR).unwrap();
        setup(home.path(), "bridge-deck", true).unwrap();
        let entries: Vec<_> = std::fs::read_dir(home.path()).unwrap().map(|entry| entry.unwrap().file_name()).collect();
        assert_eq!(entries, vec![std::ffi::OsString::from(".config")]);
        remove(home.path()).unwrap();
        assert_eq!(snapshot(home.path()).keys().collect::<Vec<_>>(), vec![Path::new(".config/hypr/hyprland.lua")]);
    }

    #[test]
    fn a_missing_menu_file_is_created_and_removed_again() {
        let home = tempfile::tempdir().unwrap();
        setup(home.path(), "bridge-deck", false).unwrap();
        let menu = home.path().join(".config/omarchy/extensions/omarchy-menu.jsonc");
        let text = std::fs::read_to_string(&menu).unwrap();
        assert!(text.starts_with("{\n") && text.trim_end().ends_with('}'));
        assert!(!home.path().join(".config/hypr").exists(), "no Hyprland files without --hyprland");
        remove(home.path()).unwrap();
        assert!(!home.path().join(".config").join("omarchy").exists());
    }

    #[test]
    fn the_menu_block_parses_the_way_omarchy_parses_jsonc() {
        // Omarchy strips whole-line // comments and trailing commas, then
        // parses JSON. Mirror that to prove the merged file stays valid.
        let merged = merge_menu(Some(USER_MENU), &menu_block("'/opt/Bridge App/bridge-deck'"));
        let stripped: String = merged.lines().filter(|line| !line.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
        let without_trailing = regex_lite_trailing_commas(&stripped);
        let value: serde_json::Value = serde_json::from_str(&without_trailing).unwrap();
        assert_eq!(value["bridge.approvals"]["action"], "'/opt/Bridge App/bridge-deck' --approvals");
        assert_eq!(value["personal.notes"]["action"], "nvim ~/notes");
        assert_eq!(strip_menu_block(&merged), USER_MENU);
    }

    fn regex_lite_trailing_commas(text: &str) -> String {
        let mut out = String::new();
        let chars: Vec<char> = text.chars().collect();
        for (i, c) in chars.iter().enumerate() {
            if *c == ',' {
                let next = chars[i + 1..].iter().find(|c| !c.is_whitespace());
                if matches!(next, Some('}') | Some(']')) {
                    continue;
                }
            }
            out.push(*c);
        }
        out
    }

    #[test]
    fn hyprland_setup_refuses_without_an_omarchy_hyprland_config() {
        let home = tempfile::tempdir().unwrap();
        assert!(setup(home.path(), "bridge-deck", true).unwrap_err().contains("hyprland.lua"));
    }

    #[test]
    fn the_quick_prompt_binding_and_rule_target_the_bridge_window() {
        let module = hypr_module("bridge-deck");
        assert!(module.contains("o.bind(\"SUPER + SHIFT + CTRL + B\", \"Bridge quick prompt\", \"bridge-deck --quick\")"));
        assert!(module.contains("class = \"^bridge$\""));
        assert!(module.contains("workspace = \"special:bridge\""));
    }

    #[test]
    fn invocations_with_spaces_are_quoted_for_the_shell() {
        assert_eq!(shell_quote("/usr/bin/bridge-deck"), "/usr/bin/bridge-deck");
        assert_eq!(shell_quote("/opt/Bridge App/bridge"), "'/opt/Bridge App/bridge'");
        assert_eq!(shell_quote("/tmp/it's"), "'/tmp/it'\\''s'");
    }
}
