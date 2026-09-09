use crate::{binary, marketplace, BridgeError};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
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
    OpenCode,
}

impl SkillProvider {
    fn agent_name(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude-code",
            Self::OpenCode => "opencode",
        }
    }

    fn skill_root(self, home: &Path) -> PathBuf {
        crate::capability_projection::skill_install_root(self.capability_harness(), home)
    }

    fn discovery_roots(self, home: &Path) -> Vec<PathBuf> {
        crate::capability_projection::skill_discovery_roots(
            self.capability_harness(),
            home,
            None,
        )
    }

    fn capability_harness(self) -> crate::capability_projection::CapabilityHarness {
        match self {
            Self::Codex => crate::capability_projection::CapabilityHarness::Codex,
            Self::Claude => crate::capability_projection::CapabilityHarness::Claude,
            Self::OpenCode => crate::capability_projection::CapabilityHarness::OpenCode,
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
    receipt_error: Option<String>,
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

static CATALOG: OnceLock<Result<Vec<CatalogSkill>, String>> = OnceLock::new();

fn catalog_entries() -> Result<&'static [CatalogSkill], BridgeError> {
    CATALOG
        .get_or_init(|| {
            serde_json::from_str(include_str!("community_skills.json"))
                .map_err(|error| format!("Community skill catalog is invalid: {error}"))
        })
        .as_deref()
        .map_err(|error| BridgeError::Invalid(error.clone()))
}

fn receipt_path(store: &Path, provider: SkillProvider, slug: &str) -> PathBuf {
    store
        .join("receipts")
        .join(provider.agent_name())
        .join(format!("{slug}.json"))
}

enum ReceiptRead {
    Missing,
    Valid(SkillReceipt),
    Corrupt(String),
}

fn read_receipt(store: &Path, provider: SkillProvider, slug: &str) -> ReceiptRead {
    let path = receipt_path(store, provider, slug);
    match fs::read(&path) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(receipt) => ReceiptRead::Valid(receipt),
            Err(error) => ReceiptRead::Corrupt(format!(
                "Bridge receipt for {slug} is unreadable ({error}). Remove {} and retry; the skill itself was not changed.",
                path.display()
            )),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => ReceiptRead::Missing,
        Err(error) => ReceiptRead::Corrupt(format!(
            "Bridge receipt for {slug} cannot be read ({error}). Check {} and retry; the skill itself was not changed.",
            path.display()
        )),
    }
}

fn matching_receipt(
    store: &Path,
    provider: SkillProvider,
    entry: &CatalogSkill,
) -> Result<Option<SkillReceipt>, BridgeError> {
    match read_receipt(store, provider, &entry.slug) {
        ReceiptRead::Missing => Ok(None),
        ReceiptRead::Valid(receipt) if receipt.skill_id == entry.id => Ok(Some(receipt)),
        ReceiptRead::Valid(_) => Err(BridgeError::Invalid(format!(
            "Bridge receipt for {} belongs to a different catalog skill. Remove {} and retry; the skill itself was not changed.",
            entry.slug,
            receipt_path(store, provider, &entry.slug).display()
        ))),
        ReceiptRead::Corrupt(error) => Err(BridgeError::Invalid(error)),
    }
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
    let temporary = path.with_extension(format!("json.tmp-{}", Uuid::new_v4()));
    fs::write(&temporary, bytes)?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(())
}

fn community_view(entry: &CatalogSkill, home: &Path, store: &Path) -> CommunitySkill {
    let provider_states = entry
        .compatibility
        .iter()
        .copied()
        .map(|provider| {
            let skill_path = provider
                .skill_root(home)
                .join(&entry.slug)
                .join("SKILL.md");
            let on_disk = fs::symlink_metadata(&skill_path)
                .is_ok_and(|metadata| metadata.file_type().is_file());
            let (receipt, mut receipt_error) = match read_receipt(store, provider, &entry.slug) {
                ReceiptRead::Valid(receipt) if receipt.skill_id == entry.id => (Some(receipt), None),
                ReceiptRead::Valid(_) => (None, Some("Bridge receipt belongs to a different catalog skill; repair or remove the stale receipt before changing this skill.".into())),
                ReceiptRead::Corrupt(error) => (None, Some(error)),
                ReceiptRead::Missing => (None, None),
            };
            if on_disk && receipt.is_none() && receipt_error.is_none() {
                receipt_error = Some("A Personal Skill uses this catalog slug. Bridge will not attribute, replace, or remove it.".into());
            }
            let installed = on_disk && receipt.is_some();
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
                receipt_error,
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
    const MAX_DISCOVERY_DEPTH: usize = 8;
    fn walk(base: &Path, directory: &Path, depth: usize, output: &mut Vec<(String, PathBuf)>) {
        if depth >= MAX_DISCOVERY_DEPTH {
            return;
        }
        let Ok(entries) = fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if entry.file_name().to_string_lossy().starts_with('.')
                || file_type.is_symlink()
                || !file_type.is_dir()
            {
                continue;
            }
            let skill = path.join("SKILL.md");
            let regular_skill =
                fs::symlink_metadata(&skill).is_ok_and(|metadata| metadata.file_type().is_file());
            if regular_skill {
                if let Ok(relative) = path.strip_prefix(base) {
                    let name = relative
                        .components()
                        .map(|part| part.as_os_str().to_string_lossy())
                        .collect::<Vec<_>>()
                        .join(":");
                    output.push((name, skill));
                }
            } else {
                walk(base, &path, depth + 1, output);
            }
        }
    }
    let mut output = Vec::new();
    walk(root, root, 0, &mut output);
    output
}

fn personal_skills(home: &Path, store: &Path) -> Vec<PersonalSkill> {
    let mut grouped: HashMap<String, PersonalSkill> = HashMap::new();
    for provider in [
        SkillProvider::Codex,
        SkillProvider::Claude,
        SkillProvider::OpenCode,
    ] {
        for (name, path) in provider
            .discovery_roots(home)
            .into_iter()
            .flat_map(|root| collect_skill_files(&root))
        {
            let slug = name.split(':').next_back().unwrap_or(&name);
            if !matches!(read_receipt(store, provider, slug), ReceiptRead::Missing) {
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
        .iter()
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
        let receipt = matching_receipt(store, *target, entry)?;
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
        .iter()
        .find(|entry| entry.id == consent.skill_id && entry.pinned_ref == consent.pinned_ref)
        .ok_or_else(|| BridgeError::Invalid("Pinned skill catalog changed after preview".into()))?;
    let mut results = Vec::new();
    for provider in consent.targets {
        let operation = (|| -> Result<String, BridgeError> {
            let destination = provider.skill_root(home).join(&entry.slug).join("SKILL.md");
            let receipt = matching_receipt(store, provider, entry)?;
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
                        entry.pinned_ref.get(..8).unwrap_or(&entry.pinned_ref)
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
        assert!(entries.iter().all(|entry| entry.file_count <= 256
            && !entry.skill_path.starts_with('/')
            && !entry.skill_path.split('/').any(|part| part == "..")));
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
    fn unmanaged_catalog_slug_keeps_personal_attribution() {
        let home = tempdir().unwrap();
        let store = tempdir().unwrap();
        let entry = catalog_entries().unwrap().first().unwrap().clone();
        let skill = SkillProvider::Codex
            .skill_root(home.path())
            .join(&entry.slug);
        fs::create_dir_all(&skill).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            format!(
                "---\nname: {}\ndescription: Personal trust-domain marker\n---\n",
                entry.slug
            ),
        )
        .unwrap();

        let view = catalog(home.path(), store.path()).unwrap();
        let community = view
            .community
            .iter()
            .find(|skill| skill.id == entry.id)
            .unwrap();
        assert!(!community
            .provider_states
            .iter()
            .any(|state| state.installed));
        assert!(community.provider_states.iter().any(|state| state
            .receipt_error
            .as_deref()
            .is_some_and(|error| error.contains("Personal Skill"))));
        assert!(view.personal.iter().any(|skill| skill.name == entry.slug));
        let suggestions = suggestions(
            "trust domain",
            SkillProvider::Codex,
            home.path(),
            store.path(),
        )
        .unwrap();
        assert_eq!(suggestions[0].source, "Personal skill");
        let capabilities = available_capabilities(home.path(), store.path()).unwrap();
        assert!(entry
            .categories
            .iter()
            .all(|category| !capabilities.contains(&format!("skill_category:{category}"))));
    }

    #[cfg(unix)]
    #[test]
    fn personal_discovery_skips_symlink_cycles() {
        use std::os::unix::fs::symlink;
        let root = tempdir().unwrap();
        let valid = root.path().join("valid");
        fs::create_dir_all(&valid).unwrap();
        fs::write(valid.join("SKILL.md"), "---\nname: valid\n---\n").unwrap();
        symlink(root.path(), valid.join("cycle")).unwrap();
        let linked = root.path().join("linked");
        fs::create_dir_all(&linked).unwrap();
        symlink(valid.join("SKILL.md"), linked.join("SKILL.md")).unwrap();
        assert_eq!(collect_skill_files(root.path()).len(), 1);
    }

    #[test]
    fn corrupt_receipt_is_visible_and_actionable() {
        let home = tempdir().unwrap();
        let store = tempdir().unwrap();
        let entry = catalog_entries().unwrap().first().unwrap().clone();
        let path = receipt_path(store.path(), SkillProvider::Codex, &entry.slug);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"{truncated").unwrap();
        let view = catalog(home.path(), store.path()).unwrap();
        let state = view
            .community
            .iter()
            .find(|skill| skill.id == entry.id)
            .unwrap()
            .provider_states
            .iter()
            .find(|state| state.provider == SkillProvider::Codex)
            .unwrap();
        assert!(state
            .receipt_error
            .as_deref()
            .unwrap()
            .contains("unreadable"));
        let error = preview(
            &entry.id,
            SkillAction::Install,
            &[SkillProvider::Codex],
            home.path(),
            store.path(),
            &Mutex::new(HashMap::new()),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("unreadable") && error.contains(&path.display().to_string()));
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
        let entry = catalog_entries().unwrap().first().unwrap().clone();
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
        let entry = catalog_entries().unwrap().first().unwrap().clone();
        let args = installer_args(&entry, SkillProvider::Claude, &entry.pinned_ref);
        assert!(args.iter().any(|value| value.contains(&entry.pinned_ref)));
        assert!(args.iter().any(|value| value == "claude-code"));
        assert!(!args.join(" ").to_lowercase().contains("token="));
    }

    #[test]
    fn missing_managed_skill_is_immediately_ineligible() {
        let home = tempdir().unwrap();
        let store = tempdir().unwrap();
        let entry = catalog_entries().unwrap().first().unwrap().clone();
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
