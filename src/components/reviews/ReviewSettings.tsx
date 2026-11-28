import { useEffect, useState } from "react";
import { SkeletonRows } from "@/components/ui/skeleton";
import { bridgeApi } from "../../api";
import { friendlyError } from "../../errors";
import type { AdapterDescriptor } from "../../types";
import type { ReviewerHarnessSettings, ReviewerSettings, ReviewJobSettings } from "../../protocol/generated/protocol";
import { NumberField, SavedFlash, SaveBar, Segmented, Select, SettingsBlockRow, SettingsGroup, SettingsRow, Switch, TextArea, TextButton, useSavedFlash } from "../settings/kit";

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

/** The job settings with every field the form edits filled in. */
type JobForm = Required<Pick<ReviewJobSettings, "defaultHarnesses" | "fallbackHarnesses" | "maxConcurrentReviews" | "maxReviewWorktrees" | "postMode" | "secondOpinion" | "autoReview" | "skipDraftPrs" | "incremental" | "handoff" | "hideRepeatDrops">> & ReviewJobSettings;
const normalizeJobs = (settings: ReviewJobSettings): JobForm => ({
  ...settings,
  defaultHarnesses: settings.defaultHarnesses ?? [],
  fallbackHarnesses: settings.fallbackHarnesses ?? [],
  maxConcurrentReviews: settings.maxConcurrentReviews ?? 2,
  maxReviewWorktrees: settings.maxReviewWorktrees ?? 8,
  postMode: settings.postMode ?? "draft",
  secondOpinion: settings.secondOpinion ?? false,
  autoReview: settings.autoReview ?? "off",
  skipDraftPrs: settings.skipDraftPrs ?? true,
  incremental: settings.incremental ?? false,
  handoff: settings.handoff ?? "off",
  hideRepeatDrops: settings.hideRepeatDrops ?? true,
});

const HANDOFF_TEXT: Record<JobForm["handoff"], string> = {
  off: "Reviews stay here and on GitHub.",
  drafted: "As soon as a review drafts, its findings go to the newest chat working on the pull request's branch, which starts on them. Nobody has triaged them yet, and the message says so.",
  posted: "When you post a review, what you posted (with your edits) goes to the newest chat working on the pull request's branch, which starts on them.",
};

const AUTO_REVIEW_TEXT: Record<JobForm["autoReview"], string> = {
  off: "Reviews start only when you ask from the GitHub pane.",
  mine: "Pull requests you opened are reviewed when they open or get new commits, two quiet minutes after the last push. Bridge watches the repositories you have opened in the GitHub pane or reviewed before.",
  all: "Every open pull request is reviewed when it opens or gets new commits, two quiet minutes after the last push. Bridge watches the repositories you have opened in the GitHub pane or reviewed before.",
};

const harnessOptions = (exclude: string[], none: string) => [
  { value: "", label: none },
  ...REVIEWER_HARNESSES.filter(harness => !exclude.includes(harness.id)).map(harness => ({ value: harness.id, label: harness.label })),
];

/** How review jobs run: when they post, on which harnesses, and how many at
 * once. Every behaviour here is opt-in and says what it does. */
function ReviewJobSettingsGroup() {
  const [stored, setStored] = useState<JobForm>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [isFlashed, flash] = useSavedFlash();
  useEffect(() => {
    let active = true;
    void bridgeApi.getReviewJobSettings()
      .then(result => { if (active) setStored(normalizeJobs(result)); })
      .catch(cause => { if (active) setError(friendlyError(cause, "Couldn't load the review settings.")); });
    return () => { active = false; };
  }, []);
  const save = async (patch: Partial<JobForm>, key: string) => {
    if (!stored) return;
    setBusy(true); setError(undefined);
    try {
      setStored(normalizeJobs(await bridgeApi.saveReviewJobSettings({ ...stored, ...patch })));
      flash(key);
    } catch (cause) { setError(friendlyError(cause, "Couldn't save the review settings.")); }
    finally { setBusy(false); }
  };
  if (error && !stored) return <p role="alert" className="text-ui text-destructive">{error}</p>;
  if (!stored) return <SkeletonRows label="Loading review job settings" rows={4} trailing />;
  const [first = "", second = ""] = stored.defaultHarnesses;
  const [fallbackFirst = "", fallbackSecond = ""] = stored.fallbackHarnesses;
  const defaults = (next: string[]) => next.filter(Boolean);
  return <SettingsGroup anchor="review-jobs" label="Review jobs" note="How a review runs once you start it from the GitHub pane. Nothing here posts on its own unless you turn on Post automatically.">
    {error && <p role="alert" className="px-1 text-ui text-destructive">{error}</p>}
    <SettingsRow label="When a review finishes" saved={isFlashed("postMode")}
      description={stored.postMode === "auto" ? "Bridge posts every finding as one comment review as soon as the reviewer is done." : "Findings wait in the pane: keep, drop or edit them, then post."}
      control={<Segmented label="When a review finishes" disabled={busy} value={stored.postMode} onChange={postMode => void save({ postMode }, "postMode")} options={[{ value: "draft", label: "Draft first" }, { value: "auto", label: "Post automatically" }]} />} />
    <SettingsRow label="Auto-review" saved={isFlashed("autoReview")}
      description={AUTO_REVIEW_TEXT[stored.autoReview]}
      control={<Segmented label="Auto-review" disabled={busy} value={stored.autoReview} onChange={autoReview => void save({ autoReview }, "autoReview")} options={[{ value: "off", label: "Off" }, { value: "mine", label: "My PRs" }, { value: "all", label: "All PRs" }]} />} />
    <SettingsRow label="Skip draft PRs" saved={isFlashed("skipDraftPrs")}
      description={stored.autoReview === "off" ? "Applies once auto-review is on." : stored.skipDraftPrs ? "A draft is reviewed once it is marked ready for review." : "Drafts are reviewed like any other pull request."}
      control={<Switch label="Skip draft PRs" disabled={busy || stored.autoReview === "off"} checked={stored.skipDraftPrs} onChange={skipDraftPrs => void save({ skipDraftPrs }, "skipDraftPrs")} />} />
    <SettingsRow label="Incremental re-review" saved={isFlashed("incremental")}
      description={stored.incremental ? "A later review looks at the commits since the last posted review, remembers what was kept, dropped or resolved, and does not repeat it." : "Every review looks at the whole pull request."}
      control={<Switch label="Incremental re-review" disabled={busy} checked={stored.incremental} onChange={incremental => void save({ incremental }, "incremental")} />} />
    <SettingsRow label="Hide findings you keep dropping" saved={isFlashed("hideRepeatDrops")}
      description={stored.hideRepeatDrops ? "A finding like ones you dropped at least three times on two pull requests arrives hidden, with the reason, one click from coming back. It never posts unless you keep it, and critical or security findings are never hidden." : "Every finding shows in the draft."}
      control={<Switch label="Hide findings you keep dropping" disabled={busy} checked={stored.hideRepeatDrops} onChange={hideRepeatDrops => void save({ hideRepeatDrops }, "hideRepeatDrops")} />} />
    <SettingsRow label="Send to the chat that wrote it" saved={isFlashed("handoff")}
      description={`${HANDOFF_TEXT[stored.handoff]}${stored.handoff === "off" ? "" : " At most three reviews of a pull request go to its chat in a day."}`}
      control={<Segmented label="Send reviews to the chat that wrote the pull request" disabled={busy} value={stored.handoff} onChange={handoff => void save({ handoff }, "handoff")} options={[{ value: "off", label: "Off" }, { value: "drafted", label: "When drafted" }, { value: "posted", label: "When posted" }]} />} />
    <SettingsRow label="Second opinion" saved={isFlashed("secondOpinion")}
      description="Default and automatic reviews run on two harnesses. Findings both of them report are marked Both reviewers. Any single review can still ask for a second opinion from the Review menu."
      control={<Switch label="Second opinion" disabled={busy} checked={stored.secondOpinion} onChange={secondOpinion => void save(secondOpinion ? { secondOpinion } : { secondOpinion, defaultHarnesses: defaults([first]) }, "secondOpinion")} />} />
    <SettingsRow label="Default reviewer" saved={isFlashed("default")}
      description="Used when a review is started without picking a harness. Automatic takes the first available of Claude, Codex and OpenCode."
      control={<div className="flex flex-wrap items-center justify-end gap-2">
        <Select label="Default reviewer" disabled={busy} value={first} onChange={value => void save({ defaultHarnesses: defaults([value, value === second ? "" : second]) }, "default")} options={harnessOptions([], "Automatic")} />
        {stored.secondOpinion && first && <Select label="Second reviewer" disabled={busy} value={second} onChange={value => void save({ defaultHarnesses: defaults([first, value]) }, "default")} options={harnessOptions([first], "No second reviewer")} />}
      </div>} />
    <SettingsRow label="Fallback reviewers" saved={isFlashed("fallback")}
      description="When a harness cannot review (signed out, out of usage, crashed), the review moves to these, in this order."
      control={<div className="flex flex-wrap items-center justify-end gap-2">
        <Select label="First fallback" disabled={busy} value={fallbackFirst} onChange={value => void save({ fallbackHarnesses: defaults([value, value === fallbackSecond ? "" : fallbackSecond]) }, "fallback")} options={harnessOptions([], "No fallback")} />
        {fallbackFirst && <Select label="Then" disabled={busy} value={fallbackSecond} onChange={value => void save({ fallbackHarnesses: defaults([fallbackFirst, value]) }, "fallback")} options={harnessOptions([fallbackFirst], "Nothing after")} />}
      </div>} />
    <SettingsRow label="Reviews at once" saved={isFlashed("concurrency")}
      description="How many reviews run at the same time, 1 to 4. Others wait in the queue."
      control={<NumberField label="Reviews at once" disabled={busy} min={1} max={4} value={stored.maxConcurrentReviews} onChange={maxConcurrentReviews => void save({ maxConcurrentReviews }, "concurrency")} />} />
    <SettingsRow label="Review checkouts" saved={isFlashed("worktrees")}
      description="How many PR checkouts reviews may keep on disk at once, 1 to 32. A review waits while they are all in use."
      control={<NumberField label="Review checkouts" disabled={busy} min={1} max={32} value={stored.maxReviewWorktrees} onChange={maxReviewWorktrees => void save({ maxReviewWorktrees }, "worktrees")} />} />
  </SettingsGroup>;
}

/** The Reviews app's Settings: each reviewer's model and effort, how review
 * jobs run, and the reviewer instructions. Global rather than per workspace:
 * they are preferences about the reviewer, not about a repository. An empty
 * model or effort falls through to the Reviewer model profile, then the
 * harness default. */
export function ReviewSettings({ adapters }: { adapters: AdapterDescriptor[] }) {
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
      .catch(cause => { if (active) setError(friendlyError(cause, "Couldn't load the reviewer settings.")); });
    return () => { active = false; };
  }, []);
  const save = async (next: FormSettings, key: string) => {
    setBusy(true); setError(undefined);
    try {
      const result = await bridgeApi.saveReviewerSettings(next);
      setStored(normalize(result.settings)); setDefaultPrompt(result.defaultSystemPrompt);
      flash(key);
      return true;
    } catch (cause) { setError(friendlyError(cause, "Couldn't save the reviewer settings.")); return false; }
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
    <SettingsGroup anchor="reviewer" label="Reviewers" note="The model and effort each harness reviews with. Every review on that harness uses them, whether you start it from the GitHub pane or it starts on its own. A blank model or effort falls back to the Reviewer model profile, then the harness default.">
      {REVIEWER_HARNESSES.map(harness => {
        const adapter = adapters.find(item => item.id === harness.id);
        const entry = stored.harnesses[harness.id] ?? { model: null, effort: null };
        const models = adapter?.models ?? [];
        // A stored model the catalog no longer lists stays selectable rather
        // than silently vanishing from the control.
        const options = [{ value: "", label: "Harness default" }, ...models.map(model => ({ value: model.id, label: model.label }))];
        if (entry.model && !models.some(model => model.id === entry.model)) options.push({ value: entry.model, label: `${entry.model} (not in catalog)` });
        return <SettingsRow key={harness.id} label={harness.label} saved={isFlashed(harness.id)} description={adapter && !adapter.available ? "Unavailable right now" : undefined} control={<div className="flex flex-wrap items-center justify-end gap-2">
          <Select label={`${harness.label} reviewer model`} disabled={busy} value={entry.model ?? ""} onChange={value => changeHarness(harness.id, { model: value || null }, harness.id)} options={options} />
          <Select label={`${harness.label} reviewer effort`} disabled={busy} width="w-36" value={entry.effort ?? ""} onChange={value => changeHarness(harness.id, { effort: (value || null) as ReviewerHarnessSettings["effort"] }, harness.id)} options={[{ value: "", label: "Profile default" }, ...EFFORTS.map(effort => ({ value: effort, label: effort }))]} />
        </div>} />;
      })}
    </SettingsGroup>
    <ReviewJobSettingsGroup />
    <SettingsGroup label="Reviewer instructions" note="Leave empty to use Bridge's default review instructions, shown as the placeholder. {number} expands to the pull request number. The reviewer reads the PR offline and never touches GitHub: Bridge posts one comment review, and never approves, merges or closes.">
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
