import { open } from "@tauri-apps/plugin-shell";

// Anchors and window.open() targets a bare WKWebView/WebView2 has no browser
// chrome to open — without this they either no-op or spawn an empty app
// window. Routing through the shell plugin's `open` hands the URL to the OS
// default browser instead. Scheme-gated to match the shell:allow-open scope
// in capabilities/default.json.
const EXTERNAL_SCHEME = /^(https?|mailto):/i;

export function isExternalUrl(url: string): boolean {
  return EXTERNAL_SCHEME.test(url.trim());
}

export async function openExternalUrl(url: string): Promise<void> {
  if (!isExternalUrl(url)) return;
  await open(url);
}

/** Installs a document-wide click interceptor so every rendered `<a href>` opens in the system browser. */
export function installExternalLinkHandler(): void {
  document.addEventListener("click", event => {
    if (event.defaultPrevented || event.button !== 0) return;
    const anchor = (event.target as HTMLElement | null)?.closest?.("a[href]");
    if (!anchor) return;
    const href = anchor.getAttribute("href") ?? "";
    if (!isExternalUrl(href)) return;
    event.preventDefault();
    void openExternalUrl(href);
  });
}
