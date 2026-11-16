import { Check } from "lucide-react";
import { cn } from "../lib/utils";
import { HarnessMark } from "./harnessMarks";

/** Where a provider sign-in is. `preparing` covers the call that creates an
 * account's folder and spawns the CLI; `starting` is the CLI booting before
 * it has printed a link; `browser` waits on the person; `checking` confirms
 * the result after the CLI exits. */
export type SignInStage = "preparing" | "starting" | "browser" | "checking" | "done";

type Step = { stage: Exclude<SignInStage, "done">; label: string };

/** The steps a sign-in walks through. An account adds its folder step first,
 * since that is the part Bridge now does for the person. */
export function signInSteps(account: boolean): Step[] {
  return [
    ...(account ? [{ stage: "preparing" as const, label: "Set up folder" }] : []),
    { stage: account ? "starting" : "preparing", label: "Start agent" },
    { stage: "browser", label: "Sign in" },
    { stage: "checking", label: "Confirm" },
  ];
}

/** The index of the step in progress; `steps.length` once everything is done. */
export function activeStepIndex(steps: readonly Step[], stage: SignInStage): number {
  if (stage === "done") return steps.length;
  // Without an account, `preparing` and `starting` are one step.
  const effective = stage === "starting" && !steps.some(step => step.stage === "starting") ? "preparing" : stage;
  const index = steps.findIndex(step => step.stage === effective);
  return index < 0 ? 0 : index;
}

/** The one line narrating what Bridge is waiting on right now. */
export function signInNarration(stage: SignInStage, label: string, account: boolean, slow: boolean): string {
  switch (stage) {
    case "preparing": return account ? "Creating this account's folder…" : `Starting ${label}…`;
    case "starting": return slow ? `${label} is taking a moment to start…` : `Starting ${label}…`;
    case "browser": return "Waiting for you to finish in the browser…";
    case "checking": return "Confirming sign-in…";
    case "done": return "Signed in";
  }
}

/** Sign-in progress: the agent's mark breathing inside a halo, one line of
 * narration, and a step rail whose active segment carries a travelling light.
 * It replaces a bare spinner so a CLI that is slow to print still reads as
 * work in progress. */
export function SignInProgress({ harness, label, stage, account = false, slow = false }: {
  harness: string;
  label: string;
  stage: SignInStage;
  account?: boolean;
  slow?: boolean;
}) {
  const steps = signInSteps(account);
  const active = activeStepIndex(steps, stage);
  const narration = signInNarration(stage, label, account, slow);
  return <div role="status" aria-live="polite" aria-label={`${label} sign-in progress`} data-stage={stage} className="grid gap-3">
    <div className="flex items-center gap-3">
      <span className="relative grid size-9 shrink-0 place-items-center">
        {stage !== "done" && <span aria-hidden="true" className="signin-halo absolute inset-0 rounded-xl border border-ring/60" />}
        <span className="relative grid size-9 place-items-center rounded-xl border border-border-card bg-background">
          <HarnessMark harness={harness} size={17} live={stage !== "done"} />
        </span>
      </span>
      <p className="min-w-0 flex-1 text-[13px] font-medium text-foreground" data-testid="signin-narration">{narration}</p>
    </div>
    <ol className={cn("grid gap-1.5", steps.length === 4 ? "grid-cols-4" : "grid-cols-3")}>
      {steps.map((step, index) => {
        const state = index < active ? "done" : index === active ? "active" : "pending";
        return <li key={step.label} data-state={state} aria-current={state === "active" ? "step" : undefined} className="grid gap-1">
          <span className="relative h-1 overflow-hidden rounded-full bg-border">
            <span className={cn("absolute inset-0 rounded-full bg-foreground transition-transform duration-500 ease-out origin-left",
              state === "done" ? "scale-x-100" : "scale-x-0")} />
            {state === "active" && <span aria-hidden="true" className="signin-rail-light absolute inset-y-0 left-0 w-2/5 rounded-full bg-foreground/70" />}
          </span>
          <span className={cn("flex items-center gap-1 text-[11px]", state === "pending" ? "text-muted-foreground/70" : "text-muted-foreground", state === "active" && "font-medium text-foreground")}>
            {state === "done" && <Check size={11} aria-hidden="true" />}
            {step.label}
          </span>
        </li>;
      })}
    </ol>
  </div>;
}
