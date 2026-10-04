use tauri::AppHandle;
use tauri_plugin_updater::UpdaterExt;

#[cfg(target_os = "macos")]
use std::path::Path;

const FEED: &str =
    "https://github.com/Atharva-Kanherkar/bridge-harness/releases/download/nightly/latest.json";
const DEVELOPMENT_INSTALL_ERROR: &str =
    "Install updates from a packaged Bridge app. This development build cannot replace itself safely.";

#[cfg(target_os = "macos")]
fn is_packaged_macos_executable(executable: &Path) -> bool {
    let Some(macos) = executable.parent() else {
        return false;
    };
    let Some(contents) = macos.parent() else {
        return false;
    };
    let Some(bundle) = contents.parent() else {
        return false;
    };
    macos.file_name().is_some_and(|name| name == "MacOS")
        && contents.file_name().is_some_and(|name| name == "Contents")
        && bundle
            .extension()
            .is_some_and(|extension| extension == "app")
}

#[tauri::command]
fn ensure_update_installable() -> Result<(), String> {
    if tauri::is_dev() {
        return Err(DEVELOPMENT_INSTALL_ERROR.into());
    }
    #[cfg(target_os = "macos")]
    {
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        if !is_packaged_macos_executable(&executable) {
            return Err(DEVELOPMENT_INSTALL_ERROR.into());
        }
    }
    Ok(())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NightlyUpdate {
    version: String,
    current_version: String,
    body: Option<String>,
}

async fn check(app: &AppHandle) -> Result<Option<tauri_plugin_updater::Update>, String> {
    let endpoint = FEED
        .parse()
        .map_err(|error| format!("Invalid nightly feed: {error}"))?;
    let updater = app
        .updater_builder()
        .endpoints(vec![endpoint])
        .map_err(|error| error.to_string())?
        .build()
        .map_err(|error| error.to_string())?;
    updater.check().await.map_err(|error| match error {
        tauri_plugin_updater::Error::ReleaseNotFound => {
            "The beta nightly feed is unavailable. A nightly build may not be published yet."
                .to_string()
        }
        other => other.to_string(),
    })
}

#[tauri::command]
async fn check_nightly_update(app: AppHandle) -> Result<Option<NightlyUpdate>, String> {
    Ok(check(&app).await?.map(|update| NightlyUpdate {
        version: update.version,
        current_version: update.current_version,
        body: update.body,
    }))
}

#[tauri::command]
async fn install_nightly_update(app: AppHandle, version: String) -> Result<(), String> {
    ensure_update_installable()?;
    let update = check(&app)
        .await?
        .ok_or("Nightly update is no longer available")?;
    if update.version != version {
        return Err("Nightly update changed; check again before installing".into());
    }
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|error| error.to_string())
}

pub fn commands(invoke: tauri::ipc::Invoke<tauri::Wry>) -> bool {
    let handler: Box<dyn Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync> =
        Box::new(tauri::generate_handler![
            check_nightly_update,
            install_nightly_update,
            ensure_update_installable
        ]);
    handler(invoke)
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::is_packaged_macos_executable;
    use std::path::Path;

    #[test]
    fn updater_target_must_be_an_app_bundle() {
        assert!(is_packaged_macos_executable(Path::new(
            "/Applications/Bridge.app/Contents/MacOS/bridge-deck"
        )));
        assert!(!is_packaged_macos_executable(Path::new(
            "/workspace/src-tauri/target/debug/bridge-deck"
        )));
        assert!(!is_packaged_macos_executable(Path::new(
            "/workspace/Contents/MacOS/bridge-deck"
        )));
    }
}
