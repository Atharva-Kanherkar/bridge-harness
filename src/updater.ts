const isTauri = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export type UpdateInfo = { version: string; currentVersion: string; body: string | null };
export type UpdateChannel = "stable" | "beta";
const CHANNEL_KEY = "bridge:update-channel";

export function getUpdateChannel(): UpdateChannel {
  try { return window.localStorage.getItem(CHANNEL_KEY) === "beta" ? "beta" : "stable"; }
  catch { return "stable"; }
}

export function setUpdateChannel(channel: UpdateChannel): void {
  window.localStorage.setItem(CHANNEL_KEY, channel);
}

let cachedUpdate: import("@tauri-apps/plugin-updater").Update | null = null;
let cachedNightlyVersion: string | null = null;
let checkGeneration = 0;

export async function checkForUpdate(channel = getUpdateChannel()): Promise<UpdateInfo | null> {
  if (!isTauri()) return null;
  const generation = ++checkGeneration;
  cachedUpdate = null;
  cachedNightlyVersion = null;
  if (channel === "beta") {
    const { invoke } = await import("@tauri-apps/api/core");
    const update = await invoke<UpdateInfo | null>("check_nightly_update");
    if (generation !== checkGeneration) return null;
    cachedNightlyVersion = update?.version ?? null;
    return update;
  }
  const { check } = await import("@tauri-apps/plugin-updater");
  const update = await check();
  if (generation !== checkGeneration) return null;
  if (!update) return null;
  cachedUpdate = update;
  return { version: update.version, currentVersion: update.currentVersion, body: update.body ?? null };
}

export async function installUpdateAndRestart(): Promise<void> {
  if (cachedNightlyVersion) {
    const { invoke } = await import("@tauri-apps/api/core");
    await invoke("install_nightly_update", { version: cachedNightlyVersion });
  } else if (cachedUpdate) {
    await cachedUpdate.downloadAndInstall();
  } else return;
  const { relaunch } = await import("@tauri-apps/plugin-process");
  await relaunch();
}
