// @vitest-environment jsdom
import { act, useEffect, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it } from "vitest";
import { useResetOnChange } from "./useResetOnChange";

let root: Root;
let host: HTMLDivElement;

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

function Draft({ id, painted }: { id: string; painted: string[] }) {
  const [text, setText] = useState("typed");
  useResetOnChange([id], () => setText(""));
  useEffect(() => { painted.push(`${id}:${text}`); });
  return <button type="button" onClick={() => setText("typed")}>{text}</button>;
}

it("resets before the new key paints, and only when the key changes", () => {
  const painted: string[] = [];
  act(() => root.render(<Draft id="a" painted={painted} />));
  act(() => root.render(<Draft id="a" painted={painted} />));
  act(() => root.render(<Draft id="b" painted={painted} />));
  expect(painted).toEqual(["a:typed", "a:typed", "b:"]);
  act(() => host.querySelector("button")!.click());
  expect(host.textContent).toBe("typed");
});

it("compares every part of the key", () => {
  const resets: string[] = [];
  function Probe({ a, b }: { a: string; b: number }) {
    useResetOnChange([a, b], () => resets.push(`${a}${b}`));
    return null;
  }
  act(() => root.render(<Probe a="x" b={1} />));
  act(() => root.render(<Probe a="x" b={1} />));
  act(() => root.render(<Probe a="x" b={2} />));
  act(() => root.render(<Probe a="y" b={2} />));
  expect(resets).toEqual(["x2", "y2"]);
});
