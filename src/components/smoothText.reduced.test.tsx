// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it } from "vitest";
import { useSmoothText } from "./smoothText";

function Probe({ text }: { text: string }) {
  return <p>{useSmoothText(text, true)}</p>;
}

it("shows streamed text at once under reduced motion", async () => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  // framer reads the preference once per module, so it is stubbed before the first render.
  window.matchMedia = (query: string) => ({
    matches: query.includes("prefers-reduced-motion"), media: query, onchange: null,
    addListener: () => undefined, removeListener: () => undefined,
    addEventListener: () => undefined, removeEventListener: () => undefined, dispatchEvent: () => false,
  });
  const host = document.createElement("div");
  const root = createRoot(host);
  await act(async () => root.render(<Probe text="Hi" />));
  await act(async () => root.render(<Probe text="Hi there, all at once" />));
  expect(host.textContent).toBe("Hi there, all at once");
  await act(async () => root.unmount());
});
