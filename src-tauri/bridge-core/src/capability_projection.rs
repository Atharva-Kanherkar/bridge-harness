//! Canonical capability roots shared by adapters and user-facing discovery.
//!
//! Providers support overlapping compatibility roots. Keeping this matrix here
//! prevents installation, slash expansion, and worker launch from disagreeing
//! about what is available.

use std::{
    collections::HashSet,
    env,
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
}
