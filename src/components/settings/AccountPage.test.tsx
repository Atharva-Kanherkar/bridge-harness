// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { AccountStatus } from "../../protocol/generated/protocol";
import { AccountPage } from "./AccountPage";
import { filterSettingsRows, PAGE_ROWS, SETTINGS_ROWS } from "./settingsSearch";

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

const ADA = { id: "user_1", email: "ada@example.com", name: "Ada Lovelace" };

/** A host whose status the test moves, the way `account/changed` would. */
function host(initial: AccountStatus) {
  let status = initial;
  let changed = () => {};
  const api = {
    accountStatus: vi.fn(async () => status),
    beginAccountSignIn: vi.fn(async () => { status = { state: "signing_in", available: true }; return { authorizationUrl: "https://api.workos.com/user_management/authorize?state=s1" }; }),
    cancelAccountSignIn: vi.fn(async () => (status = { state: "signed_out", available: true })),
    signOutAccount: vi.fn(async () => (status = { state: "signed_out", available: true })),
    onAccountChanged: vi.fn(async (handler: () => void) => { changed = handler; return () => { changed = () => {}; }; }),
  };
  return { api, move: async (next: AccountStatus) => { status = next; await act(async () => changed()); } };
}

const button = (label: string) => [...container.querySelectorAll("button")].find(node => node.textContent?.trim() === label);

async function mount(initial: AccountStatus, openUrl = vi.fn(async () => {})) {
  const fake = host(initial);
  await act(async () => root.render(<AccountPage api={fake.api} openUrl={openUrl} />));
  return { ...fake, openUrl };
}

it("offers sign-in when signed out and says Bridge works without it", async () => {
  await mount({ state: "signed_out", available: true });
  expect(container.textContent).toContain("Sign in to Bridge");
  expect(container.textContent).toContain("Bridge works the same signed out");
  expect(button("Sign in")?.disabled).toBe(false);
  expect(container.querySelector("[role=alert]")).toBeNull();
});

it("opens the system browser, waits with Cancel and a reopen link, then shows who signed in without a restart", async () => {
  const { api, move, openUrl } = await mount({ state: "signed_out", available: true });
  await act(async () => button("Sign in")?.click());
  expect(api.beginAccountSignIn).toHaveBeenCalledTimes(1);
  expect(openUrl).toHaveBeenCalledWith("https://api.workos.com/user_management/authorize?state=s1");
  expect(container.textContent).toContain("Finish signing in in your browser");
  expect(button("Cancel")).toBeTruthy();

  await act(async () => button("Reopen the browser")?.click());
  expect(openUrl).toHaveBeenCalledTimes(2);

  await move({ state: "signed_in", user: ADA, available: true });
  expect(container.textContent).toContain("Ada Lovelace");
  expect(container.textContent).toContain("ada@example.com");
  expect(button("Sign out")).toBeTruthy();
  expect(button("Sign in")).toBeUndefined();
});

it("cancels a sign-in back to signed out", async () => {
  const { api } = await mount({ state: "signing_in", available: true });
  await act(async () => button("Cancel")?.click());
  expect(api.cancelAccountSignIn).toHaveBeenCalledTimes(1);
  expect(container.textContent).toContain("Sign in to Bridge");
});

it("signs out", async () => {
  const { api } = await mount({ state: "signed_in", user: ADA, available: true });
  expect(container.textContent).toContain("AL");
  await act(async () => button("Sign out")?.click());
  expect(api.signOutAccount).toHaveBeenCalledTimes(1);
  expect(container.textContent).toContain("Sign in to Bridge");
  expect(container.textContent).not.toContain("ada@example.com");
});

it("shows why the last sign-in ended signed out", async () => {
  await mount({ state: "signed_out", available: true, error: "Your Bridge sign-in expired. Sign in again." });
  expect(container.querySelector("[role=alert]")?.textContent).toBe("Your Bridge sign-in expired. Sign in again.");
  expect(button("Sign in")?.disabled).toBe(false);
});

it("shows a failure to start sign-in and stays usable", async () => {
  const { api } = await mount({ state: "signed_out", available: true });
  api.beginAccountSignIn.mockRejectedValueOnce("Bridge could not open a local port for sign-in.");
  await act(async () => button("Sign in")?.click());
  expect(container.querySelector("[role=alert]")?.textContent).toContain("could not open a local port");
  expect(button("Sign in")?.disabled).toBe(false);
});

it("turns sign-in off in a build that cannot sign in", async () => {
  await mount({ state: "signed_out", available: false });
  expect(container.textContent).toContain("Sign-in is not available in this build of Bridge.");
  expect(button("Sign in")?.disabled).toBe(true);
});

it("stops listening when the page closes", async () => {
  const { api, move } = await mount({ state: "signed_out", available: true });
  const reads = api.accountStatus.mock.calls.length;
  act(() => root.unmount());
  await move({ state: "signed_in", user: ADA, available: true });
  expect(api.accountStatus).toHaveBeenCalledTimes(reads);
  root = createRoot(container);
});

it("is found by settings search", () => {
  const rows = [...PAGE_ROWS, ...SETTINGS_ROWS];
  expect(filterSettingsRows("account", rows).map(group => group.page)).toContain("account");
  expect(filterSettingsRows("sign in", rows).map(group => group.page)).toContain("account");
});
