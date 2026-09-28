use tauri::AppHandle;
use tauri_plugin_updater::UpdaterExt;

const FEED: &str =
    "https://github.com/Atharva-Kanherkar/bridge-harness/releases/download/nightly/latest.json";

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
            install_nightly_update
        ]);
    handler(invoke)
}
