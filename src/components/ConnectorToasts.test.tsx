// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ConnectorToasts, CONNECTOR_TOAST_TTL_MS } from "./ConnectorToasts";
import { reduceToasts, type ConnectorToast } from "../connectorSurface";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let root: Root | undefined;
let host: HTMLDivElement | undefined;

const toast = (overrides: Partial<ConnectorToast> = {}): ConnectorToast => ({
  key: "slack:D09:1.0",
  itemKey: "slack:D09:1.0",
  family: "slack",
  headline: "Nina Alvarez sent you a direct message",
  detail: "Nina Alvarez",
  settled: false,
  ...overrides,
});

function mount(node: React.ReactElement) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => { root!.render(node); });
  return host;
}

afterEach(() => {
  act(() => root?.unmount());
  host?.remove();
  root = undefined;
  host = undefined;
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("ConnectorToasts", () => {
  it("renders nothing when there is nothing to say", () => {
    expect(mount(<ConnectorToasts toasts={[]} onOpen={() => {}} onDismiss={() => {}} />).textContent).toBe("");
  });

  it("shows the headline and offers to reply in place", () => {
    const node = mount(<ConnectorToasts toasts={[toast()]} onOpen={() => {}} onDismiss={() => {}} />);
    expect(node.textContent).toContain("Nina Alvarez sent you a direct message");
    // The whole premise: the notification is answerable here, not in a browser.
    expect(node.textContent).toContain("Reply here");
  });

  it("shimmers while unsettled and lifts once the card lands", () => {
    const node = mount(<ConnectorToasts toasts={[toast()]} onOpen={() => {}} onDismiss={() => {}} />);
    expect(node.querySelector(".connector-shimmer")).not.toBeNull();
    act(() => {
      root!.render(<ConnectorToasts
        toasts={[toast({ headline: "Nina wants the release checklist reviewed", settled: true })]}
        onOpen={() => {}}
        onDismiss={() => {}}
      />);
    });
    expect(node.querySelector(".connector-settle")).not.toBeNull();
    expect(node.textContent).toContain("Nina wants the release checklist reviewed");
  });

  it("deep-links into the pane when the body is clicked", () => {
    const onOpen = vi.fn();
    const node = mount(<ConnectorToasts toasts={[toast()]} onOpen={onOpen} onDismiss={() => {}} />);
    const body = node.querySelector("button[aria-label]") as HTMLButtonElement;
    act(() => { body.click(); });
    expect(onOpen).toHaveBeenCalledWith(expect.objectContaining({ itemKey: "slack:D09:1.0" }));
  });

  it("dismisses only the toast that was dismissed", () => {
    const onDismiss = vi.fn();
    const node = mount(<ConnectorToasts
      toasts={[toast(), toast({ key: "slack:C07:2.0", itemKey: "slack:C07:2.0" })]}
      onOpen={() => {}}
      onDismiss={onDismiss}
    />);
    const close = node.querySelectorAll('button[aria-label="Dismiss notification"]')[1] as HTMLButtonElement;
    act(() => { close.click(); });
    expect(onDismiss).toHaveBeenCalledWith("slack:C07:2.0");
  });

  it("auto-expires after the TTL", () => {
    vi.useFakeTimers();
    const onDismiss = vi.fn();
    mount(<ConnectorToasts toasts={[toast()]} onOpen={() => {}} onDismiss={onDismiss} />);
    expect(onDismiss).not.toHaveBeenCalled();
    act(() => { vi.advanceTimersByTime(CONNECTOR_TOAST_TTL_MS + 10); });
    expect(onDismiss).toHaveBeenCalledWith("slack:D09:1.0");
  });

  it("caps the stack and counts the overflow instead of walling the screen", () => {
    const many = Array.from({ length: 6 }, (_, index) =>
      toast({ key: `slack:C${index}:1.0`, itemKey: `slack:C${index}:1.0` }));
    const node = mount(<ConnectorToasts toasts={many} onOpen={() => {}} onDismiss={() => {}} />);
    expect(node.querySelectorAll('button[aria-label="Dismiss notification"]')).toHaveLength(3);
    expect(node.textContent).toContain("+3 more waiting");
  });

  it("renders one toast for a message that arrives and then renders", () => {
    // The reducer and the component agree: an upgraded toast is the same node.
    let toasts = reduceToasts([], {
      type: "arrived",
      payload: { family: "slack", itemKey: "slack:D09:1.0", headline: "Nina sent you a direct message", channelLabel: "Nina", author: "Nina" },
    });
    toasts = reduceToasts(toasts, {
      type: "card",
      payload: { family: "slack", itemKey: "slack:D09:1.0", headline: "Nina wants the checklist reviewed", harnessRendered: true },
    });
    const node = mount(<ConnectorToasts toasts={toasts} onOpen={() => {}} onDismiss={() => {}} />);
    expect(node.querySelectorAll('button[aria-label="Dismiss notification"]')).toHaveLength(1);
    expect(node.textContent).toContain("Nina wants the checklist reviewed");
  });
});

describe("suppression", () => {
  it("shows nothing while the Inbox pane is the one on screen", () => {
    // The stack sits exactly where that pane's reply box is, and telling
    // someone about a message they are reading is noise.
    const node = mount(<ConnectorToasts toasts={[toast()]} suppressed onOpen={() => {}} onDismiss={() => {}} />);
    expect(node.textContent).toBe("");
  });

  it("still shows when some other pane is open", () => {
    const node = mount(<ConnectorToasts toasts={[toast()]} suppressed={false} onOpen={() => {}} onDismiss={() => {}} />);
    expect(node.textContent).toContain("Nina Alvarez");
  });
});
