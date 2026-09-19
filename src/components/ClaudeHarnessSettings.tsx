import { Select, SettingsGroup, SettingsRow } from "./settings/kit";

// Where a read-only Claude worker gets its credential. An interactive session
// never needs this: the sidecar finds the user's own sign-in. A read-only
// worker cannot, because its CLAUDE_CONFIG_DIR is redirected and Claude Code
// scopes credential lookup to that directory, so Bridge hands it one.
//
// This names a *source*, never a credential. Nothing here is a token, and the
// backend refuses secret-looking fields in this harness's advanced JSON.

export type ClaudeWorkerCredentialSource = "auto" | "environment" | "none";

export interface ClaudeAdvancedSettings {
  workerCredentialSource?: ClaudeWorkerCredentialSource;
}

const SOURCES: { value: ClaudeWorkerCredentialSource; label: string; description: string }[] = [
  { value: "auto", label: "Automatic", description: "CLAUDE_CODE_OAUTH_TOKEN if Bridge has it, else the claude CLI's Keychain entry" },
  { value: "environment", label: "Environment only", description: "CLAUDE_CODE_OAUTH_TOKEN from claude setup-token. The Keychain is never read" },
  { value: "none", label: "None", description: "Hand the worker nothing" },
];

export function ClaudeHarnessSettings({ value, disabled, saved, onChange }: {
  value: ClaudeAdvancedSettings;
  disabled: boolean;
  saved?: boolean;
  onChange: (value: ClaudeAdvancedSettings) => void;
}) {
  const source = value.workerCredentialSource ?? "auto";
  return <SettingsGroup label="Read-only workers" note="Interactive sessions sign in on their own">
    <SettingsRow
      label="Worker credential source"
      description="A read-only worker runs with its own config directory, so it cannot see your sign-in. This is where Bridge gets the one it hands over. Bridge never stores the token."
      saved={saved}
      control={<Select
        label="Claude Code worker credential source"
        value={source}
        options={SOURCES}
        disabled={disabled}
        onChange={next => onChange({ workerCredentialSource: next as ClaudeWorkerCredentialSource })}
      />}
    />
  </SettingsGroup>;
}
