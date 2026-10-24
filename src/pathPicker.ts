import { open, type OpenDialogOptions } from "@tauri-apps/plugin-dialog";
import { backendMode } from "./api";

/**
 * Pick a path on the machine the backend runs on. The desktop opens the native
 * dialog. A browser tab talking to a daemon cannot browse that machine's disk,
 * so it asks for the path to be typed. Mock mode answers with its demo path.
 */
export async function pickPath(options: OpenDialogOptions & { title: string }, demoPath: string): Promise<string | null> {
  const mode = backendMode();
  if (mode === "tauri") {
    const chosen = await open({ ...options, multiple: false });
    return (Array.isArray(chosen) ? chosen[0] : chosen) ?? null;
  }
  if (mode === "web") {
    const typed = window.prompt(`${options.title}\n\nType its full path on the machine running bridged.`);
    return typed?.trim() || null;
  }
  return demoPath;
}
