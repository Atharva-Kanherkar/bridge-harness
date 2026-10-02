const isTauri = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export type UpdateChannel = "stable" | "beta";
export type UpdateInfo = { version: string; currentVersion: string; body: string | null; channel: UpdateChannel };
export class UpdateInstallUnavailableError extends Error {}
export class UpdateCheckSupersededError extends Error {}
let checkGeneration = 0;
const CHANNEL_KEY = "bridge:update-channel";

export function getUpdateChannel(): UpdateChannel {
  try { return window.localStorage.getItem(CHANNEL_KEY) === "beta" ? "beta" : "stable"; }
  catch { return "stable"; }
}

export function setUpdateChannel(channel: UpdateChannel): void {
  window.localStorage.setItem(CHANNEL_KEY, channel);
}

export async function checkForUpdate(channel = getUpdateChannel()): Promise<UpdateInfo | null> {
  const generation = ++checkGeneration;
  if (!isTauri()) return null;
  let result: UpdateInfo | null;
  if (channel === "beta") {
    const { invoke } = await import("@tauri-apps/api/core");
    const update = await invoke<Omit<UpdateInfo, "channel"> | null>("check_nightly_update");
    result = update ? { ...update, channel } : null;
  } else {
    const { check } = await import("@tauri-apps/plugin-updater");
    const update = await check();
    try {
      result = update ? { version: update.version, currentVersion: update.currentVersion, body: update.body ?? null, channel } : null;
    } finally {
      await update?.close();
    }
  }
  // A stale result must not look like "up to date" to callers that clear the toast.
  if (generation !== checkGeneration) throw new UpdateCheckSupersededError();
  return result;
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
    try {
      if (!current || current.version !== update.version) throw new Error("Update changed. Check again before installing.");
      await current.downloadAndInstall();
    } finally {
      // Cleanup failure must not prevent relaunch after a successful install.
      await current?.close().catch(() => undefined);
    }
  }
  const { relaunch } = await import("@tauri-apps/plugin-process");
  await relaunch();
}
