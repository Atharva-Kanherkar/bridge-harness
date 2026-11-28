// The Bridge account outside the desktop app (`bun run dev`, component tests):
// a sign-in that needs no browser and lands on a demo person a moment later.
import type { AccountStatus, BeginAccountSignIn } from "./protocol/generated/protocol";

const DEMO_USER = { id: "user_demo", email: "ada@example.com", name: "Ada Lovelace" };
const SIGN_IN_MS = 1200;

let status: AccountStatus = { state: "signed_out", available: true };
let landing: ReturnType<typeof setTimeout> | undefined;
const listeners = new Set<() => void>();

function move(next: AccountStatus) {
  status = next;
  listeners.forEach(listener => listener());
}

export function mockAccountStatus(): AccountStatus {
  return { ...status };
}

export function mockBeginAccountSignIn(): BeginAccountSignIn {
  clearTimeout(landing);
  move({ state: "signing_in", available: true });
  landing = setTimeout(() => move({ state: "signed_in", user: DEMO_USER, available: true }), SIGN_IN_MS);
  return { authorizationUrl: "" };
}

export function mockCancelAccountSignIn(): AccountStatus {
  clearTimeout(landing);
  if (status.state === "signing_in") move({ state: "signed_out", available: true });
  return mockAccountStatus();
}

export function mockSignOutAccount(): AccountStatus {
  clearTimeout(landing);
  move({ state: "signed_out", available: true });
  return mockAccountStatus();
}

export function onMockAccountChanged(handler: () => void): () => void {
  listeners.add(handler);
  return () => { listeners.delete(handler); };
}
