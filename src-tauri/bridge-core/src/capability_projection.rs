//! Canonical capability roots shared by adapters and user-facing discovery.
//!
//! Providers support overlapping compatibility roots. Keeping this matrix here
//! prevents installation, slash expansion, and worker launch from disagreeing
//! about what is available.

use crate::BridgeError;
use serde::Serialize;
use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapabilityHarness {
    Claude,
    Codex,
    OpenCode,
}

impl CapabilityHarness {
    pub fn from_id(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "claude" | "claude-code" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "opencode" => Some(Self::OpenCode),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct CapabilityEnvironment {
    pub claude_config_dir: Option<PathBuf>,
    pub codex_home: Option<PathBuf>,
    pub opencode_config_dir: Option<PathBuf>,
    pub xdg_config_home: Option<PathBuf>,
}

impl CapabilityEnvironment {
    pub fn from_process() -> Self {
        Self {
            claude_config_dir: env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from),
            codex_home: env::var_os("CODEX_HOME").map(PathBuf::from),
            opencode_config_dir: env::var_os("OPENCODE_CONFIG_DIR").map(PathBuf::from),
            xdg_config_home: env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        }
    }
}

pub fn user_config_root(
    harness: CapabilityHarness,
    home: &Path,
    environment: &CapabilityEnvironment,
) -> PathBuf {
    match harness {
        CapabilityHarness::Claude => environment
            .claude_config_dir
            .clone()
            .unwrap_or_else(|| home.join(".claude")),
        CapabilityHarness::Codex => environment
            .codex_home
            .clone()
            .unwrap_or_else(|| home.join(".codex")),
        CapabilityHarness::OpenCode => environment
            .opencode_config_dir
            .clone()
            .or_else(|| {
                environment
                    .xdg_config_home
                    .as_ref()
                    .map(|root| root.join("opencode"))
            })
            .unwrap_or_else(|| home.join(".config/opencode")),
    }
}

pub fn skill_install_root(harness: CapabilityHarness, home: &Path) -> PathBuf {
    skill_install_root_with_environment(harness, home, &CapabilityEnvironment::from_process())
}

pub fn skill_install_root_with_environment(
    harness: CapabilityHarness,
    home: &Path,
    environment: &CapabilityEnvironment,
) -> PathBuf {
    match harness {
        // skills@1.5.19 uses the Agent Skills standard root for Codex.
        CapabilityHarness::Codex => home.join(".agents/skills"),
        CapabilityHarness::Claude => user_config_root(harness, home, environment).join("skills"),
        CapabilityHarness::OpenCode => user_config_root(harness, home, environment).join("skills"),
    }
}

pub fn skill_discovery_roots(
    harness: CapabilityHarness,
    home: &Path,
    project: Option<&Path>,
) -> Vec<PathBuf> {
    skill_discovery_roots_with_environment(
        harness,
        home,
        project,
        &CapabilityEnvironment::from_process(),
    )
}

pub fn skill_discovery_roots_with_environment(
    harness: CapabilityHarness,
    home: &Path,
    project: Option<&Path>,
    environment: &CapabilityEnvironment,
) -> Vec<PathBuf> {
    let config = user_config_root(harness, home, environment);
    let mut roots = Vec::new();
    if let Some(project) = project {
        match harness {
            CapabilityHarness::Claude => roots.push(project.join(".claude/skills")),
            CapabilityHarness::Codex => roots.extend([
                project.join(".agents/skills"),
                project.join(".codex/skills"),
            ]),
            CapabilityHarness::OpenCode => roots.extend([
                project.join(".opencode/skills"),
                project.join(".opencode/skill"),
                project.join(".agents/skills"),
                project.join(".claude/skills"),
            ]),
        }
    }
    match harness {
        CapabilityHarness::Claude => roots.push(config.join("skills")),
        CapabilityHarness::Codex => {
            roots.extend([home.join(".agents/skills"), config.join("skills")]);
        }
        CapabilityHarness::OpenCode => roots.extend([
            config.join("skills"),
            config.join("skill"),
            home.join(".agents/skills"),
            home.join(".claude/skills"),
        ]),
    }
    stable_deduplicate(roots)
}

pub fn command_discovery_roots(
    harness: CapabilityHarness,
    home: &Path,
    project: Option<&Path>,
) -> Vec<PathBuf> {
    command_discovery_roots_with_environment(
        harness,
        home,
        project,
        &CapabilityEnvironment::from_process(),
    )
}

pub fn command_discovery_roots_with_environment(
    harness: CapabilityHarness,
    home: &Path,
    project: Option<&Path>,
    environment: &CapabilityEnvironment,
) -> Vec<PathBuf> {
    let config = user_config_root(harness, home, environment);
    let mut roots = Vec::new();
    if let Some(project) = project {
        match harness {
            CapabilityHarness::Claude => roots.push(project.join(".claude/commands")),
            CapabilityHarness::Codex => roots.push(project.join(".codex/prompts")),
            CapabilityHarness::OpenCode => {}
        }
    }
    match harness {
        CapabilityHarness::Claude => roots.push(config.join("commands")),
        CapabilityHarness::Codex => roots.push(config.join("prompts")),
        CapabilityHarness::OpenCode => {}
    }
    stable_deduplicate(roots)
}

fn stable_deduplicate(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    paths
        .into_iter()
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

pub fn configured_capability_summary(
    harness: CapabilityHarness,
    home: &Path,
    project: Option<&Path>,
) -> String {
    let loaded_skills = skill_discovery_roots(harness, home, project)
        .into_iter()
        .filter(|path| path.is_dir())
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let loaded_commands = command_discovery_roots(harness, home, project)
        .into_iter()
        .filter(|path| path.is_dir())
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    format!(
        "Harness capability inventory: provider={}; skill roots={}; command roots={}. Use capabilities from these roots and the tools presented by the harness. Do not claim an absent capability is installed.",
        harness.as_str(),
        if loaded_skills.is_empty() { "none".into() } else { loaded_skills.join(", ") },
        if loaded_commands.is_empty() { "none".into() } else { loaded_commands.join(", ") },
    )
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectedCapability {
    pub kind: String,
    pub source: String,
    pub destination: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityProjectionNotice {
    pub kind: String,
    pub path: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityProjectionReport {
    pub schema_version: u32,
    pub harness: String,
    pub projected: Vec<ProjectedCapability>,
    pub withheld: Vec<CapabilityProjectionNotice>,
    pub unavailable: Vec<CapabilityProjectionNotice>,
    pub failures: Vec<CapabilityProjectionNotice>,
}

impl CapabilityProjectionReport {
    fn new(harness: CapabilityHarness) -> Self {
        Self {
            schema_version: 1,
            harness: harness.as_str().to_owned(),
            projected: Vec::new(),
            withheld: Vec::new(),
            unavailable: Vec::new(),
            failures: Vec::new(),
        }
    }

    pub fn unavailable(harness: CapabilityHarness, kind: &str, path: &Path, reason: &str) -> Self {
        let mut report = Self::new(harness);
        report.unavailable.push(CapabilityProjectionNotice {
            kind: kind.into(),
            path: path.to_string_lossy().into_owned(),
            reason: reason.into(),
        });
        report
    }

    pub fn summary(&self) -> String {
        let summarize = |kinds: Vec<&str>| {
            let mut seen = HashSet::new();
            kinds
                .into_iter()
                .filter(|kind| seen.insert(*kind))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let projected = summarize(
            self.projected
                .iter()
                .map(|item| item.kind.as_str())
                .collect(),
        );
        let unavailable = summarize(
            self.unavailable
                .iter()
                .map(|item| item.kind.as_str())
                .collect(),
        );
        let mut summary = format!(
            "Harness capabilities: {} read-only worker; projected: {}.",
            self.harness,
            if projected.is_empty() {
                "none"
            } else {
                &projected
            }
        );
        if !unavailable.is_empty() {
            summary.push_str(&format!(" Not configured on this machine: {unavailable}."));
        }
        if !self.withheld.is_empty() {
            summary.push_str(" Some capabilities were withheld by Bridge policy; inspect worker.capabilities_projected for reasons.");
        }
        if !self.failures.is_empty() {
            summary.push_str(" Capability projection had failures; do not claim the affected capability is loaded.");
        }
        summary
    }
}

struct ProjectionEntry {
    kind: &'static str,
    source: PathBuf,
    destination: PathBuf,
}

pub fn project_read_only_capabilities(
    harness: CapabilityHarness,
    home: &Path,
    output: &Path,
) -> Result<CapabilityProjectionReport, BridgeError> {
    project_read_only_capabilities_with_environment(
        harness,
        home,
        output,
        &CapabilityEnvironment::from_process(),
    )
}

pub fn project_read_only_capabilities_with_environment(
    harness: CapabilityHarness,
    home: &Path,
    output: &Path,
    environment: &CapabilityEnvironment,
) -> Result<CapabilityProjectionReport, BridgeError> {
    let config = user_config_root(harness, home, environment);
    let isolated = match harness {
        CapabilityHarness::Claude => output.join(".claude"),
        CapabilityHarness::Codex => output.join(".codex"),
        CapabilityHarness::OpenCode => output.join(".config/opencode"),
    };
    fs::create_dir_all(&isolated)?;
    let entries = match harness {
        CapabilityHarness::Claude => {
            let state = environment
                .claude_config_dir
                .as_ref()
                .map(|root| root.join(".claude.json"))
                .unwrap_or_else(|| home.join(".claude.json"));
            vec![
                entry(
                    "credentials",
                    config.join(".credentials.json"),
                    isolated.join(".credentials.json"),
                ),
                entry("state-and-mcp", state, isolated.join(".claude.json")),
                entry(
                    "global-instructions",
                    config.join("CLAUDE.md"),
                    isolated.join("CLAUDE.md"),
                ),
                entry("skills", config.join("skills"), isolated.join("skills")),
                entry(
                    "commands",
                    config.join("commands"),
                    isolated.join("commands"),
                ),
                entry("agents", config.join("agents"), isolated.join("agents")),
                entry("plugins", config.join("plugins"), isolated.join("plugins")),
            ]
        }
        CapabilityHarness::Codex => vec![
            entry(
                "credentials",
                config.join("auth.json"),
                isolated.join("auth.json"),
            ),
            entry(
                "mcp-credentials",
                config.join(".credentials.json"),
                isolated.join(".credentials.json"),
            ),
            entry(
                "settings-and-mcp",
                config.join("config.toml"),
                isolated.join("config.toml"),
            ),
            entry(
                "global-instructions",
                config.join("AGENTS.md"),
                isolated.join("AGENTS.md"),
            ),
            entry(
                "global-instructions-override",
                config.join("AGENTS.override.md"),
                isolated.join("AGENTS.override.md"),
            ),
            entry("plugins", config.join("plugins"), isolated.join("plugins")),
        ],
        CapabilityHarness::OpenCode => Vec::new(),
    };
    let mut report = CapabilityProjectionReport::new(harness);
    if harness == CapabilityHarness::OpenCode {
        report.withheld.push(CapabilityProjectionNotice {
            kind: "read-only-worker".into(),
            path: isolated.to_string_lossy().into_owned(),
            reason: "OpenCode does not advertise a read-only transport in Bridge".into(),
        });
        return Ok(report);
    }
    for item in entries {
        project_entry(item, &mut report)?;
    }
    if harness == CapabilityHarness::Claude {
        project_sanitized_claude_settings(
            &config.join("settings.json"),
            &isolated.join("settings.json"),
            &mut report,
        )?;
    }
    if harness == CapabilityHarness::Codex {
        project_directory_children(
            "skills",
            &config.join("skills"),
            &isolated.join("skills"),
            &[".system"],
            &mut report,
        )?;
        project_directory_children(
            "agent-skills",
            &home.join(".agents/skills"),
            &output.join(".agents/skills"),
            &[],
            &mut report,
        )?;
        project_entry(
            entry(
                "agent-plugins",
                home.join(".agents/plugins"),
                output.join(".agents/plugins"),
            ),
            &mut report,
        )?;
    }
    Ok(report)
}

fn entry(kind: &'static str, source: PathBuf, destination: PathBuf) -> ProjectionEntry {
    ProjectionEntry {
        kind,
        source,
        destination,
    }
}

fn project_entry(
    entry: ProjectionEntry,
    report: &mut CapabilityProjectionReport,
) -> Result<(), BridgeError> {
    match fs::symlink_metadata(&entry.source) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            report.unavailable.push(CapabilityProjectionNotice {
                kind: entry.kind.into(),
                path: entry.source.to_string_lossy().into_owned(),
                reason: "not configured".into(),
            });
            return Ok(());
        }
        Err(error) => {
            report.failures.push(CapabilityProjectionNotice {
                kind: entry.kind.into(),
                path: entry.source.to_string_lossy().into_owned(),
                reason: format!("source inspection failed: {}", error.kind()),
            });
            return Ok(());
        }
    }
    match fs::symlink_metadata(&entry.destination) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            if fs::read_link(&entry.destination).ok().as_deref() != Some(entry.source.as_path()) {
                fs::remove_file(&entry.destination)?;
            }
        }
        Ok(_) => {
            report.failures.push(CapabilityProjectionNotice {
                kind: entry.kind.into(),
                path: entry.destination.to_string_lossy().into_owned(),
                reason: "projection destination already exists and was left untouched".into(),
            });
            return Ok(());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if fs::symlink_metadata(&entry.destination).is_err() {
        if let Some(parent) = entry.destination.parent() {
            fs::create_dir_all(parent)?;
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&entry.source, &entry.destination)?;
        #[cfg(not(unix))]
        return Err(BridgeError::Invalid(
            "Read-only capability projection is unsupported on this platform".into(),
        ));
    }
    report.projected.push(ProjectedCapability {
        kind: entry.kind.into(),
        source: entry.source.to_string_lossy().into_owned(),
        destination: entry.destination.to_string_lossy().into_owned(),
    });
    Ok(())
}

fn project_directory_children(
    kind: &'static str,
    source: &Path,
    destination: &Path,
    reserved_names: &[&str],
    report: &mut CapabilityProjectionReport,
) -> Result<(), BridgeError> {
    let entries = match fs::read_dir(source) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            report.unavailable.push(CapabilityProjectionNotice {
                kind: kind.into(),
                path: source.to_string_lossy().into_owned(),
                reason: "not configured".into(),
            });
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    fs::create_dir_all(destination)?;
    let mut entries = entries.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for child in entries {
        let name = child.file_name();
        if reserved_names.iter().any(|reserved| name == *reserved) {
            continue;
        }
        project_entry(entry(kind, child.path(), destination.join(name)), report)?;
    }
    Ok(())
}

fn project_sanitized_claude_settings(
    source: &Path,
    destination: &Path,
    report: &mut CapabilityProjectionReport,
) -> Result<(), BridgeError> {
    match fs::metadata(source) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            report.unavailable.push(CapabilityProjectionNotice {
                kind: "settings".into(),
                path: source.to_string_lossy().into_owned(),
                reason: "not configured".into(),
            });
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    }
    match fs::symlink_metadata(destination) {
        Ok(metadata) if metadata.file_type().is_symlink() => fs::remove_file(destination)?,
        Ok(_) if fs::read(destination).ok().as_deref() == Some(b"{}\n") => {}
        Ok(_) => {
            report.failures.push(CapabilityProjectionNotice {
                kind: "settings-sanitized".into(),
                path: destination.to_string_lossy().into_owned(),
                reason: "projection destination already exists and was left untouched".into(),
            });
            return Ok(());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if !destination.exists() {
        fs::write(destination, b"{}\n")?;
    }
    report.projected.push(ProjectedCapability {
        kind: "settings-sanitized".into(),
        source: source.to_string_lossy().into_owned(),
        destination: destination.to_string_lossy().into_owned(),
    });
    report.withheld.push(CapabilityProjectionNotice {
        kind: "user-hooks-permissions-and-env".into(),
        path: source.to_string_lossy().into_owned(),
        reason: "read-only workers load an empty settings document; capabilities are projected separately".into(),
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_matrix_includes_standard_legacy_and_project_locations() {
        let home = Path::new("/Users/test user");
        let project = Path::new("/repo with spaces");
        let environment = CapabilityEnvironment::default();

        assert_eq!(
            skill_discovery_roots_with_environment(
                CapabilityHarness::Codex,
                home,
                Some(project),
                &environment,
            ),
            vec![
                project.join(".agents/skills"),
                project.join(".codex/skills"),
                home.join(".agents/skills"),
                home.join(".codex/skills"),
            ]
        );
        assert_eq!(
            skill_install_root_with_environment(CapabilityHarness::Codex, home, &environment),
            home.join(".agents/skills")
        );
        assert_eq!(
            command_discovery_roots_with_environment(
                CapabilityHarness::Claude,
                home,
                Some(project),
                &environment,
            ),
            vec![
                project.join(".claude/commands"),
                home.join(".claude/commands"),
            ]
        );
    }

    #[test]
    fn configured_roots_are_used_without_mutating_process_environment() {
        let home = Path::new("/home/test");
        let environment = CapabilityEnvironment {
            claude_config_dir: Some(PathBuf::from("/configured/claude")),
            codex_home: Some(PathBuf::from("/configured/codex")),
            opencode_config_dir: None,
            xdg_config_home: Some(PathBuf::from("/configured/xdg")),
        };
        assert_eq!(
            skill_discovery_roots_with_environment(
                CapabilityHarness::Claude,
                home,
                None,
                &environment,
            ),
            vec![PathBuf::from("/configured/claude/skills")]
        );
        assert_eq!(
            command_discovery_roots_with_environment(
                CapabilityHarness::Codex,
                home,
                None,
                &environment,
            ),
            vec![PathBuf::from("/configured/codex/prompts")]
        );
        assert!(skill_discovery_roots_with_environment(
            CapabilityHarness::OpenCode,
            home,
            None,
            &environment,
        )
        .starts_with(&[
            PathBuf::from("/configured/xdg/opencode/skills"),
            PathBuf::from("/configured/xdg/opencode/skill"),
        ]));
    }

    #[test]
    fn duplicate_compatibility_roots_are_returned_once_in_precedence_order() {
        let home = Path::new("/home/test");
        let environment = CapabilityEnvironment {
            opencode_config_dir: Some(home.join(".agents")),
            ..CapabilityEnvironment::default()
        };
        let roots = skill_discovery_roots_with_environment(
            CapabilityHarness::OpenCode,
            home,
            None,
            &environment,
        );
        assert_eq!(
            roots
                .iter()
                .filter(|root| **root == home.join(".agents/skills"))
                .count(),
            1
        );
    }

    #[test]
    fn configured_summary_is_deterministic_and_reports_only_existing_roots() {
        let home = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        fs::create_dir_all(home.path().join(".agents/skills/review")).unwrap();
        fs::create_dir_all(project.path().join(".codex/prompts")).unwrap();
        let first = configured_capability_summary(
            CapabilityHarness::Codex,
            home.path(),
            Some(project.path()),
        );
        let second = configured_capability_summary(
            CapabilityHarness::Codex,
            home.path(),
            Some(project.path()),
        );
        assert_eq!(first, second);
        assert!(first.contains(home.path().join(".agents/skills").to_string_lossy().as_ref()));
        assert!(first.contains(project.path().join(".codex/prompts").to_string_lossy().as_ref()));
        assert!(!first.contains(".codex/skills"));
    }

    #[cfg(unix)]
    #[test]
    fn claude_projection_is_complete_idempotent_and_secret_free() {
        let home = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let config = home.path().join(".claude");
        fs::create_dir_all(config.join("skills/review")).unwrap();
        fs::create_dir_all(config.join("commands")).unwrap();
        fs::create_dir_all(config.join("agents")).unwrap();
        fs::create_dir_all(config.join("plugins")).unwrap();
        let secret = "sk-proj-do-not-report-this-secret";
        fs::write(config.join(".credentials.json"), secret).unwrap();
        fs::write(
            config.join("settings.json"),
            r#"{"hooks":{"PreToolUse":[{"command":"curl bad.example"}]},"permissions":{"allow":["Write"]}}"#,
        )
        .unwrap();
        fs::write(home.path().join(".claude.json"), "{}").unwrap();
        fs::write(config.join("CLAUDE.md"), "instructions").unwrap();

        let first = project_read_only_capabilities_with_environment(
            CapabilityHarness::Claude,
            home.path(),
            output.path(),
            &CapabilityEnvironment::default(),
        )
        .unwrap();
        let second = project_read_only_capabilities_with_environment(
            CapabilityHarness::Claude,
            home.path(),
            output.path(),
            &CapabilityEnvironment::default(),
        )
        .unwrap();
        assert_eq!(first, second);
        assert_eq!(first.projected.len(), 8);
        assert!(first.failures.is_empty());
        assert_eq!(
            fs::read_link(output.path().join(".claude/skills")).unwrap(),
            config.join("skills")
        );
        assert_eq!(
            fs::read_link(output.path().join(".claude/.claude.json")).unwrap(),
            home.path().join(".claude.json")
        );
        assert_eq!(
            fs::read_to_string(output.path().join(".claude/settings.json")).unwrap(),
            "{}\n"
        );
        assert!(first
            .withheld
            .iter()
            .any(|item| item.kind == "user-hooks-permissions-and-env"));
        assert!(!serde_json::to_string(&first).unwrap().contains(secret));
        assert!(!first
            .summary()
            .contains(home.path().to_string_lossy().as_ref()));
    }

    #[cfg(unix)]
    #[test]
    fn projection_repairs_wrong_symlinks_but_preserves_real_collisions() {
        let home = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let config = home.path().join(".codex");
        fs::create_dir_all(config.join("skills/review")).unwrap();
        fs::create_dir_all(home.path().join(".agents/skills/shared")).unwrap();
        fs::create_dir_all(home.path().join(".agents/plugins")).unwrap();
        fs::write(config.join("auth.json"), "secret-value").unwrap();
        fs::create_dir_all(output.path().join(".codex")).unwrap();
        fs::create_dir_all(output.path().join(".codex/skills")).unwrap();
        std::os::unix::fs::symlink(
            output.path().join("missing"),
            output.path().join(".codex/skills/review"),
        )
        .unwrap();
        fs::write(output.path().join(".codex/auth.json"), "owned-output").unwrap();

        let report = project_read_only_capabilities_with_environment(
            CapabilityHarness::Codex,
            home.path(),
            output.path(),
            &CapabilityEnvironment::default(),
        )
        .unwrap();
        assert_eq!(
            fs::read_link(output.path().join(".codex/skills/review")).unwrap(),
            config.join("skills/review")
        );
        fs::create_dir(output.path().join(".codex/skills/.system")).unwrap();
        assert_eq!(
            fs::read_link(output.path().join(".agents/skills/shared")).unwrap(),
            home.path().join(".agents/skills/shared")
        );
        assert_eq!(
            fs::read_link(output.path().join(".agents/plugins")).unwrap(),
            home.path().join(".agents/plugins")
        );
        assert_eq!(
            fs::read_to_string(output.path().join(".codex/auth.json")).unwrap(),
            "owned-output"
        );
        assert!(report
            .failures
            .iter()
            .any(|item| item.kind == "credentials"));
        let serialized = serde_json::to_string(&report).unwrap();
        assert!(!serialized.contains("secret-value"));
        assert!(!serialized.contains("owned-output"));
    }
}
