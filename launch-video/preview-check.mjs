import assert from "node:assert/strict";
import { openBrowser } from "@remotion/renderer";
import { writeFile, mkdir } from "node:fs/promises";
import { dirname } from "node:path";
import { fileURLToPath } from "node:url";
process.chdir(dirname(fileURLToPath(import.meta.url)));
const errors = [];
const browser = await openBrowser("chrome", {
  chromiumOptions: { gl: "swangle" },
});
try {
  const page = await browser.newPage({
    context: () => null,
    logLevel: "error",
    indent: false,
    pageIndex: 0,
    onBrowserLog: (log) => {
      if (log.type === "error") errors.push(log.text);
    },
    onLog: () => {},
  });
  page.on("error", (e) => errors.push(String(e)));
  await page.setViewport({ width: 1440, height: 1080, deviceScaleFactor: 1 });
  await page.goto({
    url: process.env.PREVIEW_URL ?? "http://127.0.0.1:1473",
    timeout: 30000,
  });
  await new Promise((r) => setTimeout(r, 1500));
  for (const [label, expected] of [
    ["Introduction", "Your agents."],
    ["MEMORY", "Your way of"],
    ["Get Bridge", "Bring your agents."],
  ]) {
    const clicked = await page.evaluate((name) => {
      const button = [...document.querySelectorAll("button")].find((b) =>
        b.textContent.includes(name),
      );
      button?.click();
      return !!button;
    }, label);
    assert(clicked, `Chapter button ${label} exists`);
    await new Promise((r) => setTimeout(r, 800));
    const state = await page.evaluate(() => ({
      text: document.body.innerText,
      broken: [...document.images].filter((i) => !i.complete || !i.naturalWidth)
        .length,
      controls: [...document.querySelectorAll("button")].map((b) =>
        b.getAttribute("aria-label"),
      ),
    }));
    assert(
      state.text.includes(expected),
      `${label} renders its expected scene`,
    );
    assert.equal(state.broken, 0);
    console.log(`PASS: ${label}: chapter seek and playback, no broken images.`);
  }
  await page.evaluate(() =>
    document.querySelector('button[aria-label="Pause"]')?.click(),
  );
  await mkdir("out", { recursive: true });
  const screenshot = await page
    ._client()
    .send("Page.captureScreenshot", { format: "png" });
  await writeFile(
    "out/html-preview.png",
    Buffer.from(screenshot.value.data, "base64"),
  );
  assert.deepEqual(errors, []);
  console.log("PASS: preview has no browser errors.");
} finally {
  await browser.close({ silent: true });
}
