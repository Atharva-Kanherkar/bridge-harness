use crate::{
    delegation::{Effort, WorkerRole},
    model::{AdapterDescriptor, CapabilityTier},
    BridgeError,
};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const PROFILE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ProfilePurpose {
    StandardOrchestrator,
    PremiumOrchestrator,
    Planner,
    Implementer,
    Verifier,
    Reviewer,
    Research,
    Documentation,
    Evaluator,
}

impl ProfilePurpose {
    pub const ALL: [Self; 9] = [
        Self::StandardOrchestrator,
        Self::PremiumOrchestrator,
        Self::Planner,
        Self::Implementer,
        Self::Verifier,
        Self::Reviewer,
        Self::Research,
        Self::Documentation,
        Self::Evaluator,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::StandardOrchestrator => "standard_orchestrator",
            Self::PremiumOrchestrator => "premium_orchestrator",
            Self::Planner => "planner",
            Self::Implementer => "implementer",
            Self::Verifier => "verifier",
            Self::Reviewer => "reviewer",
            Self::Research => "research",
            Self::Documentation => "documentation",
            Self::Evaluator => "evaluator",
        }
    }

    pub fn canonical_role(self) -> WorkerRole {
        match self {
            Self::StandardOrchestrator | Self::PremiumOrchestrator | Self::Planner => {
                WorkerRole::Planning
            }
            Self::Implementer => WorkerRole::Implementation,
            Self::Verifier | Self::Reviewer | Self::Evaluator => WorkerRole::Verification,
            Self::Research => WorkerRole::Research,
            Self::Documentation => WorkerRole::Documentation,
        }
    }

    fn tier(self) -> CapabilityTier {
        match self {
            Self::PremiumOrchestrator | Self::Planner | Self::Reviewer | Self::Evaluator => {
                CapabilityTier::Strong
            }
            Self::StandardOrchestrator | Self::Implementer | Self::Verifier | Self::Research => {
                CapabilityTier::Standard
            }
            Self::Documentation => CapabilityTier::Fast,
        }
    }

    fn effort(self) -> Effort {
        match self.tier() {
            CapabilityTier::Fast => Effort::Low,
            CapabilityTier::Standard => Effort::Medium,
            CapabilityTier::Strong => Effort::High,
        }
    }

    fn fallback(self) -> Option<Self> {
        match self {
            Self::StandardOrchestrator => None,
            Self::PremiumOrchestrator | Self::Planner => Some(Self::StandardOrchestrator),
            Self::Implementer | Self::Research | Self::Documentation => {
                Some(Self::StandardOrchestrator)
            }
            Self::Verifier | Self::Reviewer | Self::Evaluator => Some(Self::Verifier),
        }
        .filter(|fallback| *fallback != self)
    }

    pub fn for_worker_role(role: WorkerRole) -> Self {
        match role {
            WorkerRole::Research => Self::Research,
            WorkerRole::Implementation => Self::Implementer,
            WorkerRole::Verification => Self::Verifier,
            WorkerRole::Planning => Self::Planner,
            WorkerRole::Documentation => Self::Documentation,
        }
    }
}

fn purpose_from_str(value: &str) -> Result<ProfilePurpose, BridgeError> {
    ProfilePurpose::ALL
        .into_iter()
        .find(|purpose| purpose.as_str() == value)
        .ok_or_else(|| BridgeError::Invalid(format!("unknown model profile purpose {value}")))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelProfileDraft {
    pub purpose: ProfilePurpose,
    pub provider: String,
    pub model: String,
    pub effort: Effort,
    #[serde(default)]
    pub fallback_purpose: Option<ProfilePurpose>,
    pub pinned: bool,
    pub learning_enabled: bool,
    #[serde(default)]
    pub budget_preference: Option<String>,
    #[serde(default)]
    pub latency_preference: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelProfile {
    pub schema_version: u32,
    pub version: i64,
    pub profile_id: String,
    pub purpose: ProfilePurpose,
    pub canonical_role: WorkerRole,
    pub provider: String,
    pub model: String,
    pub effort: Effort,
    pub fallback_purpose: Option<ProfilePurpose>,
    pub pinned: bool,
    pub learning_enabled: bool,
    pub budget_preference: Option<String>,
    pub latency_preference: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelSetupState {
    pub complete: bool,
    pub active_version: Option<i64>,
    pub profiles: Vec<ModelProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedProfile {
    pub purpose: ProfilePurpose,
    pub profile_version: i64,
    pub provider: String,
    pub model: String,
    pub tier: CapabilityTier,
    pub effort: Effort,
    pub pinned: bool,
    pub learning_enabled: bool,
    pub budget_preference: Option<String>,
    pub latency_preference: Option<String>,
    pub used_fallback: bool,
}

fn catalog_default(
    descriptors: &[AdapterDescriptor],
    tier: CapabilityTier,
) -> Option<(&AdapterDescriptor, &crate::model::ModelOption)> {
    descriptors
        .iter()
        .filter(|descriptor| descriptor.available)
        .flat_map(|descriptor| {
            descriptor
                .models
                .iter()
                .filter(move |model| model.tier == tier && model.default_for_tier)
                .map(move |model| (descriptor, model))
        })
        .next()
        .or_else(|| {
            descriptors
                .iter()
                .filter(|descriptor| descriptor.available)
                .flat_map(|descriptor| {
                    descriptor
                        .models
                        .iter()
                        .filter(move |model| model.tier == tier)
                        .map(move |model| (descriptor, model))
                })
                .next()
        })
}

pub fn recommended_profiles(
    descriptors: &[AdapterDescriptor],
) -> Result<Vec<ModelProfileDraft>, BridgeError> {
    ProfilePurpose::ALL
        .into_iter()
        .map(|purpose| {
            let (adapter, model) =
                catalog_default(descriptors, purpose.tier()).ok_or_else(|| {
                    BridgeError::Invalid(format!(
                        "no available adapter advertises a {} default for {}",
                        purpose.tier().as_str(),
                        purpose.as_str()
                    ))
                })?;
            Ok(ModelProfileDraft {
                purpose,
                provider: adapter.id.clone(),
                model: model.id.clone(),
                effort: purpose.effort(),
                fallback_purpose: purpose.fallback(),
                pinned: false,
                learning_enabled: true,
                budget_preference: None,
                latency_preference: None,
            })
        })
        .collect()
}

fn validate_profiles(
    profiles: &[ModelProfileDraft],
    descriptors: &[AdapterDescriptor],
) -> Result<(), BridgeError> {
    let by_purpose = profiles
        .iter()
        .map(|profile| (profile.purpose, profile))
        .collect::<BTreeMap<_, _>>();
    if by_purpose.len() != ProfilePurpose::ALL.len()
        || ProfilePurpose::ALL
            .iter()
            .any(|purpose| !by_purpose.contains_key(purpose))
    {
        return Err(BridgeError::Invalid(
            "model setup must contain exactly one profile for every purpose".into(),
        ));
    }
    for profile in profiles {
        if profile.provider.trim().is_empty() || profile.model.trim().is_empty() {
            return Err(BridgeError::Invalid(
                "model profile provider and model cannot be empty".into(),
            ));
        }
        let supported = descriptors.iter().any(|descriptor| {
            descriptor.available
                && descriptor.id == profile.provider
                && descriptor
                    .models
                    .iter()
                    .any(|model| model.id == profile.model)
        });
        if !supported {
            return Err(BridgeError::Invalid(format!(
                "{}:{} is not available in the live adapter catalog",
                profile.provider, profile.model
            )));
        }
        if profile
            .fallback_purpose
            .is_some_and(|fallback| fallback == profile.purpose)
        {
            return Err(BridgeError::Invalid(format!(
                "{} cannot fall back to itself",
                profile.purpose.as_str()
            )));
        }
        if profile
            .budget_preference
            .as_deref()
            .is_some_and(|value| !matches!(value, "economy" | "quality"))
        {
            return Err(BridgeError::Invalid(
                "profile budget preference must be economy or quality".into(),
            ));
        }
        if profile
            .latency_preference
            .as_deref()
            .is_some_and(|value| !matches!(value, "fast" | "patient"))
        {
            return Err(BridgeError::Invalid(
                "profile latency preference must be fast or patient".into(),
            ));
        }
    }
    for purpose in ProfilePurpose::ALL {
        let mut seen = BTreeSet::new();
        let mut cursor = Some(purpose);
        while let Some(current) = cursor {
            if !seen.insert(current) {
                return Err(BridgeError::Invalid(format!(
                    "fallback cycle detected from {}",
                    purpose.as_str()
                )));
            }
            cursor = by_purpose
                .get(&current)
                .and_then(|profile| profile.fallback_purpose);
        }
    }
    Ok(())
}

pub fn setup_state(db: &Connection) -> Result<ModelSetupState, BridgeError> {
    let active_version: Option<i64> = db
        .query_row(
            "SELECT active_version FROM model_setup_state WHERE id='default'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let profiles = active_version
        .map(|version| profiles_at_version(db, version))
        .transpose()?
        .unwrap_or_default();
    Ok(ModelSetupState {
        complete: active_version.is_some() && profiles.len() == ProfilePurpose::ALL.len(),
        active_version,
        profiles,
    })
}

fn profiles_at_version(db: &Connection, version: i64) -> Result<Vec<ModelProfile>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT version,purpose,canonical_role,provider,model,effort,fallback_purpose,pinned,learning_enabled,budget_preference,latency_preference,created_at
         FROM model_profiles WHERE version=?1 ORDER BY purpose",
    )?;
    let profiles = statement.query_map(params![version], |row| {
        let purpose = purpose_from_str(&row.get::<_, String>(1)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        let canonical_role: WorkerRole =
            serde_json::from_str(&format!("\"{}\"", row.get::<_, String>(2)?)).map_err(
                |error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                },
            )?;
        let effort: Effort = serde_json::from_str(&format!("\"{}\"", row.get::<_, String>(5)?))
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    5,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
        let fallback = row
            .get::<_, Option<String>>(6)?
            .map(|value| purpose_from_str(&value))
            .transpose()
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    6,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
        Ok(ModelProfile {
            schema_version: PROFILE_SCHEMA_VERSION,
            version: row.get(0)?,
            profile_id: purpose.as_str().into(),
            purpose,
            canonical_role,
            provider: row.get(3)?,
            model: row.get(4)?,
            effort,
            fallback_purpose: fallback,
            pinned: row.get(7)?,
            learning_enabled: row.get(8)?,
            budget_preference: row.get(9)?,
            latency_preference: row.get(10)?,
            created_at: row.get(11)?,
        })
    })?;
    profiles
        .collect::<Result<Vec<_>, _>>()
        .map_err(BridgeError::from)
}

pub fn save_profiles(
    db: &Connection,
    descriptors: &[AdapterDescriptor],
    profiles: &[ModelProfileDraft],
) -> Result<ModelSetupState, BridgeError> {
    validate_profiles(profiles, descriptors)?;
    let transaction = db.unchecked_transaction()?;
    let version: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(version),0)+1 FROM model_profiles",
        [],
        |row| row.get(0),
    )?;
    let now = Utc::now().to_rfc3339();
    for profile in profiles {
        transaction.execute(
            "INSERT INTO model_profiles(version,profile_id,purpose,canonical_role,provider,model,effort,fallback_purpose,pinned,learning_enabled,budget_preference,latency_preference,created_at)
             VALUES(?1,?2,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                version,
                profile.purpose.as_str(),
                serde_json::to_value(profile.purpose.canonical_role())
                    .map_err(|error| BridgeError::Invalid(error.to_string()))?
                    .as_str()
                    .unwrap_or_default(),
                profile.provider,
                profile.model,
                profile.effort.as_str(),
                profile.fallback_purpose.map(ProfilePurpose::as_str),
                profile.pinned,
                profile.learning_enabled,
                profile.budget_preference,
                profile.latency_preference,
                now,
            ],
        )?;
    }
    transaction.execute(
        "INSERT INTO model_setup_state(id,active_version,updated_at) VALUES('default',?1,?2)
         ON CONFLICT(id) DO UPDATE SET active_version=excluded.active_version,updated_at=excluded.updated_at",
        params![version, now],
    )?;
    transaction.commit()?;
    setup_state(db)
}

pub fn reset_profiles(
    db: &Connection,
    descriptors: &[AdapterDescriptor],
) -> Result<ModelSetupState, BridgeError> {
    let profiles = recommended_profiles(descriptors)?;
    save_profiles(db, descriptors, &profiles)
}

pub fn resolve_for_role(
    db: &Connection,
    descriptors: &[AdapterDescriptor],
    role: WorkerRole,
) -> Result<Option<ResolvedProfile>, BridgeError> {
    resolve_profile(db, descriptors, ProfilePurpose::for_worker_role(role))
}

pub fn resolve_profile(
    db: &Connection,
    descriptors: &[AdapterDescriptor],
    purpose: ProfilePurpose,
) -> Result<Option<ResolvedProfile>, BridgeError> {
    let state = setup_state(db)?;
    let Some(version) = state.active_version else {
        return Ok(None);
    };
    let by_purpose = state
        .profiles
        .into_iter()
        .map(|profile| (profile.purpose, profile))
        .collect::<BTreeMap<_, _>>();
    let mut cursor = Some(purpose);
    let mut seen = BTreeSet::new();
    while let Some(current) = cursor {
        if !seen.insert(current) {
            break;
        }
        let Some(profile) = by_purpose.get(&current) else {
            break;
        };
        if let Some((descriptor, model)) = descriptors.iter().find_map(|descriptor| {
            (descriptor.available && descriptor.id == profile.provider).then(|| {
                descriptor
                    .models
                    .iter()
                    .find(|model| model.id == profile.model)
                    .map(|model| (descriptor, model))
            })?
        }) {
            return Ok(Some(ResolvedProfile {
                purpose,
                profile_version: version,
                provider: descriptor.id.clone(),
                model: model.id.clone(),
                tier: model.tier,
                effort: profile.effort,
                pinned: profile.pinned,
                learning_enabled: profile.learning_enabled,
                budget_preference: profile.budget_preference.clone(),
                latency_preference: profile.latency_preference.clone(),
                used_fallback: current != purpose,
            }));
        }
        cursor = profile.fallback_purpose;
    }
    Ok(
        catalog_default(descriptors, purpose.tier()).map(|(descriptor, model)| ResolvedProfile {
            purpose,
            profile_version: version,
            provider: descriptor.id.clone(),
            model: model.id.clone(),
            tier: model.tier,
            effort: purpose.effort(),
            pinned: false,
            learning_enabled: true,
            budget_preference: None,
            latency_preference: None,
            used_fallback: true,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{model::ModelOption, store};

    fn catalog() -> Vec<AdapterDescriptor> {
        vec![AdapterDescriptor {
            sandbox_modes: crate::model::SandboxMode::ALL.to_vec(),
            id: "catalog-provider".into(),
            label: "Catalog Provider".into(),
            available: true,
            auth_state: crate::model::AuthState::Unknown,
            version: Some("1".into()),
            capabilities: vec!["tools".into()],
            unavailable_reason: None,
            models: vec![
                ModelOption {
                    id: "fast-default".into(),
                    label: "Fast".into(),
                    tier: CapabilityTier::Fast,
                    default_for_tier: true,
                },
                ModelOption {
                    id: "standard-default".into(),
                    label: "Standard".into(),
                    tier: CapabilityTier::Standard,
                    default_for_tier: true,
                },
                ModelOption {
                    id: "strong-default".into(),
                    label: "Strong".into(),
                    tier: CapabilityTier::Strong,
                    default_for_tier: true,
                },
            ],
            default_model: Some("standard-default".into()),
        }]
    }

    #[test]
    fn recommended_profiles_use_catalog_defaults() {
        let profiles = recommended_profiles(&catalog()).unwrap();
        assert_eq!(profiles.len(), ProfilePurpose::ALL.len());
        assert!(profiles
            .iter()
            .all(|profile| profile.provider == "catalog-provider"));
        assert!(profiles
            .iter()
            .all(|profile| profile.model.ends_with("-default")));
    }

    #[test]
    fn review_purposes_map_to_verification() {
        assert_eq!(
            ProfilePurpose::Reviewer.canonical_role(),
            WorkerRole::Verification
        );
        assert_eq!(
            ProfilePurpose::Evaluator.canonical_role(),
            WorkerRole::Verification
        );
        assert_eq!(
            ProfilePurpose::Verifier.canonical_role(),
            WorkerRole::Verification
        );
    }

    #[test]
    fn save_profiles_versions_immutably() {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        let first = reset_profiles(&db, &catalog()).unwrap();
        let second = reset_profiles(&db, &catalog()).unwrap();
        assert_eq!(first.active_version, Some(1));
        assert_eq!(second.active_version, Some(2));
        assert_eq!(
            profiles_at_version(&db, 1).unwrap().len(),
            ProfilePurpose::ALL.len()
        );
        assert_eq!(
            profiles_at_version(&db, 2).unwrap().len(),
            ProfilePurpose::ALL.len()
        );
    }

    #[test]
    fn resolve_profile_falls_back_when_model_disappears() {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        reset_profiles(&db, &catalog()).unwrap();
        let changed = vec![AdapterDescriptor {
            models: vec![
                ModelOption {
                    id: "fast-v2".into(),
                    label: "Fast v2".into(),
                    tier: CapabilityTier::Fast,
                    default_for_tier: true,
                },
                ModelOption {
                    id: "standard-v2".into(),
                    label: "Standard v2".into(),
                    tier: CapabilityTier::Standard,
                    default_for_tier: true,
                },
                ModelOption {
                    id: "strong-v2".into(),
                    label: "Strong v2".into(),
                    tier: CapabilityTier::Strong,
                    default_for_tier: true,
                },
            ],
            ..catalog().remove(0)
        }];
        let resolved = resolve_profile(&db, &changed, ProfilePurpose::Implementer)
            .unwrap()
            .unwrap();
        assert_eq!(resolved.model, "standard-v2");
        assert!(resolved.used_fallback);
    }

    #[test]
    fn rejects_unsupported_or_recursive_profiles() {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        let mut profiles = recommended_profiles(&catalog()).unwrap();
        profiles[0].model = "invented".into();
        assert!(save_profiles(&db, &catalog(), &profiles).is_err());
        let mut profiles = recommended_profiles(&catalog()).unwrap();
        profiles[0].fallback_purpose = Some(profiles[1].purpose);
        profiles[1].fallback_purpose = Some(profiles[0].purpose);
        assert!(save_profiles(&db, &catalog(), &profiles).is_err());
    }
}
