// Run against `bun run dev --port 1462`, with an ordinary fresh Playwright
// profile. Optional external install: see docs/personal-apps.md.
import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
const { chromium, webkit } = await import(process.env.BRIDGE_PLAYWRIGHT_MODULE ?? "playwright");
const engine = process.env.BRIDGE_PREVIEW_BROWSER === "webkit" ? webkit : chromium;
const browser = await engine.launch({ headless: true });
try {
  const context = await browser.newContext({ viewport: { width: 1440, height: 1200 } });
  const page = await context.newPage();
  page.setDefaultTimeout(15000);
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  const base = process.argv[2] ?? "http://127.0.0.1:1462";
  await page.goto(base);
  await page.getByText("New chat", { exact: true }).first().click();
  const card = page.locator("[data-personal-app-state]");
  await card.waitFor();
  await card.scrollIntoViewIfNeeded();
  const app = card.locator("iframe").contentFrame().locator("iframe").contentFrame();
  await app.getByLabel("Name or email").waitFor();
  await page.waitForFunction(() => document.querySelector('[data-personal-app-state] iframe')?.style.height !== "320px");
  assert.equal(await app.getByLabel("Name or email").inputValue(), "Ada");
  await app.getByRole("button", { name: "Find teammate" }).click();
  await app.getByRole("status").filter({ hasText: "Ada Lovelace" }).waitFor();
  await app.getByRole("button", { name: "Preview message" }).click();
  await app.getByRole("status").filter({ hasText: "Nothing was sent" }).waitFor();
  await mkdir("docs/design/personal-apps", { recursive: true });
  for (const theme of ["light", "dark"]) {
    await page.evaluate(theme => document.documentElement.classList.toggle("dark", theme === "dark"), theme);
    await page.waitForTimeout(200);
    await card.screenshot({ path: `docs/design/personal-apps/${theme}.png` });
  }
  await card.getByRole("button", { name: "Revise", exact: true }).click();
  const composer = await page.locator('[contenteditable=true],textarea').evaluateAll(nodes => nodes.map(node => node.value ?? node.innerText).join("\n"));
  assert.match(composer, /Revise the personal app.*app_id: slack-dm/);
  await card.getByRole("button", { name: "Lock version", exact: true }).click();
  await card.getByRole("button", { name: "Version locked", exact: true }).waitFor();
  assert.deepEqual(errors, []);
  console.log("PASS preview samples, read result, write dry-run, Lock, Revise and both themes");

  // Use the actual production document builder. Route observation sees any
  // request before HTTP; CSP must block it before it reaches this handler.
  const probe = await context.newPage();
  const requests = [];
  probe.on("websocket", socket => { if (socket.url().includes("/preview-leak/")) requests.push(socket.url()); });
  await probe.route("**/preview-leak/**", route => { requests.push(route.request().url()); return route.abort(); });
  await probe.goto(base);
  const report = await probe.evaluate(async () => {
    const { personalViewDocument } = await import("/src/mcp-apps/viewDocument.ts");
    const html = `<style>@import url('/preview-leak/css');body{background:url('/preview-leak/background')}</style>
      <img src="/preview-leak/image"><iframe src="/preview-leak/frame"></iframe>
      <form action="/preview-leak/form"><button>Submit</button></form>
      <script>
      fetch('/preview-leak/fetch').catch(()=>{});
      try{new WebSocket(new URL('/preview-leak/socket',document.baseURI).href.replace(/^http/,'ws'))}catch{}
      try{document.querySelector('form').submit()}catch{}
      try{window.open('/preview-leak/popup')}catch{}
      let parentBlocked=false;try{parent.document.body}catch{parentBlocked=true}
      parent.postMessage({probe:true,parentBlocked},'*');
      setTimeout(()=>location.href='/preview-leak/navigation',20);
      </script>`;
    const frame = document.createElement("iframe");
    frame.sandbox = "allow-scripts";
    const started = new Promise(resolve => {
      const listener = event => {
        if (event.source !== frame.contentWindow || !event.data?.probe) return;
        removeEventListener("message", listener); resolve(event.data);
      };
      addEventListener("message", listener);
      setTimeout(() => resolve(null), 3000);
    });
    document.body.replaceChildren(frame);
    frame.srcdoc = personalViewDocument(html);
    return started;
  });
  assert.deepEqual(report, { probe: true, parentBlocked: true });
  await probe.waitForTimeout(150);
  assert.deepEqual(requests, []);
  assert.equal(context.pages().length, 2, "sandbox must not open a popup");
  console.log("PASS CSP: fetch, WebSocket, CSS, image, frame, form, popup, self-navigation and parent DOM blocked");
} finally {
  await browser.close();
}
