import { useEffect, useState } from "react";
import { LoaderCircle, Save } from "lucide-react";
import { bridgeApi } from "../api";
import type { AdapterDescriptor } from "../types";
import type { SuggestionSettings, SuggestionSettingsSnapshot } from "../protocol/generated/protocol";

// The composer's inline typeahead: off by default, so this card is the whole
// of what makes it reachable. Validation is Rust's; this form only pre-empts
// the obvious mistake of an empty provider/model pair.

const field =
  "h-10 w-full rounded-xl border border-border bg-background/60 px-3 text-sm text-foreground outline-none transition-colors focus:border-foreground/25 disabled:opacity-45";

export function SuggestionSettingsCard({
  adapters,
  onChange,
  onError,
}: {
  adapters: AdapterDescriptor[];
  onChange: (snapshot: SuggestionSettingsSnapshot) => void;
  onError: (message: string) => void;
}) {
  const [draft, setDraft] = useState<SuggestionSettings>();
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    let active = true;
    bridgeApi
      .getSuggestionSettings()
      .then(snapshot => {
        if (active) setDraft(structuredClone(snapshot.settings));
      })
      .catch(error => onError(error instanceof Error ? error.message : String(error)));
    return () => {
      active = false;
    };
  }, [onError]);

  if (!draft) {
    return (
      <div className="grid h-24 place-items-center">
        <LoaderCircle className="animate-spin text-muted-foreground" size={16} />
      </div>
    );
  }

  const options = adapters
    .filter(adapter => adapter.available)
    .flatMap(adapter => adapter.models.map(model => ({ provider: adapter.id, model: model.id, label: `${adapter.label} · ${model.label}` })));
  const current = `${draft.provider}:${draft.model}`;

  const save = async () => {
    setBusy(true);
    try {
      const snapshot = await bridgeApi.saveSuggestionSettings(draft);
      onChange(snapshot);
      setSaved(true);
      window.setTimeout(() => setSaved(false), 1600);
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="mt-6 rounded-3xl border border-border/80 bg-card/45 p-5">
      <div className="mb-4 flex items-start justify-between gap-3">
        <div>
          <h3 className="font-display text-base font-semibold">Inline suggestions</h3>
          <p className="mt-1 text-xs text-muted-foreground">
            Ghost-text continuations of your draft in the composer, accepted with Tab. Off by
            default — your draft text is sent to the model only while this is enabled.
          </p>
        </div>
        <label className="flex shrink-0 items-center gap-2 text-xs text-muted-foreground">
          <input
            type="checkbox"
            checked={draft.enabled}
            onChange={event => setDraft(value => value && { ...value, enabled: event.target.checked })}
          />
          Enabled
        </label>
      </div>
      <label className="block max-w-sm space-y-1.5 text-[11px] font-medium text-muted-foreground">
        Suggestion model
        <select
          className={field}
          disabled={!draft.enabled || options.length === 0}
          value={current}
          onChange={event => {
            const [provider, model] = event.target.value.split(":");
            setDraft(value => value && { ...value, provider, model });
          }}
        >
          {!options.some(option => `${option.provider}:${option.model}` === current) && (
            <option value={current}>{current}</option>
          )}
          {options.map(option => (
            <option key={`${option.provider}:${option.model}`} value={`${option.provider}:${option.model}`}>
              {option.label}
            </option>
          ))}
        </select>
      </label>
      <div className="mt-4 flex items-center gap-3">
        <button
          type="button"
          disabled={busy}
          onClick={() => void save()}
          className="inline-flex h-9 items-center gap-2 rounded-xl bg-foreground px-3.5 text-xs font-medium text-background disabled:opacity-40"
        >
          <Save size={13} />
          Save
        </button>
        {saved && <span className="text-xs text-success">Saved</span>}
      </div>
    </section>
  );
}
