import { useEffect, useState } from "react";
import { SkeletonRows } from "@/components/ui/skeleton";
import { bridgeApi } from "../../api";
import type { AdapterDescriptor } from "../../types";
import type { ReviewerHarnessSettings, ReviewerSettings } from "../../protocol/generated/protocol";
import { SavedFlash, SaveBar, Select, SettingsBlockRow, SettingsGroup, SettingsRow, TextArea, TextButton, useSavedFlash } from "./kit";

/** The harnesses the GitHub pane can hand a review to. Cursor Bugbot is not a
 * local worker and takes no model. */
const REVIEWER_HARNESSES: ReadonlyArray<{ id: string; label: string }> = [
  { id: "claude", label: "Claude" },
  { id: "codex", label: "Codex" },
  { id: "opencode", label: "OpenCode" },
];

const EFFORTS = ["low", "medium", "high", "xhigh"] as const;

/** The wire type leaves both fields optional (serde defaults); the form
 * always holds them. Every other field of the loaded record rides along
 * untouched, so a save never resets a setting this form does not show. */
type FormSettings = ReviewerSettings & { harnesses: NonNullable<ReviewerSettings["harnesses"]>; systemPrompt: string };
const normalize = (settings: ReviewerSettings): FormSettings => ({ ...settings, harnesses: settings.harnesses ?? {}, systemPrompt: settings.systemPrompt ?? "" });

/** Model, effort and instructions for the subagent that reviews pull requests
 * from the GitHub pane. Global rather than per workspace: it is a preference
 * about the reviewer, not about a repository. An empty choice falls through to
 * the Reviewer model profile, then the harness default. */
export function ReviewerSettingsSection({ adapters }: { adapters: AdapterDescriptor[] }) {
  // `stored` is what the host kept; `prompt` is the unsaved instructions text.
  // A select writes the stored record with its harness entry replaced, so a
  // half-written prompt never rides along with it.
  const [stored, setStored] = useState<FormSettings>();
  const [prompt, setPrompt] = useState<string>();
  const [defaultPrompt, setDefaultPrompt] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [isFlashed, flash] = useSavedFlash();
  useEffect(() => {
    let active = true;
    void bridgeApi.reviewerSettings()
      .then(result => { if (active) { setStored(normalize(result.settings)); setDefaultPrompt(result.defaultSystemPrompt); } })
      .catch(cause => { if (active) setError(String(cause)); });
    return () => { active = false; };
  }, []);
  const save = async (next: FormSettings, key: string) => {
    setBusy(true); setError(undefined);
    try {
      const result = await bridgeApi.saveReviewerSettings(next);
      setStored(normalize(result.settings)); setDefaultPrompt(result.defaultSystemPrompt);
      flash(key);
      return true;
    } catch (cause) { setError(String(cause)); return false; }
    finally { setBusy(false); }
  };
  const changeHarness = (id: string, patch: Partial<ReviewerHarnessSettings>, key: string) => {
    if (!stored) return;
    void save({ ...stored, harnesses: { ...stored.harnesses, [id]: { model: null, effort: null, ...stored.harnesses[id], ...patch } } }, key);
  };
  if (error && !stored) return <p role="alert" className="text-ui text-destructive">{error}</p>;
  if (!stored) return <SkeletonRows label="Loading reviewer settings" rows={3} trailing />;
  const draftPrompt = prompt ?? stored.systemPrompt;
  return <div data-reviewer-settings className="space-y-6">
    {error && <p role="alert" className="text-ui text-destructive">{error}</p>}
    <SettingsGroup anchor="reviewer" label="Pull request reviewer" note="What the GitHub pane's Review button launches. A blank model or effort falls back to the Reviewer model profile, then the harness default.">
      {REVIEWER_HARNESSES.map(harness => {
        const adapter = adapters.find(item => item.id === harness.id);
        const entry = stored.harnesses[harness.id] ?? { model: null, effort: null };
        const models = adapter?.models ?? [];
        // A stored model the catalog no longer lists stays selectable rather
        // than silently vanishing from the control.
        const options = [{ value: "", label: "Harness default" }, ...models.map(model => ({ value: model.id, label: model.label }))];
        if (entry.model && !models.some(model => model.id === entry.model)) options.push({ value: entry.model, label: `${entry.model} (not in catalog)` });
        return <SettingsRow key={harness.id} label={harness.label} saved={isFlashed(harness.id)} description={adapter && !adapter.available ? "Unavailable right now" : harness.id === "opencode" ? "Reviews from an isolated worktree behind an approval, because OpenCode cannot run read-only." : undefined} control={<div className="flex flex-wrap items-center justify-end gap-2">
          <Select label={`${harness.label} reviewer model`} disabled={busy} value={entry.model ?? ""} onChange={value => changeHarness(harness.id, { model: value || null }, harness.id)} options={options} />
          <Select label={`${harness.label} reviewer effort`} disabled={busy} width="w-36" value={entry.effort ?? ""} onChange={value => changeHarness(harness.id, { effort: (value || null) as ReviewerHarnessSettings["effort"] }, harness.id)} options={[{ value: "", label: "Profile default" }, ...EFFORTS.map(effort => ({ value: effort, label: effort }))]} />
        </div>} />;
      })}
    </SettingsGroup>
    <SettingsGroup label="Reviewer instructions" note="Leave empty to use Bridge's default review instructions, shown as the placeholder. {number} expands to the pull request number. Every review posts one comment via gh and never approves, merges, or closes.">
      <SettingsBlockRow action={<span className="flex items-center gap-2">{isFlashed("prompt") && <SavedFlash />}<TextButton disabled={busy || draftPrompt === ""} onClick={() => setPrompt("")}>Use default</TextButton></span>}>
        <TextArea label="Reviewer instructions" disabled={busy} value={draftPrompt} placeholder={defaultPrompt} rows={6} onChange={setPrompt} />
      </SettingsBlockRow>
    </SettingsGroup>
    <SaveBar
      dirty={prompt !== undefined && prompt !== stored.systemPrompt}
      saving={busy}
      label="Unsaved reviewer instructions"
      onDiscard={() => setPrompt(undefined)}
      onSave={() => void save({ ...stored, systemPrompt: draftPrompt }, "prompt").then(ok => { if (ok) setPrompt(undefined); })}
    />
  </div>;
}
