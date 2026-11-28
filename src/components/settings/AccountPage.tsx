// The Bridge account: who is signed in to Bridge itself. Not a provider
// account; those live under Coding agents. Sign-in happens in the person's own
// browser, and this page only ever learns who is signed in, never a token.
import { useCallback, useEffect, useRef, useState } from "react";
import { bridgeApi } from "../../api";
import { Button } from "@/components/ui/button";
import { friendlyError } from "../../errors";
import { openInSystemBrowser } from "../../externalLinks";
import type { AccountStatus } from "../../protocol/generated/protocol";
import { GhostButton, PrimaryButton, RowLead, SettingsGroup, SettingsPage, SettingsRow, TextButton } from "./kit";
import { PAGES } from "./sections";

type AccountApi = Pick<typeof bridgeApi, "accountStatus" | "beginAccountSignIn" | "cancelAccountSignIn" | "signOutAccount" | "onAccountChanged">;

function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  return ((parts[0]?.[0] ?? "") + (parts.length > 1 ? parts[parts.length - 1][0] : "")).toUpperCase() || "?";
}

export function AccountPage({ api = bridgeApi, openUrl = openInSystemBrowser }: { api?: AccountApi; openUrl?: (url: string) => Promise<void> }) {
  const [status, setStatus] = useState<AccountStatus>();
  const [failure, setFailure] = useState<string>();
  const [busy, setBusy] = useState(false);
  // The page to reopen while a sign-in waits. It is a URL, not a credential:
  // it carries the PKCE challenge and never the verifier.
  const [authorizationUrl, setAuthorizationUrl] = useState<string>();
  const alive = useRef(true);

  // The latest read wins: a hint can arrive while an older read is in flight.
  const ticket = useRef(0);
  const read = useCallback(() => {
    const mine = ++ticket.current;
    api.accountStatus().then(
      next => { if (alive.current && mine === ticket.current) setStatus(next); },
      error => { if (alive.current && mine === ticket.current) setFailure(friendlyError(error, "Couldn't read your Bridge account.")); },
    );
  }, [api]);

  useEffect(() => {
    alive.current = true;
    let off: (() => void) | undefined;
    // Read once the hint is wired, so a change between the two is not missed.
    void api.onAccountChanged(read).then(
      unlisten => { if (alive.current) off = unlisten; else unlisten(); },
      () => undefined,
    ).finally(() => { if (alive.current) read(); });
    return () => { alive.current = false; off?.(); };
  }, [api, read]);

  async function run(action: () => Promise<AccountStatus | void>, fallback: string) {
    setBusy(true);
    setFailure(undefined);
    try {
      const next = await action();
      if (alive.current && next) setStatus(next);
    } catch (error) {
      if (alive.current) setFailure(friendlyError(error, fallback));
    } finally {
      if (alive.current) setBusy(false);
      read();
    }
  }

  const signIn = () => run(async () => {
    const { authorizationUrl: url } = await api.beginAccountSignIn();
    if (alive.current) setAuthorizationUrl(url);
    if (url) await openUrl(url);
  }, "Couldn't start sign-in.");

  const state = status?.state;
  const user = status?.user;
  const problem = failure ?? (state === "signed_out" ? status?.error : undefined);

  return <SettingsPage title={PAGES.account.label} description={PAGES.account.description}>
    <SettingsGroup label="Bridge account" note="Optional. Bridge works the same signed out, and your chats stay on this Mac.">
      {!status && !failure && <SettingsRow anchor="bridge-account" label="Bridge account" description="Checking…" />}

      {state === "signed_in" && user && <SettingsRow
        anchor="bridge-account"
        lead={<RowLead>{user.avatarUrl
          ? <img src={user.avatarUrl} alt="" referrerPolicy="no-referrer" className="size-7 rounded-full object-cover" />
          : <span aria-hidden="true" className="flex size-7 items-center justify-center rounded-full bg-muted text-caption font-medium text-muted-foreground">{initials(user.name)}</span>}</RowLead>}
        label={user.name}
        description={user.email}
        control={<TextButton busy={busy} disabled={busy} onClick={() => void run(() => api.signOutAccount(), "Couldn't sign out.")}>Sign out</TextButton>}
      />}

      {state === "signing_in" && <SettingsRow
        anchor="bridge-account"
        label="Finish signing in in your browser"
        description={<>Bridge is waiting for you to sign in.{authorizationUrl && <> <Button variant="link" size="inline" onClick={() => void openUrl(authorizationUrl)}>Reopen the browser</Button></>}</>}
        control={<GhostButton disabled={busy} onClick={() => void run(() => api.cancelAccountSignIn(), "Couldn't cancel sign-in.")}>Cancel</GhostButton>}
      />}

      {state === "signed_out" && <SettingsRow
        anchor="bridge-account"
        label="Sign in to Bridge"
        description={status?.available
          ? "Sign in with your browser. Bridge never sees your password."
          : "Sign-in is not available in this build of Bridge."}
        control={<PrimaryButton busy={busy} disabled={busy || !status?.available} onClick={() => void signIn()}>Sign in</PrimaryButton>}
      />}

      {problem && <p role="alert" className="px-3 py-2 text-caption text-destructive">{problem}</p>}
    </SettingsGroup>
  </SettingsPage>;
}
