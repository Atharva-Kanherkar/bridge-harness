const isTauri = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export type UpdateInfo = { version: string; currentVersion: string; body: string | null };

let cachedUpdate: import("@tauri-apps/plugin-updater").Update | null = null;

export async function checkForUpdate(): Promise<UpdateInfo | null> {
  if (!isTauri()) return null;
  const { check } = await import("@tauri-apps/plugin-updater");
  const update = await check();
  if (!update) return null;
  cachedUpdate = update;
  return { version: update.version, currentVersion: update.currentVersion, body: update.body ?? null };
}

export async function installUpdateAndRestart(): Promise<void> {
  if (!cachedUpdate) return;
  await cachedUpdate.downloadAndInstall();
  const { relaunch } = await import("@tauri-apps/plugin-process");
  await relaunch();
}
