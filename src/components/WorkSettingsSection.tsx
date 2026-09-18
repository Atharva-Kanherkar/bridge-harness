import { useEffect, useState } from "react";
import { LoaderCircle as CircleNotch } from "lucide-react";
import { bridgeApi } from "../api";
import { CONNECTOR_LOGOS } from "./connectorLogos";
import type {
  WorkBriefingOptions,
  WorkBriefingProfile,
  WorkSettings,
  WorkSettingsSnapshot,
} from "../protocol/generated/protocol";
import { HarnessMark } from "./harnessMarks";
import { Select, SettingsGroup, SettingsPage, SettingsRow, Switch, useSavedFlash } from "./settings/kit";

// Work briefing: the write path that makes a briefing reachable from a fresh
// install through the UI alone. This surface pre-empts the obvious mistakes, but
// it is not the authority: every rule here is enforced again in Rust, and a
// payload that bypasses this form is refused by the same rules.
//
// Every control on this page is a switch or a select, so every change persists
// on the spot and there is no save bar. The one exception is a change that
// would produce a payload Rust refuses (a briefing with no model, or a
// narrowing with nothing in it). Those are held locally and named in the row
// rather than sent and bounced.

const efforts = ["low", "medium", "high", "xhigh"] as const;

/** Cadence choices. The 15-minute floor mirrors the Rust validation. */
const CADENCES: { value: string; label: string }[] = [
  { value: "", label: "Off, manual refresh only" },
  { value: "15", label: "Every 15 minutes" },
  { value: "30", label: "Every 30 minutes" },
  { value: "60", label: "Every hour" },
  { value: "240", label: "Every 4 hours" },
  { value: "1440", label: "Once a day" },
];

export function WorkSettingsSection({ onError, onOpenBoard }: { onError: (message: string) => void; onOpenBoard?: () => void }) {
  const [snapshot, setSnapshot] = useState<WorkSettingsSnapshot>();
  const [options, setOptions] = useState<WorkBriefingOptions>();
  const [draft, setDraft] = useState<WorkSettings>();
  const [busy, setBusy] = useState(false);
  const [isFlashed, flash] = useSavedFlash();
  // Whether the user is narrowing to specific tools. Local state because the
  // stored shape cannot say "narrowed to nothing": an empty list means read
  // everything, so the in-between moment while tools are being picked lives here.
  const [narrowed, setNarrowed] = useState(false);

  useEffect(() => {
    let active = true;
    Promise.all([bridgeApi.readWorkSettings(), bridgeApi.workBriefingOptions()])
      .then(([stored, briefing]) => {
        if (!active) return;
        setSnapshot(stored);
        setDraft(structuredClone(stored.settings));
        setNarrowed(stored.settings.enabledConnectorInstances.length > 0);
        setOptions(briefing);
      })
      .catch(error => onError(error instanceof Error ? error.message : String(error)));
    return () => { active = false; };
  }, [onError]);

  if (!draft || !options) {
    return <SettingsPage title="Work briefing">
      <SettingsGroup>
        <SettingsRow label="Loading…" control={<CircleNotch size={12} strokeWidth={1.7} className="animate-spin text-muted-foreground" aria-hidden="true" />} />
      </SettingsGroup>
    </SettingsPage>;
  }

  const supported = options.harnesses.filter(harness => harness.supported);
  const refused = options.harnesses.filter(harness => !harness.supported);
  const briefingOn = draft.briefing !== null;
  const chosen = supported.find(harness => harness.id === draft.briefing?.harness);
  const emptyNarrowing = briefingOn && narrowed && draft.enabledConnectorInstances.length === 0;

  /** Hold anything Rust would refuse; persist everything else immediately. */
  const persist = async (next: WorkSettings, key: string, narrowing = narrowed) => {
    setDraft(next);
    const invalid = next.briefing != null
      && (!next.briefing.model || (narrowing && next.enabledConnectorInstances.length === 0));
    if (invalid) return;
    setBusy(true);
    try {
      const stored = await bridgeApi.writeWorkSettings(next);
      setSnapshot(stored);
      setDraft(structuredClone(stored.settings));
      setNarrowed(stored.settings.enabledConnectorInstances.length > 0);
      flash(key);
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };

  const setBriefingOn = (on: boolean) => {
    if (!on) return void persist({ ...draft, briefing: null }, "enabled");
    // The cheapest capable model for the harness is the default; a user pick
    // below wins over it, which is the whole meaning of a default.
    const first = supported[0];
    if (!first) return;
    void persist({ ...draft, briefing: { harness: first.id, model: first.defaultModel ?? "", effort: null } }, "enabled");
  };

  return <SettingsPage
    title="Work briefing"
    description="A briefing of activity from your connected tools in the past 24 hours, using the model you choose."
  >
    {onOpenBoard && <SettingsRow label="Past 24 hours" description="See recent activity from your connected integrations." control={<button type="button" onClick={onOpenBoard} className="rounded-md border border-border px-3 py-2 text-xs text-foreground hover:bg-accent">Open Work</button>} />}
    <SettingsGroup
      label="Integration briefing"
      note={snapshot?.configured ? "Configured" : "Never configured"}
    >
      <SettingsRow
        label="Enabled"
        description="Read recent Slack, GitHub, and other connected integration activity. Turning this off stops new briefings."
        saved={isFlashed("enabled")}
        control={<Switch
          label="Integration briefing"
          checked={briefingOn}
          // Only turning ON needs a certified harness. Off must always be
          // reachable: a stored profile whose harness later loses certification
          // would otherwise freeze this whole form.
          disabled={busy || (!briefingOn && supported.length === 0)}
          onChange={setBriefingOn}
        />}
      />

      {supported.length === 0 && <SettingsRow label="No installed harness has passed the briefing conformance gate yet." />}

      {briefingOn && draft.briefing && !chosen && <SettingsRow
        label={`${draft.briefing.harness} is no longer certified`}
        description="Runs are skipped until you pick another harness or turn the briefing off. Your other settings still save."
      />}

      {briefingOn && draft.briefing && <>
        <SettingsRow
          label="Harness"
          saved={isFlashed("harness")}
          control={<Select
            label="Briefing harness"
            value={draft.briefing.harness}
            disabled={busy}
            options={[
              ...supported.map(harness => ({ value: harness.id, label: harness.label, lead: <HarnessMark harness={harness.id} size={12} /> })),
              ...refused.map(harness => ({ value: harness.id, label: `${harness.label}, unsupported`, disabled: true })),
            ]}
            onChange={value => {
              const next = supported.find(harness => harness.id === value);
              if (!next) return;
              // Connector instance ids are harness-specific, so switching
              // harness drops any narrowing along with the model choice.
              setNarrowed(false);
              void persist({
                ...draft,
                briefing: { harness: next.id, model: next.defaultModel ?? "", effort: null },
                enabledConnectorInstances: [],
              }, "harness", false);
            }}
          />}
        />
        <SettingsRow
          label="Model"
          saved={isFlashed("model")}
          control={<Select
            label="Briefing model"
            value={draft.briefing.model}
            disabled={busy}
            options={(chosen?.models ?? []).map(model => ({
              value: model.id,
              label: model.label,
              description: model.defaultForBriefing ? "Cheapest capable, the default" : undefined,
            }))}
            onChange={value => void persist(
              { ...draft, briefing: draft.briefing ? { ...draft.briefing, model: value } : null },
              "model",
            )}
          />}
        />
        <SettingsRow
          label="Effort"
          saved={isFlashed("effort")}
          control={<Select
            label="Briefing effort"
            value={draft.briefing.effort ?? ""}
            disabled={busy}
            width="w-40"
            options={[{ value: "", label: "Provider default" }, ...efforts.map(value => ({ value, label: value }))]}
            onChange={value => void persist(
              { ...draft, briefing: draft.briefing ? { ...draft.briefing, effort: (value || null) as WorkBriefingProfile["effort"] } : null },
              "effort",
            )}
          />}
        />
      </>}

      {refused.map(harness => <SettingsRow
        key={harness.id}
        label={harness.label}
        description={`Cannot run a briefing: ${harness.reason}`}
      />)}
    </SettingsGroup>

    {briefingOn && chosen && chosen.connectors.length > 0 && <SettingsGroup
      label="What it reads"
      note={`Connected in ${chosen.label}`}
    >
      <SettingsRow
        label="Everything connected"
        description="Including tools you add later. The briefing only ever reads; a tool needing sign-in is reconnected in the harness itself."
        saved={isFlashed("everything")}
        control={<Switch
          label="Everything connected"
          checked={!narrowed}
          disabled={busy}
          onChange={everything => {
            setNarrowed(!everything);
            // Narrowing starts from every signed-in tool checked; "everything"
            // is stored as the empty list so tools connected later are read
            // without another visit here.
            void persist({
              ...draft,
              enabledConnectorInstances: everything
                ? []
                : chosen.connectors.filter(connector => connector.connected !== false).map(connector => connector.id),
            }, "everything", !everything);
          }}
        />}
      />
      {narrowed && chosen.connectors.map(connector => {
        const Logo = CONNECTOR_LOGOS[connector.family];
        const needsAuth = connector.connected === false;
        return <SettingsRow
          key={connector.id}
          lead={Logo ? <Logo size={12} /> : undefined}
          label={connector.id}
          description={needsAuth ? `Needs sign-in in ${chosen.label}` : undefined}
          saved={isFlashed(connector.id)}
          control={<Switch
            label={connector.id}
            checked={draft.enabledConnectorInstances.includes(connector.id) && !needsAuth}
            disabled={busy || needsAuth}
            onChange={on => void persist({
              ...draft,
              enabledConnectorInstances: on
                ? [...draft.enabledConnectorInstances, connector.id]
                : draft.enabledConnectorInstances.filter(id => id !== connector.id),
            }, connector.id)}
          />}
        />;
      })}
      {emptyNarrowing && <SettingsRow label="Pick at least one tool, or switch back to everything. A briefing with nothing to read has nothing to say." />}
    </SettingsGroup>}

    <SettingsGroup label="What it reports">
      <SettingsRow
        label="Include mentions you have already read"
        description="Read state stops excluding an item. Useful for checking a connector is really being read, since a mention you can already see is a signal you can produce on demand."
        saved={isFlashed("mentions")}
        control={<Switch
          label="Include mentions you have already read"
          checked={draft.includeReadMentions ?? false}
          disabled={busy}
          onChange={next => void persist({ ...draft, includeReadMentions: next }, "mentions")}
        />}
      />
    </SettingsGroup>

    <SettingsGroup
      label="When it runs"
      note={`${draft.cooldownMinutes} minute cooldown`}
    >
      <SettingsRow
        label="Cadence"
        description="Racing triggers start at most one run. A manual refresh always works."
        saved={isFlashed("cadence")}
        control={<Select
          label="Cadence"
          value={draft.refreshIntervalMinutes === null ? "" : String(draft.refreshIntervalMinutes)}
          disabled={busy}
          options={CADENCES}
          onChange={value => void persist(
            { ...draft, refreshIntervalMinutes: value === "" ? null : Number(value) },
            "cadence",
          )}
        />}
      />
      <SettingsRow
        label="Refresh on focus"
        description="Also refresh when Bridge regains focus."
        saved={isFlashed("focus")}
        control={<Switch
          label="Refresh on focus"
          checked={draft.refreshOnFocus}
          disabled={busy}
          onChange={next => void persist({ ...draft, refreshOnFocus: next }, "focus")}
        />}
      />
    </SettingsGroup>
  </SettingsPage>;
}
