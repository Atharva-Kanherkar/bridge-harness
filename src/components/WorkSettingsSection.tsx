import { useEffect, useState } from "react";
import { LoaderCircle, Save } from "lucide-react";
import { bridgeApi } from "../api";
import type {
  WorkBriefingOptions,
  WorkBriefingProfile,
  WorkSettings,
  WorkSettingsSnapshot,
} from "../protocol/generated/protocol";
import { cn } from "@/lib/utils";

// The Work section: the write path that makes a briefing reachable from a fresh
// install through the UI alone. This surface pre-empts the obvious mistakes, but
// it is not the authority — every rule here is enforced again in Rust, and a
// payload that bypasses this form is refused by the same rules.

const field =
  "h-10 w-full rounded-xl border border-border bg-background/60 px-3 text-sm text-foreground outline-none transition-colors focus:border-foreground/25 disabled:opacity-45";

const efforts = ["low", "medium", "high", "xhigh"] as const;

/** Cadence choices. The 15-minute floor mirrors the Rust validation. */
const CADENCES: { value: number | null; label: string }[] = [
  { value: null, label: "Off — manual refresh only" },
  { value: 15, label: "Every 15 minutes" },
  { value: 30, label: "Every 30 minutes" },
  { value: 60, label: "Every hour" },
  { value: 240, label: "Every 4 hours" },
  { value: 1440, label: "Once a day" },
];

export function WorkSettingsSection({ onError }: { onError: (message: string) => void }) {
  const [snapshot, setSnapshot] = useState<WorkSettingsSnapshot>();
  const [options, setOptions] = useState<WorkBriefingOptions>();
  const [draft, setDraft] = useState<WorkSettings>();
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    let active = true;
    Promise.all([bridgeApi.readWorkSettings(), bridgeApi.workBriefingOptions()])
      .then(([stored, briefing]) => {
        if (!active) return;
        setSnapshot(stored);
        setDraft(structuredClone(stored.settings));
        setOptions(briefing);
      })
      .catch(error => onError(error instanceof Error ? error.message : String(error)));
    return () => {
      active = false;
    };
  }, [onError]);

  if (!draft || !options) {
    return (
      <div className="grid h-full place-items-center">
        <LoaderCircle className="animate-spin text-muted-foreground" size={18} />
      </div>
    );
  }

  const supported = options.harnesses.filter(harness => harness.supported);
  const refused = options.harnesses.filter(harness => !harness.supported);
  const briefingOn = draft.briefing !== null;
  const chosen = supported.find(harness => harness.id === draft.briefing?.harness);

  const setBriefingOn = (on: boolean) => {
    if (!on) {
      setDraft(value => value && { ...value, briefing: null });
      return;
    }
    // The cheapest capable model for the harness is the default; a user pick
    // below wins over it, which is the whole meaning of a default.
    const first = supported[0];
    if (!first) return;
    setDraft(
      value =>
        value && {
          ...value,
          briefing: { harness: first.id, model: first.defaultModel ?? "", effort: null },
        },
    );
  };

  const save = async () => {
    setBusy(true);
    try {
      const stored = await bridgeApi.writeWorkSettings(draft);
      setSnapshot(stored);
      setDraft(structuredClone(stored.settings));
      setSaved(true);
      window.setTimeout(() => setSaved(false), 1600);
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="mx-auto max-w-2xl">
      <div className="mb-5">
        <h2 className="font-display text-lg font-semibold">Work briefing</h2>
        <p className="mt-1 text-xs text-muted-foreground">
          A background model turn on the harness you pick, reading your connected tools —
          read-only, on your account, summarised onto the Work board.
          {snapshot?.configured
            ? " Configured."
            : " Never configured — briefing is off until you set it up here."}
        </p>
      </div>

      <section className="rounded-3xl border border-border/80 bg-card/45 p-5">
        <div className="flex items-start justify-between gap-3">
          <div>
            <h3 className="font-display font-semibold">Suggested work</h3>
            <p className="mt-1 text-[11px] text-muted-foreground">
              Off is a choice Bridge remembers — not the absence of one. Facts stay on the
              board either way.
            </p>
          </div>
          <label className="flex items-center gap-2 text-xs text-muted-foreground">
            {/* Only turning ON needs a certified harness. Off must always be
                reachable — a stored profile whose harness later loses
                certification would otherwise freeze this whole form. */}
            <input
              type="checkbox"
              checked={briefingOn}
              disabled={!briefingOn && supported.length === 0}
              onChange={event => setBriefingOn(event.target.checked)}
            />
            Enabled
          </label>
        </div>

        {supported.length === 0 && (
          <p className="mt-3 text-[11.5px] text-muted-foreground">
            No installed harness has passed the briefing conformance gate yet.
          </p>
        )}

        {briefingOn && draft.briefing && !chosen && (
          <p className="mt-3 text-[11.5px] leading-relaxed text-muted-foreground">
            <span className="font-medium text-foreground">
              {draft.briefing.harness} is no longer certified.
            </span>{" "}
            Runs are skipped until you pick another harness or turn the briefing off — your
            other settings still save.
          </p>
        )}

        {briefingOn && draft.briefing && (
          <div className="mt-4 grid gap-4 sm:grid-cols-2">
            <label className="space-y-1.5 text-[11px] font-medium text-muted-foreground">
              Harness
              <select
                className={field}
                value={draft.briefing.harness}
                onChange={event => {
                  const next = supported.find(harness => harness.id === event.target.value);
                  if (!next) return;
                  setDraft(
                    value =>
                      value && {
                        ...value,
                        briefing: { harness: next.id, model: next.defaultModel ?? "", effort: null },
                      },
                  );
                }}
              >
                {supported.map(harness => (
                  <option key={harness.id} value={harness.id}>
                    {harness.label}
                  </option>
                ))}
                {refused.map(harness => (
                  <option key={harness.id} value={harness.id} disabled>
                    {harness.label} — unsupported
                  </option>
                ))}
              </select>
            </label>
            <label className="space-y-1.5 text-[11px] font-medium text-muted-foreground">
              Model
              <select
                className={field}
                value={draft.briefing.model}
                onChange={event =>
                  setDraft(value =>
                    value && value.briefing
                      ? { ...value, briefing: { ...value.briefing, model: event.target.value } }
                      : value,
                  )
                }
              >
                {(chosen?.models ?? []).map(model => (
                  <option key={model.id} value={model.id}>
                    {model.label}
                    {model.defaultForBriefing ? " — cheapest capable, the default" : ""}
                  </option>
                ))}
              </select>
            </label>
            <label className="space-y-1.5 text-[11px] font-medium text-muted-foreground">
              Effort
              <select
                className={field}
                value={draft.briefing.effort ?? ""}
                onChange={event =>
                  setDraft(value =>
                    value && value.briefing
                      ? {
                          ...value,
                          briefing: {
                            ...value.briefing,
                            effort: (event.target.value || null) as WorkBriefingProfile["effort"],
                          },
                        }
                      : value,
                  )
                }
              >
                <option value="">Provider default</option>
                {efforts.map(value => (
                  <option key={value} value={value}>
                    {value}
                  </option>
                ))}
              </select>
            </label>
          </div>
        )}

        {refused.length > 0 && (
          <div className="mt-4 space-y-1">
            {refused.map(harness => (
              <p key={harness.id} className="text-[11px] leading-relaxed text-muted-foreground">
                <span className="font-medium text-foreground">{harness.label}</span> cannot run a
                briefing: {harness.reason}
              </p>
            ))}
          </div>
        )}
      </section>

      <section className="mt-4 rounded-3xl border border-border/80 bg-card/45 p-5">
        <h3 className="font-display font-semibold">When it runs</h3>
        <div className="mt-4 grid gap-4 sm:grid-cols-2">
          <label className="space-y-1.5 text-[11px] font-medium text-muted-foreground">
            Cadence
            <select
              className={field}
              value={draft.refreshIntervalMinutes ?? ""}
              onChange={event =>
                setDraft(
                  value =>
                    value && {
                      ...value,
                      refreshIntervalMinutes: event.target.value === "" ? null : Number(event.target.value),
                    },
                )
              }
            >
              {CADENCES.map(cadence => (
                <option key={cadence.label} value={cadence.value ?? ""}>
                  {cadence.label}
                </option>
              ))}
            </select>
          </label>
          <label className="mt-5 flex items-center gap-2 text-xs text-muted-foreground sm:mt-7">
            <input
              type="checkbox"
              checked={draft.refreshOnFocus}
              onChange={event =>
                setDraft(value => value && { ...value, refreshOnFocus: event.target.checked })
              }
            />
            Also refresh when Bridge regains focus
          </label>
        </div>
        <p className="mt-3 text-[11px] leading-relaxed text-muted-foreground">
          Racing triggers start at most one run, and unattended runs respect a{" "}
          {draft.cooldownMinutes}-minute cooldown. A manual refresh always works.
        </p>
      </section>

      <div className="mt-5 flex items-center gap-2">
        <button
          type="button"
          disabled={busy || (briefingOn && !draft.briefing?.model)}
          onClick={() => void save()}
          className="inline-flex h-9 items-center gap-2 rounded-xl bg-foreground px-3.5 text-xs font-medium text-background disabled:opacity-40"
        >
          <Save size={13} />
          Save Work settings
        </button>
        {saved && <span className={cn("text-xs text-success")}>Saved</span>}
      </div>
    </div>
  );
}
