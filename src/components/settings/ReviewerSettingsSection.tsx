import { useEffect, useState } from "react";
import { bridgeApi } from "../../api";
import type { AdapterDescriptor } from "../../types";
import type { ReviewerHarnessSettings, ReviewerSettings } from "../../protocol/generated/protocol";
import { GhostButton, Select, SettingsGroup, SettingsRow, TextArea } from "./kit";

/** The harnesses the GitHub pane can hand a review to. Cursor Bugbot is not a
 * local worker and takes no model. */
const REVIEWER_HARNESSES: ReadonlyArray<{ id: string; label: string }> = [
  { id: "claude", label: "Claude" },
  { id: "codex", label: "Codex" },
  { id: "opencode", label: "OpenCode" },
];

const EFFORTS = ["low", "medium", "high", "xhigh"] as const;

/** The wire type leaves both fields optional (serde defaults); the form
 * always holds them. */
type FormSettings = { harnesses: NonNullable<ReviewerSettings["harnesses"]>; systemPrompt: string };
const normalize = (settings: ReviewerSettings): FormSettings => ({ harnesses: settings.harnesses ?? {}, systemPrompt: settings.systemPrompt ?? "" });

/** Model, effort and instructions for the subagent that reviews pull requests
 * from the GitHub pane. Global rather than per workspace: it is a preference
 * about the reviewer, not about a repository. An empty choice falls through to
 * the Reviewer model profile, then the harness default. */
export function ReviewerSettingsSection({ adapters }: { adapters: AdapterDescriptor[] }) {
  const [settings, setSettings] = useState<FormSettings>();
  const [defaultPrompt, setDefaultPrompt] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [saved, setSaved] = useState(false);
  useEffect(() => {
    let active = true;
    void bridgeApi.reviewerSettings()
      .then(result => { if (active) { setSettings(normalize(result.settings)); setDefaultPrompt(result.defaultSystemPrompt); } })
      .catch(cause => { if (active) setError(String(cause)); });
    return () => { active = false; };
  }, []);
  const save = async () => {
    if (!settings) return;
    setBusy(true); setError(undefined); setSaved(false);
    try {
      const result = await bridgeApi.saveReviewerSettings(settings);
      setSettings(normalize(result.settings)); setDefaultPrompt(result.defaultSystemPrompt); setSaved(true);
    } catch (cause) { setError(String(cause)); }
    finally { setBusy(false); }
  };
  const changeHarness = (id: string, patch: Partial<ReviewerHarnessSettings>) => {
    setSaved(false);
    setSettings(current => current
      ? { ...current, harnesses: { ...current.harnesses, [id]: { model: null, effort: null, ...current.harnesses[id], ...patch } } }
      : current);
  };
  const changePrompt = (systemPrompt: string) => { setSaved(false); setSettings(current => current ? { ...current, systemPrompt } : current); };
  if (error && !settings) return <p role="alert" className="text-sm text-destructive">{error}</p>;
  if (!settings) return <p role="status" className="text-sm text-muted-foreground">Loading reviewer settings...</p>;
  return <form data-reviewer-settings onSubmit={event => { event.preventDefault(); void save(); }} className="space-y-5">
    {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
    <SettingsGroup label="Pull request reviewer" note="What the GitHub pane's Review button launches. A blank model or effort falls back to the Reviewer model profile, then the harness default. OpenCode cannot run read-only, so its reviewer works from an isolated worktree behind an approval.">
      {REVIEWER_HARNESSES.map(harness => {
        const adapter = adapters.find(item => item.id === harness.id);
        const entry = settings.harnesses[harness.id] ?? { model: null, effort: null };
        const models = adapter?.models ?? [];
        // A stored model the catalog no longer lists stays selectable rather
        // than silently vanishing from the control.
        const options = [{ value: "", label: "Harness default" }, ...models.map(model => ({ value: model.id, label: model.label }))];
        if (entry.model && !models.some(model => model.id === entry.model)) options.push({ value: entry.model, label: `${entry.model} (not in catalog)` });
        return <SettingsRow key={harness.id} label={harness.label} description={adapter && !adapter.available ? "Unavailable right now" : undefined} control={<div className="flex flex-wrap items-center justify-end gap-2">
          <Select label={`${harness.label} reviewer model`} disabled={busy} value={entry.model ?? ""} onChange={value => changeHarness(harness.id, { model: value || null })} options={options} />
          <Select label={`${harness.label} reviewer effort`} disabled={busy} width="w-36" value={entry.effort ?? ""} onChange={value => changeHarness(harness.id, { effort: (value || null) as ReviewerHarnessSettings["effort"] })} options={[{ value: "", label: "Profile default" }, ...EFFORTS.map(effort => ({ value: effort, label: effort }))]} />
        </div>} />;
      })}
    </SettingsGroup>
    <SettingsGroup label="Reviewer instructions" note="Leave empty to use Bridge's default review instructions, shown as the placeholder. {number} expands to the pull request number. Claude and Codex review read-only; OpenCode reviews from an isolated worktree behind an approval. Every review posts one comment via gh and never approves, merges, or closes — custom instructions keep that guardrail.">
      <div className="px-3 py-2.5">
        <TextArea label="Reviewer instructions" disabled={busy} value={settings.systemPrompt} placeholder={defaultPrompt} rows={6} onChange={changePrompt} />
      </div>
    </SettingsGroup>
    <div className="flex flex-wrap items-center gap-3">
      <button type="submit" disabled={busy} className="rounded-lg bg-primary px-4 py-2 text-xs font-medium text-primary-foreground disabled:opacity-50">{busy ? "Saving..." : "Save reviewer settings"}</button>
      {saved && <span role="status" className="text-xs text-muted-foreground">Saved</span>}
      <GhostButton disabled={busy || settings.systemPrompt === ""} onClick={() => changePrompt("")}>Reset instructions to default</GhostButton>
    </div>
  </form>;
}
