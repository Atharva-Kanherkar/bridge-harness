// Composer: the inline typeahead, and nothing else yet.
//
// It used to sit at the bottom of Role models, under six worker profiles, which
// is nowhere near where anyone looks for a composer behavior. Its own page is
// short on purpose: a page that exists is findable, and the rail search now
// answers "where is the suggestion model" without anyone having to guess.
//
// Both controls persist on change. There is no text on this page, so there is
// no save bar.

import { useEffect, useState } from "react";
import { LoaderCircle as CircleNotch } from "lucide-react";
import { bridgeApi } from "../../api";
import type { AdapterDescriptor } from "../../types";
import type { SuggestionSettings, SuggestionSettingsSnapshot } from "../../protocol/generated/protocol";
import { HarnessMark } from "../harnessMarks";
import { Select, SettingsGroup, SettingsPage, SettingsRow, Switch, useSavedFlash } from "./kit";

export function ComposerPage({ adapters, onChange, onError }: {
  adapters: AdapterDescriptor[];
  onChange: (snapshot: SuggestionSettingsSnapshot) => void;
  onError: (message: string) => void;
}) {
  const [draft, setDraft] = useState<SuggestionSettings>();
  const [busy, setBusy] = useState(false);
  const [isFlashed, flash] = useSavedFlash();

  useEffect(() => {
    let active = true;
    bridgeApi.getSuggestionSettings()
      .then(snapshot => { if (active) setDraft(structuredClone(snapshot.settings)); })
      .catch(error => onError(error instanceof Error ? error.message : String(error)));
    return () => { active = false; };
  }, [onError]);

  // Every write goes through here, so the row can only claim "Saved" after the
  // host has actually stored it.
  const persist = async (next: SuggestionSettings, key: string) => {
    setDraft(next);
    setBusy(true);
    try {
      const snapshot = await bridgeApi.saveSuggestionSettings(next);
      setDraft(structuredClone(snapshot.settings));
      onChange(snapshot);
      flash(key);
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };

  const options = adapters
    .filter(adapter => adapter.available)
    .flatMap(adapter => adapter.models.map(model => ({
      value: `${adapter.id}:${model.id}`,
      label: `${adapter.label} · ${model.label}`,
      lead: <HarnessMark harness={adapter.id} size={12} />,
    })));

  return <SettingsPage
    title="Composer"
    description="How Bridge behaves while you are writing, before a turn starts."
  >
    <SettingsGroup label="Inline suggestions">
      {!draft
        ? <SettingsRow label="Loading…" control={<CircleNotch size={12} strokeWidth={1.7} className="animate-spin text-muted-foreground" aria-hidden="true" />} />
        : <>
            <SettingsRow
              label="Enabled"
              description="Ghost-text continuations of your draft, accepted with Tab. Your draft text is sent to the model only while this is on."
              saved={isFlashed("enabled")}
              control={<Switch
                label="Inline suggestions"
                checked={draft.enabled}
                disabled={busy}
                onChange={next => void persist({ ...draft, enabled: next }, "enabled")}
              />}
            />
            <SettingsRow
              label="Suggestion model"
              description="Kept separate from your chat model, because this one runs on every keystroke."
              saved={isFlashed("model")}
              control={<Select
                label="Suggestion model"
                value={`${draft.provider}:${draft.model}`}
                disabled={busy || !draft.enabled}
                options={options.some(option => option.value === `${draft.provider}:${draft.model}`)
                  ? options
                  : [{ value: `${draft.provider}:${draft.model}`, label: `${draft.provider}:${draft.model}` }, ...options]}
                onChange={value => {
                  const separator = value.indexOf(":");
                  const provider = separator === -1 ? value : value.slice(0, separator);
                  const model = separator === -1 ? "" : value.slice(separator + 1);
                  void persist({ ...draft, provider, model }, "model");
                }}
              />}
            />
          </>}
    </SettingsGroup>
  </SettingsPage>;
}
