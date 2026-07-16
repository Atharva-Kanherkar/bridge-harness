use crate::{binary, marketplace, BridgeError};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};
use uuid::Uuid;

const INSTALLER_PACKAGE: &str = "skills@1.5.19";
const CONSENT_TTL_SECONDS: i64 = 300;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogSkill {
    id: String,
    slug: String,
    name: String,
    description: String,
    source: String,
    source_url: String,
    skill_path: String,
    pinned_ref: String,
    installs: u64,
    official: bool,
    compatibility: Vec<SkillProvider>,
    file_count: usize,
    permissions: Vec<String>,
    risk: String,
    risk_summary: String,
    categories: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum SkillProvider {
    Codex,
    Claude,
}

impl SkillProvider {
    fn agent_name(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude-code",
        }
    }

    fn skill_root(self, home: &Path) -> PathBuf {
        match self {
            // skills@1.5.19 uses the Agent Skills standard root for Codex.
            Self::Codex => home.join(".agents/skills"),
            Self::Claude => home.join(".claude/skills"),
        }
    }

    fn discovery_roots(self, home: &Path) -> Vec<PathBuf> {
        match self {
            // Codex supports both the shared standard and legacy native root.
            Self::Codex => vec![home.join(".agents/skills"), home.join(".codex/skills")],
            Self::Claude => vec![home.join(".claude/skills")],
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SkillAction {
    Install,
    Rollback,
    Uninstall,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillProviderState {
    provider: SkillProvider,
    installed: bool,
    managed: bool,
    installed_ref: Option<String>,
    update_available: bool,
    rollback_available: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunitySkill {
    id: String,
    slug: String,
    name: String,
    description: String,
    source: String,
    source_url: String,
    pinned_ref: String,
    installs: u64,
    official: bool,
    compatibility: Vec<SkillProvider>,
    file_count: usize,
    permissions: Vec<String>,
    risk: String,
    risk_summary: String,
    categories: Vec<String>,
    provider_states: Vec<SkillProviderState>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalSkill {
    id: String,
    name: String,
    description: String,
    providers: Vec<SkillProvider>,
    source: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillCatalog {
    community: Vec<CommunitySkill>,
    personal: Vec<PersonalSkill>,
    installer: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilitySuggestion {
    pub id: String,
    pub name: String,
    pub command: String,
    pub relevance: String,
    pub source: String,
    pub providers: Vec<SkillProvider>,
    pub permissions: Vec<String>,
    pub risk: String,
    pub installed: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillPreview {
    confirmation_id: String,
    expires_at: String,
    action: SkillAction,
    skill: CommunitySkill,
    targets: Vec<SkillProvider>,
    changes: Vec<String>,
    installer: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillActionResult {
    provider: SkillProvider,
    action: SkillAction,
    success: bool,
    message: String,
    error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SkillConsent {
    skill_id: String,
    action: SkillAction,
    targets: Vec<SkillProvider>,
    pinned_ref: String,
    expires_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SkillReceipt {
    skill_id: String,
    source: String,
    current_ref: String,
    previous_ref: Option<String>,
    installed_at: String,
}

fn catalog_entries() -> Result<Vec<CatalogSkill>, BridgeError> {
    serde_json::from_str(include_str!("community_skills.json")).map_err(|error| {
        BridgeError::Invalid(format!("Community skill catalog is invalid: {error}"))
    })
}

fn receipt_path(store: &Path, provider: SkillProvider, slug: &str) -> PathBuf {
    store
        .join("receipts")
        .join(provider.agent_name())
        .join(format!("{slug}.json"))
}

fn read_receipt(store: &Path, provider: SkillProvider, slug: &str) -> Option<SkillReceipt> {
    serde_json::from_slice(&fs::read(receipt_path(store, provider, slug)).ok()?).ok()
}

fn write_receipt(
    store: &Path,
    provider: SkillProvider,
    slug: &str,
    receipt: &SkillReceipt,
) -> Result<(), BridgeError> {
    let path = receipt_path(store, provider, slug);
    let parent = path
        .parent()
        .ok_or_else(|| BridgeError::Invalid("Skill receipt path is invalid".into()))?;
    fs::create_dir_all(parent)?;
    let bytes = serde_json::to_vec_pretty(receipt).map_err(|error| {
        BridgeError::Invalid(format!("Skill receipt could not be encoded: {error}"))
    })?;
    fs::write(path, bytes)?;
    Ok(())
}

fn community_view(entry: &CatalogSkill, home: &Path, store: &Path) -> CommunitySkill {
    let provider_states = entry
        .compatibility
        .iter()
        .copied()
        .map(|provider| {
            let installed = provider
                .skill_root(home)
                .join(&entry.slug)
                .join("SKILL.md")
                .is_file();
            let receipt = read_receipt(store, provider, &entry.slug)
                .filter(|receipt| receipt.skill_id == entry.id);
            SkillProviderState {
                provider,
                installed,
                managed: receipt.is_some(),
                installed_ref: receipt.as_ref().map(|value| value.current_ref.clone()),
                update_available: installed
                    && receipt
                        .as_ref()
                        .is_some_and(|value| value.current_ref != entry.pinned_ref),
                rollback_available: installed && receipt.is_some(),
            }
        })
        .collect();
    CommunitySkill {
        id: entry.id.clone(),
        slug: entry.slug.clone(),
        name: entry.name.clone(),
        description: entry.description.clone(),
        source: entry.source.clone(),
        source_url: entry.source_url.clone(),
        pinned_ref: entry.pinned_ref.clone(),
        installs: entry.installs,
        official: entry.official,
        compatibility: entry.compatibility.clone(),
        file_count: entry.file_count,
        permissions: entry.permissions.clone(),
        risk: entry.risk.clone(),
        risk_summary: entry.risk_summary.clone(),
        categories: entry.categories.clone(),
        provider_states,
    }
}

fn read_description(path: &Path) -> String {
    let content = fs::read_to_string(path).unwrap_or_default();
    if let Some(frontmatter) = content
        .strip_prefix("---")
        .and_then(|value| value.split_once("---").map(|parts| parts.0))
    {
        if let Some(value) = frontmatter
            .lines()
            .find_map(|line| line.trim().strip_prefix("description:"))
        {
            let clean = value.trim().trim_matches(['\'', '"']);
            if !clean.is_empty() && clean != ">" && clean != "|" {
                return clean.chars().take(240).collect();
            }
        }
    }
    content
        .lines()
        .find_map(|line| {
            let value = line.trim().trim_start_matches('#').trim();
            (!value.is_empty() && value != "---" && !value.starts_with("name:"))
                .then(|| value.chars().take(240).collect())
        })
        .unwrap_or_default()
}

fn collect_skill_files(root: &Path) -> Vec<(String, PathBuf)> {
    fn walk(base: &Path, directory: &Path, output: &mut Vec<(String, PathBuf)>) {
        let Ok(entries) = fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_name().to_string_lossy().starts_with('.') || !path.is_dir() {
                continue;
            }
            let skill = path.join("SKILL.md");
            if skill.is_file() {
                if let Ok(relative) = path.strip_prefix(base) {
                    let name = relative
                        .components()
                        .map(|part| part.as_os_str().to_string_lossy())
                        .collect::<Vec<_>>()
                        .join(":");
                    output.push((name, skill));
                }
            } else {
                walk(base, &path, output);
            }
        }
    }
    let mut output = Vec::new();
    walk(root, root, &mut output);
    output
}

fn personal_skills(home: &Path, store: &Path) -> Vec<PersonalSkill> {
    let mut grouped: HashMap<String, PersonalSkill> = HashMap::new();
    for provider in [SkillProvider::Codex, SkillProvider::Claude] {
        for (name, path) in provider
            .discovery_roots(home)
            .into_iter()
            .flat_map(|root| collect_skill_files(&root))
        {
            let slug = name.split(':').next_back().unwrap_or(&name);
            if read_receipt(store, provider, slug).is_some() {
                continue;
            }
            let entry = grouped
                .entry(name.clone())
                .or_insert_with(|| PersonalSkill {
                    id: format!("personal:{name}"),
                    name: name.clone(),
                    description: read_description(&path),
                    providers: Vec::new(),
                    source: "Personal skill".into(),
                });
            if !entry.providers.contains(&provider) {
                entry.providers.push(provider);
            }
        }
    }
    let mut values = grouped.into_values().collect::<Vec<_>>();
    values.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
    values
}

pub fn catalog(home: &Path, store: &Path) -> Result<SkillCatalog, BridgeError> {
    let community = catalog_entries()?
        .iter()
        .map(|entry| community_view(entry, home, store))
        .collect();
    Ok(SkillCatalog {
        community,
        personal: personal_skills(home, store),
        installer: INSTALLER_PACKAGE,
    })
}

fn query_tokens(value: &str) -> HashSet<String> {
    value
        .split(|character: char| !character.is_ascii_alphanumeric())
        .map(str::to_lowercase)
        .filter(|token| {
            token.len() > 2
                && !matches!(
                    token.as_str(),
                    "the" | "and" | "for" | "with" | "this" | "that" | "from" | "into" | "please"
                )
        })
        .collect()
}

pub fn suggestions(
    query: &str,
    provider: SkillProvider,
    home: &Path,
    store: &Path,
) -> Result<Vec<CapabilitySuggestion>, BridgeError> {
    let tokens = query_tokens(query);
    if tokens.is_empty() {
        return Ok(Vec::new());
    }
    let catalog = catalog(home, store)?;
    let mut scored = Vec::new();
    for skill in catalog.community.into_iter().filter(|skill| {
        skill
            .provider_states
            .iter()
            .any(|state| state.provider == provider && state.installed)
    }) {
        let haystack = query_tokens(&format!(
            "{} {} {}",
            skill.name,
            skill.description,
            skill.categories.join(" ")
        ));
        let matches = tokens.intersection(&haystack).cloned().collect::<Vec<_>>();
        if matches.is_empty() {
            continue;
        }
        let providers = vec![provider];
        scored.push((
            matches.len(),
            CapabilitySuggestion {
                id: skill.id,
                name: skill.name,
                command: skill.slug,
                relevance: format!(
                    "Matches {} in this task",
                    matches.into_iter().take(3).collect::<Vec<_>>().join(", ")
                ),
                source: skill.source,
                providers,
                permissions: skill.permissions,
                risk: skill.risk,
                installed: true,
            },
        ));
    }
    for skill in catalog
        .personal
        .into_iter()
        .filter(|skill| skill.providers.contains(&provider))
    {
        let haystack = query_tokens(&format!("{} {}", skill.name, skill.description));
        let matches = tokens.intersection(&haystack).cloned().collect::<Vec<_>>();
        if matches.is_empty() {
            continue;
        }
        scored.push((
            matches.len(),
            CapabilitySuggestion {
                id: skill.id,
                name: skill.name.clone(),
                command: skill.name,
                relevance: format!(
                    "Personal skill matches {}",
                    matches.into_iter().take(3).collect::<Vec<_>>().join(", ")
                ),
                source: skill.source,
                providers: skill.providers,
                permissions: vec!["Review personal SKILL.md instructions before use".into()],
                risk: "unknown".into(),
                installed: true,
            },
        ));
    }
    scored.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.name.cmp(&right.1.name))
    });
    Ok(scored.into_iter().take(3).map(|value| value.1).collect())
}

pub fn available_capabilities(home: &Path, store: &Path) -> Result<HashSet<String>, BridgeError> {
    let catalog = catalog(home, store)?;
    let mut capabilities = HashSet::new();
    for skill in catalog
        .community
        .into_iter()
        .filter(|skill| skill.provider_states.iter().any(|state| state.installed))
    {
        capabilities.insert("skill".into());
        capabilities.insert(format!("skill:{}", skill.slug));
        for category in &skill.categories {
            capabilities.insert(format!("skill_category:{category}"));
        }
        let name = skill.slug.to_lowercase();
        if name.contains("browser") || name.contains("playwright") {
            capabilities.extend([
                "browser".into(),
                "console_inspection".into(),
                "network_inspection".into(),
            ]);
        }
        if name.contains("review") || name.contains("audit") {
            capabilities.insert("code_review".into());
        }
    }
    for skill in catalog.personal {
        capabilities.insert("skill".into());
        capabilities.insert(format!("skill:{}", skill.name));
    }
    Ok(capabilities)
}

fn validate_targets(targets: &[SkillProvider]) -> Result<Vec<SkillProvider>, BridgeError> {
    let mut unique = Vec::new();
    for target in targets {
        if !unique.contains(target) {
            unique.push(*target);
        }
    }
    if unique.is_empty() {
        return Err(BridgeError::Invalid(
            "Choose at least one skill provider".into(),
        ));
    }
    Ok(unique)
}

pub fn preview(
    skill_id: &str,
    action: SkillAction,
    targets: &[SkillProvider],
    home: &Path,
    store: &Path,
    consents: &Mutex<HashMap<String, SkillConsent>>,
) -> Result<SkillPreview, BridgeError> {
    let targets = validate_targets(targets)?;
    let entry = catalog_entries()?
        .into_iter()
        .find(|entry| entry.id == skill_id)
        .ok_or_else(|| {
            BridgeError::Invalid("Community skill is not in the pinned catalog".into())
        })?;
    for target in &targets {
        if !entry.compatibility.contains(target) {
            return Err(BridgeError::Invalid(format!(
                "{} is not compatible with {}",
                entry.name,
                target.agent_name()
            )));
        }
        let destination = target.skill_root(home).join(&entry.slug).join("SKILL.md");
        let receipt = read_receipt(store, *target, &entry.slug);
        match action {
            SkillAction::Install if destination.is_file() && receipt.is_none() => {
                return Err(BridgeError::Invalid(format!(
                    "A personal {} skill named {} already exists; Bridge will not overwrite it",
                    target.agent_name(),
                    entry.slug
                )))
            }
            SkillAction::Rollback | SkillAction::Uninstall
                if receipt
                    .as_ref()
                    .is_none_or(|value| value.skill_id != entry.id) =>
            {
                return Err(BridgeError::Invalid(format!(
                    "{} is not managed by Bridge for {}",
                    entry.name,
                    target.agent_name()
                )))
            }
            _ => {}
        }
    }
    let expires_at = Utc::now().timestamp() + CONSENT_TTL_SECONDS;
    let confirmation_id = Uuid::new_v4().to_string();
    consents
        .lock()
        .unwrap()
        .retain(|_, consent| consent.expires_at > Utc::now().timestamp());
    consents.lock().unwrap().insert(
        confirmation_id.clone(),
        SkillConsent {
            skill_id: entry.id.clone(),
            action,
            targets: targets.clone(),
            pinned_ref: entry.pinned_ref.clone(),
            expires_at,
        },
    );
    let changes = targets.iter().map(|target| match action {
        SkillAction::Install => format!("Install the pinned snapshot into {} personal skills; preserve the previous Bridge-managed pin for rollback", target.agent_name()),
        SkillAction::Rollback => format!("Restore the previous Bridge-managed {} snapshot, or remove the first install", target.agent_name()),
        SkillAction::Uninstall => format!("Remove only the Bridge-managed {} copy", target.agent_name()),
    }).collect();
    Ok(SkillPreview {
        confirmation_id,
        expires_at: chrono::DateTime::from_timestamp(expires_at, 0)
            .unwrap_or_default()
            .to_rfc3339(),
        action,
        skill: community_view(&entry, home, store),
        targets,
        changes,
        installer: INSTALLER_PACKAGE,
    })
}

fn installer_args(entry: &CatalogSkill, provider: SkillProvider, pinned_ref: &str) -> Vec<String> {
    vec![
        "-y".into(),
        INSTALLER_PACKAGE.into(),
        "add".into(),
        format!(
            "https://github.com/{}/tree/{}/{}",
            entry.source, pinned_ref, entry.skill_path
        ),
        "--global".into(),
        "--agent".into(),
        provider.agent_name().into(),
        "--skill".into(),
        entry.slug.clone(),
        "--copy".into(),
        "--yes".into(),
    ]
}

fn remove_args(entry: &CatalogSkill, provider: SkillProvider) -> Vec<String> {
    vec![
        "-y".into(),
        INSTALLER_PACKAGE.into(),
        "remove".into(),
        "--global".into(),
        "--agent".into(),
        provider.agent_name().into(),
        "--skill".into(),
        entry.slug.clone(),
        "--yes".into(),
    ]
}

fn run_installer(args: &[String]) -> Result<(), BridgeError> {
    let npx = binary::resolve("npx").ok_or_else(|| {
        BridgeError::Invalid("Node.js and npx are required to install community skills".into())
    })?;
    let references = args.iter().map(String::as_str).collect::<Vec<_>>();
    let output = marketplace::bounded_output(&npx, &references, Duration::from_secs(180))?;
    if output.status.success() {
        return Ok(());
    }
    let detail = if output.stderr.is_empty() {
        &output.stdout
    } else {
        &output.stderr
    };
    Err(BridgeError::Adapter(format!(
        "Skill installer failed: {}",
        marketplace::sanitize_error(&String::from_utf8_lossy(detail))
    )))
}

pub fn execute(
    confirmation_id: &str,
    home: &Path,
    store: &Path,
    consents: &Mutex<HashMap<String, SkillConsent>>,
) -> Result<Vec<SkillActionResult>, BridgeError> {
    let consent = consents
        .lock()
        .unwrap()
        .remove(confirmation_id)
        .ok_or_else(|| {
            BridgeError::Invalid("Skill confirmation is missing, expired, or already used".into())
        })?;
    if consent.expires_at <= Utc::now().timestamp() {
        return Err(BridgeError::Invalid(
            "Skill confirmation expired; preview the change again".into(),
        ));
    }
    let entry = catalog_entries()?
        .into_iter()
        .find(|entry| entry.id == consent.skill_id && entry.pinned_ref == consent.pinned_ref)
        .ok_or_else(|| BridgeError::Invalid("Pinned skill catalog changed after preview".into()))?;
    let mut results = Vec::new();
    for provider in consent.targets {
        let operation = (|| -> Result<String, BridgeError> {
            let destination = provider.skill_root(home).join(&entry.slug).join("SKILL.md");
            let receipt = read_receipt(store, provider, &entry.slug);
            match consent.action {
                SkillAction::Install => {
                    if destination.is_file() && receipt.is_none() {
                        return Err(BridgeError::Invalid("A personal skill appeared after preview; Bridge refused to overwrite it".into()));
                    }
                    run_installer(&installer_args(&entry, provider, &entry.pinned_ref))?;
                    let previous_ref = receipt
                        .as_ref()
                        .and_then(|value| {
                            (value.current_ref != entry.pinned_ref)
                                .then(|| value.current_ref.clone())
                        })
                        .or_else(|| {
                            receipt
                                .as_ref()
                                .and_then(|value| value.previous_ref.clone())
                        });
                    write_receipt(
                        store,
                        provider,
                        &entry.slug,
                        &SkillReceipt {
                            skill_id: entry.id.clone(),
                            source: entry.source.clone(),
                            current_ref: entry.pinned_ref.clone(),
                            previous_ref,
                            installed_at: Utc::now().to_rfc3339(),
                        },
                    )?;
                    Ok(format!(
                        "Installed {} for {} at {}",
                        entry.name,
                        provider.agent_name(),
                        &entry.pinned_ref[..8]
                    ))
                }
                SkillAction::Rollback => {
                    let receipt = receipt
                        .filter(|value| value.skill_id == entry.id)
                        .ok_or_else(|| {
                            BridgeError::Invalid(
                                "Managed skill receipt disappeared after preview".into(),
                            )
                        })?;
                    if let Some(previous_ref) = receipt.previous_ref {
                        run_installer(&installer_args(&entry, provider, &previous_ref))?;
                        write_receipt(
                            store,
                            provider,
                            &entry.slug,
                            &SkillReceipt {
                                skill_id: entry.id.clone(),
                                source: entry.source.clone(),
                                current_ref: previous_ref.clone(),
                                previous_ref: Some(receipt.current_ref),
                                installed_at: Utc::now().to_rfc3339(),
                            },
                        )?;
                        Ok(format!(
                            "Rolled {} back for {}",
                            entry.name,
                            provider.agent_name()
                        ))
                    } else {
                        run_installer(&remove_args(&entry, provider))?;
                        let _ = fs::remove_file(receipt_path(store, provider, &entry.slug));
                        Ok(format!(
                            "Removed the first Bridge-managed {} install from {}",
                            entry.name,
                            provider.agent_name()
                        ))
                    }
                }
                SkillAction::Uninstall => {
                    if receipt
                        .as_ref()
                        .is_none_or(|value| value.skill_id != entry.id)
                    {
                        return Err(BridgeError::Invalid(
                            "Bridge will not uninstall an unmanaged personal skill".into(),
                        ));
                    }
                    run_installer(&remove_args(&entry, provider))?;
                    let _ = fs::remove_file(receipt_path(store, provider, &entry.slug));
                    Ok(format!(
                        "Uninstalled {} from {}",
                        entry.name,
                        provider.agent_name()
                    ))
                }
            }
        })();
        results.push(match operation {
            Ok(message) => SkillActionResult {
                provider,
                action: consent.action,
                success: true,
                message,
                error: None,
            },
            Err(error) => SkillActionResult {
                provider,
                action: consent.action,
                success: false,
                message: "Skill change failed".into(),
                error: Some(error.to_string()),
            },
        });
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn catalog_is_unique_pinned_and_bounded() {
        let entries = catalog_entries().unwrap();
        assert_eq!(entries.len(), 60);
        assert_eq!(
            entries
                .iter()
                .map(|entry| &entry.id)
                .collect::<HashSet<_>>()
                .len(),
            60
        );
        assert!(entries.iter().all(|entry| entry.pinned_ref.len() == 40
            && entry
                .pinned_ref
                .chars()
                .all(|value| value.is_ascii_hexdigit())));
        assert!(entries
            .iter()
            .all(|entry| entry.source.split_once('/').is_some()
                && entry.source_url.starts_with("https://github.com/")));
    }

    #[test]
    fn personal_skills_stay_separate_and_provider_specific() {
        let home = tempdir().unwrap();
        let store = tempdir().unwrap();
        let skill = home.path().join(".codex/skills/mine");
        fs::create_dir_all(&skill).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            "---\nname: mine\ndescription: Personal only\n---\n",
        )
        .unwrap();
        let catalog = catalog(home.path(), store.path()).unwrap();
        assert_eq!(catalog.personal.len(), 1);
        assert_eq!(catalog.personal[0].providers, vec![SkillProvider::Codex]);
        assert!(!catalog.community.iter().any(|entry| entry.name == "mine"));
    }

    #[test]
    fn task_suggestions_respect_the_active_provider() {
        let home = tempdir().unwrap();
        let store = tempdir().unwrap();
        let skill = home.path().join(".codex/skills/mine");
        fs::create_dir_all(&skill).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            "---\nname: mine\ndescription: Specialized testing workflow\n---\n",
        )
        .unwrap();
        assert_eq!(
            suggestions("testing", SkillProvider::Codex, home.path(), store.path())
                .unwrap()
                .len(),
            1
        );
        assert!(
            suggestions("testing", SkillProvider::Claude, home.path(), store.path())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn preview_is_single_use_and_refuses_personal_overwrite() {
        let home = tempdir().unwrap();
        let store = tempdir().unwrap();
        let consents = Mutex::new(HashMap::new());
        let entry = catalog_entries().unwrap().remove(0);
        let consent_preview = preview(
            &entry.id,
            SkillAction::Install,
            &[SkillProvider::Codex],
            home.path(),
            store.path(),
            &consents,
        )
        .unwrap();
        let consent = consents
            .lock()
            .unwrap()
            .remove(&consent_preview.confirmation_id)
            .unwrap();
        assert_eq!(consent.skill_id, entry.id);
        assert!(consents
            .lock()
            .unwrap()
            .remove(&consent_preview.confirmation_id)
            .is_none());
        let personal = SkillProvider::Codex
            .skill_root(home.path())
            .join(&entry.slug);
        fs::create_dir_all(&personal).unwrap();
        fs::write(personal.join("SKILL.md"), "personal").unwrap();
        assert!(preview(
            &entry.id,
            SkillAction::Install,
            &[SkillProvider::Codex],
            home.path(),
            store.path(),
            &consents
        )
        .unwrap_err()
        .to_string()
        .contains("personal"));
    }

    #[test]
    fn command_arguments_pin_source_and_never_contain_credentials() {
        let entry = catalog_entries().unwrap().remove(0);
        let args = installer_args(&entry, SkillProvider::Claude, &entry.pinned_ref);
        assert!(args.iter().any(|value| value.contains(&entry.pinned_ref)));
        assert!(args.iter().any(|value| value == "claude-code"));
        assert!(!args.join(" ").to_lowercase().contains("token="));
    }

    #[test]
    fn missing_managed_skill_is_immediately_ineligible() {
        let home = tempdir().unwrap();
        let store = tempdir().unwrap();
        let entry = catalog_entries().unwrap().remove(0);
        write_receipt(
            store.path(),
            SkillProvider::Codex,
            &entry.slug,
            &SkillReceipt {
                skill_id: entry.id.clone(),
                source: entry.source.clone(),
                current_ref: entry.pinned_ref,
                previous_ref: None,
                installed_at: Utc::now().to_rfc3339(),
            },
        )
        .unwrap();
        assert!(!available_capabilities(home.path(), store.path())
            .unwrap()
            .contains(&format!("skill:{}", entry.slug)));
    }
}
