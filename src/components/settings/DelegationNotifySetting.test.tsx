// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it } from "vitest";
import { bridgeApi } from "../../api";
import { DelegationNotifySetting } from "./DelegationNotifySetting";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

it("saves the global level and lets a chat override then clear it", async () => {
  const container = document.createElement("div");
  const root = createRoot(container);
  try {
    await act(async () => { root.render(<DelegationNotifySetting />); });
    const button = (label: string) => Array.from(container.querySelectorAll("button")).find(item => item.textContent === label)!;
    expect(button("Everything").getAttribute("aria-pressed")).toBe("true");
    await act(async () => { button("Actionable").click(); });
    expect(button("Actionable").getAttribute("aria-pressed")).toBe("true");
    expect((await bridgeApi.delegationNotifySettings("orch")).level).toBe("actionable");
    await bridgeApi.saveDelegationNotifySettings("orch", "all");
    expect(await bridgeApi.delegationNotifySettings("orch")).toMatchObject({ level: "all", globalLevel: "actionable", sessionOverride: "all" });
    await bridgeApi.saveDelegationNotifySettings("orch", null);
    expect((await bridgeApi.delegationNotifySettings("orch")).level).toBe("actionable");
  } finally { act(() => root.unmount()); }
});
