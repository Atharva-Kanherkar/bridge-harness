import type { AdapterDescriptor, Harness } from "./types";

export const LAST_DRAFT_MODEL_KEY = "bridge.chat.lastDraftModel";

/** The harness/model a new chat was last opened (or submitted) with, plus the
 *  catalog label for that model at the time. The label is what lets the pill
 *  paint the real name before provider discovery has landed. */
export type LastDraftModel = {
  harness: Harness;
  model: string | null;
  label?: string | null;
};

export function writeLastDraftModel(choice: LastDraftModel): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(LAST_DRAFT_MODEL_KEY, JSON.stringify({
      harness: choice.harness,
      model: choice.model,
      label: choice.label ?? null,
    }));
  } catch {
    // A blocked or full storage is not worth failing a chat over: the next new
    // chat simply resolves its model from the profile again.
  }
}

/** Shape-validated read. Whether the stored harness is still installed, and
 *  whether it still offers the stored model, is the resolver's business. */
export function readLastDraftModel(): LastDraftModel | null {
  if (typeof localStorage === "undefined") return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(localStorage.getItem(LAST_DRAFT_MODEL_KEY) ?? "null");
  } catch {
    return null;
  }
  if (!parsed || typeof parsed !== "object") return null;
  const { harness, model, label } = parsed as Record<string, unknown>;
  if (typeof harness !== "string" || harness.trim() === "") return null;
  if (model != null && typeof model !== "string") return null;
  return {
    harness: harness as Harness,
    model: (model as string | null | undefined) ?? null,
    label: typeof label === "string" ? label : null,
  };
}
