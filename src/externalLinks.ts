import { open } from "@tauri-apps/plugin-shell";

// Anchors and window.open() targets a bare WKWebView/WebView2 has no browser
// chrome to open — without this they either no-op or spawn an empty app
// window. Routing through the shell plugin's `open` hands the URL to the OS
// default browser instead. Scheme-gated to match the shell:allow-open scope
// in capabilities/default.json.
const EXTERNAL_SCHEME = /^(https?|mailto):/i;

/** An anchor carrying this attribute always leaves the app, even when the
 * router could show it inside — it is how an "open on GitHub" affordance says
 * that leaving is the whole point of the click. */
const SYSTEM_BROWSER_ATTRIBUTE = "data-system-browser";

export function isExternalUrl(url: string): boolean {
  return EXTERNAL_SCHEME.test(url.trim());
}

/**
 * Decides whether a URL has a home inside Bridge. Returning true means the
 * router took it — it has navigated somewhere — and the browser is not
 * involved; false leaves it to the OS. It may answer asynchronously, because
 * deciding can mean asking the backend which repository a workspace is on.
 */
export type InternalLinkRouter = (url: string) => boolean | Promise<boolean>;

let internalRouter: InternalLinkRouter | undefined;

/** Installed by the surface that can navigate (App), cleared when it unmounts.
 * A module-level slot rather than a prop because the click interceptor is
 * document-wide and is installed before React mounts. */
export function setInternalLinkRouter(router: InternalLinkRouter | undefined): void {
  internalRouter = router;
}

/** Hands the URL to the OS default browser, router or no router. */
export async function openInSystemBrowser(url: string): Promise<void> {
  if (!isExternalUrl(url)) return;
  await open(url);
}

/** Opens a URL the way Bridge prefers: inside the app when something can show
 * it, in the OS default browser otherwise. */
export async function openExternalUrl(url: string): Promise<void> {
  if (!isExternalUrl(url)) return;
  if (internalRouter) {
    try {
      if (await internalRouter(url)) return;
    } catch {
      // A router that fails is a router that did not take the link; the
      // browser still gets it, so a bad route never strands a click.
    }
  }
  await open(url);
}

let handlerInstalled = false;

/** Installs a document-wide click interceptor so every rendered `<a href>` opens
 * through the router, and in the system browser when nothing claims it.
 * Installing twice would open every link twice, so the second call is a no-op. */
export function installExternalLinkHandler(): void {
  if (handlerInstalled) return;
  handlerInstalled = true;
  document.addEventListener("click", event => {
    if (event.defaultPrevented || event.button !== 0) return;
    const anchor = (event.target as HTMLElement | null)?.closest?.("a[href]");
    if (!anchor) return;
    const href = anchor.getAttribute("href") ?? "";
    if (!isExternalUrl(href)) return;
    event.preventDefault();
    if (anchor.hasAttribute(SYSTEM_BROWSER_ATTRIBUTE)) {
      void openInSystemBrowser(href);
      return;
    }
    void openExternalUrl(href);
  });
}
