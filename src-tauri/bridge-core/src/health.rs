//! macOS environment checks for `health/health`.
//!
//! Two conditions outside Bridge's process make it look broken while nothing
//! in Bridge is wrong, and both end the same way: macOS file-access prompts
//! that keep coming back.
//!
//! * A registered project or workspace path inside a TCC-protected folder
//!   (`~/Desktop`, `~/Documents`, `~/Downloads`). macOS gates those behind
//!   per-app consent, so Bridge and every agent process it supervises each
//!   trigger their own "would like to access" prompt — and a denied one turns
//!   into silent file-access failures later.
//! * A running binary that is only ad-hoc signed. macOS keys file-access
//!   grants to the app's code-signing identity, and an ad-hoc identity changes
//!   on every rebuild, so grants silently reset and the prompts return.
//!
//! The predicates are pure and host-agnostic so they test everywhere; only
//! the glue that reads the registry and spawns `codesign` is macOS-gated.
//! Classification is deliberately conservative: output we cannot interpret is
//! [`SigningIdentity::Unknown`] and produces no warning, because a wrong
//! "your build is broken" banner costs more trust than a missed one.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// The home-relative folders macOS places behind per-app consent (TCC).
pub const TCC_PROTECTED_FOLDERS: [&str; 3] = ["Desktop", "Documents", "Downloads"];

/// Stable id for the protected-folder warning.
pub const TCC_WARNING_ID: &str = "macos-tcc-protected-path";
/// Stable id for the ad-hoc-signature warning.
pub const ADHOC_WARNING_ID: &str = "macos-adhoc-signature";

/// Where the remediation lives; both warnings point readers here by name.
const README_SECTION: &str = "\u{201c}macOS file access prompts\u{201d} in README.md";

/// One actionable environment warning on the health response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthWarning {
    /// Stable kebab-case id a client may key styling or dismissal on.
    pub id: String,
    pub title: String,
    /// Actionable guidance: the symptom, the cause, and where the fix is.
    pub detail: String,
    /// The offending registered paths; empty when the warning is not about paths.
    pub paths: Vec<String>,
}

/// What `codesign` says about the running binary's identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigningIdentity {
    /// A certificate-backed signature: file-access grants survive rebuilds.
    Stable,
    /// Ad-hoc (including linker-signed): the identity changes every build.
    AdHoc,
    Unsigned,
    /// Output we could not interpret — deliberately never a warning.
    Unknown,
}

/// The protected folder containing `path`, if any.
///
/// Component-wise on purpose: `~/Documents/notes` is inside, `~/DocumentsOld`
/// is not, and a relative path can never match an absolute home.
pub fn tcc_protected_folder(path: &Path, home: &Path) -> Option<&'static str> {
    TCC_PROTECTED_FOLDERS
        .into_iter()
        .find(|folder| path.starts_with(home.join(folder)))
}

/// Classify `codesign --display --verbose` output (codesign details go to
/// stderr; pass whichever stream carried them).
pub fn signing_identity(codesign_output: &str) -> SigningIdentity {
    if codesign_output.contains("code object is not signed at all") {
        return SigningIdentity::Unsigned;
    }
    let lines: Vec<&str> = codesign_output.lines().map(str::trim).collect();
    // `Signature=adhoc` is the direct verdict; the CodeDirectory flags carry
    // the same fact for linker-signed builds (`flags=0x20002(adhoc,linker-signed)`).
    if lines.iter().any(|line| {
        *line == "Signature=adhoc"
            || (line.starts_with("CodeDirectory") && line.contains("(adhoc"))
    }) {
        return SigningIdentity::AdHoc;
    }
    // A certificate chain or a real team is a stable identity. Ad-hoc builds
    // print `TeamIdentifier=not set`, which is excluded above.
    if lines.iter().any(|line| {
        line.starts_with("Authority=")
            || (line.strip_prefix("TeamIdentifier=")
                .is_some_and(|team| !team.is_empty() && team != "not set"))
    }) {
        return SigningIdentity::Stable;
    }
    SigningIdentity::Unknown
}

/// Assemble the warnings from already-gathered facts. Pure so every branch
/// tests off-macOS; deterministic order (paths first) so clients render stably.
pub fn macos_warnings(
    registered_paths: impl IntoIterator<Item = String>,
    home: Option<&Path>,
    signing: SigningIdentity,
) -> Vec<HealthWarning> {
    let mut warnings = Vec::new();
    if let Some(home) = home {
        let mut offending: Vec<String> = registered_paths
            .into_iter()
            .filter(|path| tcc_protected_folder(Path::new(path), home).is_some())
            .collect();
        offending.sort();
        offending.dedup();
        if !offending.is_empty() {
            warnings.push(HealthWarning {
                id: TCC_WARNING_ID.into(),
                title: "Project folders sit inside macOS-protected locations".into(),
                detail: format!(
                    "These registered folders are inside ~/Desktop, ~/Documents, or ~/Downloads, \
                     which macOS gates behind per-app file-access consent. Bridge and the agent \
                     processes it launches can each trigger repeated \u{201c}would like to \
                     access\u{201d} permission prompts, and a denied prompt becomes silent \
                     file-access failures. Move the folders somewhere unprotected such as ~/Code, \
                     or grant Bridge Full Disk Access \u{2014} see {README_SECTION}."
                ),
                paths: offending,
            });
        }
    }
    if matches!(signing, SigningIdentity::AdHoc | SigningIdentity::Unsigned) {
        warnings.push(HealthWarning {
            id: ADHOC_WARNING_ID.into(),
            title: "This build is ad-hoc signed \u{2014} file-access grants reset on every rebuild"
                .into(),
            detail: format!(
                "The running Bridge binary has no stable code-signing identity (codesign reports \
                 an ad-hoc or missing signature). macOS keys Desktop, Documents, and Downloads \
                 access grants to that identity, so every rebuild invalidates them and the \
                 permission prompts come back. Sign development builds with a stable identity, or \
                 expect to re-approve access after each rebuild \u{2014} see {README_SECTION}."
            ),
            paths: Vec::new(),
        });
    }
    warnings
}

/// The macOS environment warnings for `health/health`. Inert off-macOS.
pub fn macos_environment_warnings(core: &crate::BridgeCore) -> Vec<HealthWarning> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = core;
        Vec::new()
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        macos_warnings(
            registered_paths(core),
            home.as_deref(),
            current_exe_signing_identity(),
        )
    }
}

/// Every path the registry ties Bridge to: project roots and workspace paths.
#[cfg(target_os = "macos")]
fn registered_paths(core: &crate::BridgeCore) -> Vec<String> {
    core.state_snapshot()
        .map(|state| {
            state
                .projects
                .into_iter()
                .map(|project| project.path)
                .chain(state.workspaces.into_iter().filter_map(|workspace| workspace.path))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(target_os = "macos")]
fn current_exe_signing_identity() -> SigningIdentity {
    let Ok(exe) = std::env::current_exe() else {
        return SigningIdentity::Unknown;
    };
    // /usr/bin/codesign ships with macOS; details land on stderr, and an
    // unsigned binary reports there with a non-zero status, so read both
    // streams without gating on success.
    let Ok(output) = std::process::Command::new("/usr/bin/codesign")
        .args(["--display", "--verbose"])
        .arg(&exe)
        .output()
    else {
        return SigningIdentity::Unknown;
    };
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    signing_identity(&text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn home() -> PathBuf {
        PathBuf::from("/Users/dev")
    }

    #[test]
    fn a_path_under_a_protected_folder_is_detected_and_names_the_folder() {
        for (path, folder) in [
            ("/Users/dev/Desktop/app", "Desktop"),
            ("/Users/dev/Documents/repos/bridge", "Documents"),
            ("/Users/dev/Downloads", "Downloads"),
        ] {
            assert_eq!(tcc_protected_folder(Path::new(path), &home()), Some(folder), "{path}");
        }
    }

    #[test]
    fn similar_names_siblings_and_relative_paths_are_not_protected() {
        // `starts_with` is component-wise, so a folder merely named like a
        // protected one must not match — that was the easy bug to write.
        for path in [
            "/Users/dev/DesktopBackup/app",
            "/Users/dev/Code/Documents", // protected names only directly under home
            "/Users/dev",
            "/Users/other/Documents/app",
            "Documents/app",
        ] {
            assert_eq!(tcc_protected_folder(Path::new(path), &home()), None, "{path}");
        }
    }

    #[test]
    fn codesign_output_classifies_every_identity_shape() {
        let adhoc = "Executable=/tmp/bridge\nIdentifier=bridge\n\
                     CodeDirectory v=20400 size=568 flags=0x2(adhoc) hashes=12+0 location=embedded\n\
                     Signature=adhoc\nTeamIdentifier=not set\n";
        assert_eq!(signing_identity(adhoc), SigningIdentity::AdHoc);

        // A locally built binary macOS linker-signed is still ad-hoc.
        let linker = "CodeDirectory v=20400 size=568 flags=0x20002(adhoc,linker-signed) \
                      hashes=12+0 location=embedded\nSignature=adhoc\nTeamIdentifier=not set\n";
        assert_eq!(signing_identity(linker), SigningIdentity::AdHoc);

        let developer_id = "Executable=/Applications/Bridge.app/Contents/MacOS/Bridge\n\
                            CodeDirectory v=34816 size=1980 flags=0x10000(runtime) hashes=50+7 location=embedded\n\
                            Signature size=8968\nAuthority=Developer ID Application: Example Corp (ABCDE12345)\n\
                            TeamIdentifier=ABCDE12345\n";
        assert_eq!(signing_identity(developer_id), SigningIdentity::Stable);

        // `codesign -dv` (verbose=1) prints the team without Authority lines.
        let team_only = "CodeDirectory v=34816 size=1980 flags=0x10000(runtime)\n\
                         Signature size=8968\nTeamIdentifier=ABCDE12345\n";
        assert_eq!(signing_identity(team_only), SigningIdentity::Stable);

        let unsigned = "/tmp/bridge: code object is not signed at all";
        assert_eq!(signing_identity(unsigned), SigningIdentity::Unsigned);

        assert_eq!(signing_identity(""), SigningIdentity::Unknown);
        assert_eq!(signing_identity("some unexpected output"), SigningIdentity::Unknown);
    }

    #[test]
    fn offending_paths_aggregate_into_one_sorted_deduplicated_warning() {
        let warnings = macos_warnings(
            [
                "/Users/dev/Documents/b".to_owned(),
                "/Users/dev/Code/safe".to_owned(),
                "/Users/dev/Desktop/a".to_owned(),
                // A workspace sharing its project's path must not list twice.
                "/Users/dev/Documents/b".to_owned(),
            ],
            Some(&home()),
            SigningIdentity::Stable,
        );
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].id, TCC_WARNING_ID);
        assert_eq!(warnings[0].paths, ["/Users/dev/Desktop/a", "/Users/dev/Documents/b"]);
    }

    #[test]
    fn adhoc_and_unsigned_warn_while_stable_and_unknown_stay_silent() {
        for signing in [SigningIdentity::AdHoc, SigningIdentity::Unsigned] {
            let warnings = macos_warnings([], Some(&home()), signing);
            assert_eq!(warnings.len(), 1, "{signing:?}");
            assert_eq!(warnings[0].id, ADHOC_WARNING_ID);
            assert!(warnings[0].paths.is_empty());
        }
        for signing in [SigningIdentity::Stable, SigningIdentity::Unknown] {
            assert!(macos_warnings([], Some(&home()), signing).is_empty(), "{signing:?}");
        }
    }

    #[test]
    fn both_warnings_explain_the_prompt_symptom_and_point_at_the_readme() {
        let warnings = macos_warnings(
            ["/Users/dev/Downloads/repo".to_owned()],
            Some(&home()),
            SigningIdentity::AdHoc,
        );
        assert_eq!(
            warnings.iter().map(|warning| warning.id.as_str()).collect::<Vec<_>>(),
            [TCC_WARNING_ID, ADHOC_WARNING_ID],
            "deterministic order: paths first"
        );
        for warning in &warnings {
            assert!(warning.detail.contains("prompt"), "{}: names the symptom", warning.id);
            assert!(
                warning.detail.contains("macOS file access prompts")
                    && warning.detail.contains("README.md"),
                "{}: points at the README section",
                warning.id
            );
        }
    }

    #[test]
    fn no_home_and_clean_facts_produce_no_warnings() {
        assert!(macos_warnings(
            ["/Users/dev/Documents/app".to_owned()],
            None,
            SigningIdentity::Stable
        )
        .is_empty());
        assert!(macos_warnings([], Some(&home()), SigningIdentity::Stable).is_empty());
    }
}
