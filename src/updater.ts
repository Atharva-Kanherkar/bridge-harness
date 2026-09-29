const isTauri = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export type UpdateChannel = "stable" | "beta";
export type UpdateInfo = { version: string; currentVersion: string; body: string | null; channel: UpdateChannel };
export class UpdateInstallUnavailableError extends Error {}
const CHANNEL_KEY = "bridge:update-channel";

export function getUpdateChannel(): UpdateChannel {
  try { return window.localStorage.getItem(CHANNEL_KEY) === "beta" ? "beta" : "stable"; }
  catch { return "stable"; }
}

export function setUpdateChannel(channel: UpdateChannel): void {
  window.localStorage.setItem(CHANNEL_KEY, channel);
}

export async function checkForUpdate(channel = getUpdateChannel()): Promise<UpdateInfo | null> {
  if (!isTauri()) return null;
  if (channel === "beta") {
    const { invoke } = await import("@tauri-apps/api/core");
    const update = await invoke<Omit<UpdateInfo, "channel"> | null>("check_nightly_update");
    return update ? { ...update, channel } : null;
  }
  const { check } = await import("@tauri-apps/plugin-updater");
  const update = await check();
  if (!update) return null;
  return { version: update.version, currentVersion: update.currentVersion, body: update.body ?? null, channel };
}

export async function installUpdateAndRestart(update: UpdateInfo): Promise<void> {
  const { invoke } = await import("@tauri-apps/api/core");
  try { await invoke("ensure_update_installable"); }
  catch (error) {
    const message = String(error);
    if (message.includes("development build cannot replace itself safely")) throw new UpdateInstallUnavailableError(message);
    throw error;
  }
  if (update.channel === "beta") {
    await invoke("install_nightly_update", { version: update.version });
  } else {
    const { check } = await import("@tauri-apps/plugin-updater");
    const current = await check();
    if (!current || current.version !== update.version) throw new Error("Update changed. Check again before installing.");
    await current.downloadAndInstall();
  }
  const { relaunch } = await import("@tauri-apps/plugin-process");
  await relaunch();
}
