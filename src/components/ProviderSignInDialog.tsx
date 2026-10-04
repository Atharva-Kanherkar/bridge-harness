import { useRef, useState } from "react";
import { CircleAlert, CircleCheck, CircleHelp, LoaderCircle } from "lucide-react";
import { bridgeApi } from "../api";
import type { UsageProvider } from "../usage";
import { Button } from "./ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogPanel, DialogTitle } from "./ui/dialog";
import { HarnessMark } from "./harnessMarks";
import { ProviderLoginPane } from "./ProviderLoginPane";

type Phase = "signing-in" | "checking" | "signed-in" | "failed" | "unconfirmed";
type Outcome = "signed-in" | "failed" | "unconfirmed";

// Vendor CLIs report a failed login in their own words before exiting. The
// exit itself carries no status, so this text is the only negative signal.
const FAILED_LOGIN = /\b(?:login|sign-?in|authentication|oauth)\b.{0,40}\b(?:failed|error|cancel+ed|denied|timed out)\b|^error:/im;

/** Health's auth probe can lag a successful login (Claude reads it from the
 * Keychain), so only an explicit signed-out answer counts as a failure. A
 * provider that reports `unknown` after a clean exit is trusted; a health
 * read that failed, or never mentioned the provider, proves nothing. */
export async function providerSignInOutcome(provider: string, output: string): Promise<Outcome> {
  if (FAILED_LOGIN.test(output)) return "failed";
  const health = await bridgeApi.health().catch(() => null);
  const state = health?.adapters.find(adapter => adapter.id === provider)?.authState;
  if (!state) return "unconfirmed";
  return state === "signed_out" ? "failed" : "signed-in";
}

/** Sign-in raised by a failed turn. It confirms the result before closing so
 * the user knows the provider is reachable again, and offers the retry. */
export function ProviderSignInDialog({ provider, label, onRetry, onAuthChanged, onClose }: {
  provider: UsageProvider | null;
  label: string;
  onRetry?: () => void;
  onAuthChanged: () => void;
  onClose: () => void;
}) {
  const [phase, setPhase] = useState<Phase>("signing-in");
  const [attempt, setAttempt] = useState(0);
  // A check still in flight when the dialog closes must not paint its
  // verdict onto the next sign-in.
  const runRef = useRef(0);
  const outputRef = useRef("");
  const close = () => { runRef.current += 1; setPhase("signing-in"); onClose(); };
  const exited = (output: string) => {
    if (!provider) return;
    outputRef.current = output;
    const run = runRef.current;
    setPhase("checking");
    void providerSignInOutcome(provider, output).then(outcome => {
      onAuthChanged();
      if (runRef.current === run) setPhase(outcome);
    });
  };
  const retry = () => { close(); onRetry?.(); };
  return <Dialog open={provider !== null} onOpenChange={open => { if (!open) close(); }}>
    <DialogContent className="max-w-md">
      <DialogHeader>
        <span className="grid size-10 place-items-center rounded-xl border border-border-card bg-background">
          {phase === "signed-in"
            ? <CircleCheck size={18} className="text-success" aria-hidden="true" />
            : phase === "failed"
              ? <CircleAlert size={18} className="text-destructive" aria-hidden="true" />
              : phase === "unconfirmed"
                ? <CircleHelp size={18} className="text-muted-foreground" aria-hidden="true" />
                : <HarnessMark harness={provider} size={18} />}
        </span>
        <DialogTitle className="mt-2 text-lg">
          {phase === "signed-in" ? `Signed in to ${label}` : phase === "failed" ? "Sign-in didn't finish" : phase === "unconfirmed" ? "Couldn't confirm sign-in" : `Sign in to ${label}`}
        </DialogTitle>
        <DialogDescription>
          {phase === "signed-in"
            ? onRetry ? "You're all set. Send your message again to pick up where you left off." : "You're all set. Your next message will go through."
            : phase === "failed"
              ? `${label} still reports you're signed out. Your conversation is kept.`
              : phase === "unconfirmed"
                ? `Bridge couldn't read ${label}'s sign-in status. If you finished in the browser, check again.`
                : `Your ${label} session expired. Your conversation is kept; sign in to continue.`}
        </DialogDescription>
      </DialogHeader>
      {(phase === "signing-in" || phase === "checking") && provider && <DialogPanel scrollFade={false}>
        {phase === "checking"
          ? <p className="flex items-center gap-2 text-[12px] text-muted-foreground" role="status">
            <LoaderCircle size={14} className="animate-spin" aria-hidden="true" /> Checking your {label} sign-in…
          </p>
          : <ProviderLoginPane key={attempt} bare provider={provider} label={label} onClose={close} onExited={exited} />}
      </DialogPanel>}
      <DialogFooter variant="bare">
        {phase === "signed-in"
          ? <>
            <Button variant="ghost" onClick={close}>{onRetry ? "Not now" : "Done"}</Button>
            {onRetry && <Button onClick={retry}>Retry message</Button>}
          </>
          : phase === "failed"
            ? <>
              <Button variant="ghost" onClick={close}>Close</Button>
              <Button onClick={() => { setAttempt(value => value + 1); setPhase("signing-in"); }}>Try again</Button>
            </>
            : phase === "unconfirmed"
              ? <>
                <Button variant="ghost" onClick={close}>Close</Button>
                <Button onClick={() => exited(outputRef.current)}>Check again</Button>
              </>
              : <Button variant="ghost" onClick={close}>Cancel</Button>}
      </DialogFooter>
    </DialogContent>
  </Dialog>;
}
