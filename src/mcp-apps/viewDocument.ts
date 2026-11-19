import policy from "./view-csp.txt?raw";

export const VIEW_CSP = policy.trim();

/**
 * Applies the host's theme inside the view: its CSS variables
 * (`hostContext.styles.variables`) and color scheme, from the `ui/initialize`
 * answer and every `ui/notifications/host-context-changed`. A view written
 * to use `var(--color-text-primary)` and the rest works even when its own
 * script never applies them, which the personal-app eval found most
 * model-written views forget: they came out black text on white in dark
 * mode. It runs in the view's own sandbox and only sets style properties.
 */
const THEME_PRELUDE = `<script>(()=>{const apply=c=>{if(!c||typeof c!=="object")return;const root=document.documentElement;const vars=c.styles&&c.styles.variables;if(vars&&typeof vars==="object")for(const [name,value] of Object.entries(vars))if(name.startsWith("--")&&typeof value==="string")root.style.setProperty(name,value);if(c.theme==="light"||c.theme==="dark")root.style.colorScheme=c.theme;};addEventListener("message",event=>{if(event.source!==parent)return;const message=event.data;if(!message||typeof message!=="object")return;if(message.result&&message.result.hostContext)apply(message.result.hostContext);if(message.method==="ui/notifications/host-context-changed")apply(message.params);});})();</script>`;

/** Policy precedes all model markup, including documents without a head.
 *  The view also declares both color schemes and gets the theme prelude, so
 *  it follows Bridge's light and dark theme; its own CSS still wins. */
/** The host's theme when the view is built. */
export type InitialTheme = { theme: "light" | "dark"; variables: Record<string, string> };

/** The theme written into the document itself, for views that never send
 *  `ui/initialize` and so never get a host context. Bridge's own token
 *  values; anything that could leave the style rule is dropped. */
function initialStyle(initial: InitialTheme | undefined): string {
  if (!initial) return "";
  const declarations = Object.entries(initial.variables)
    .filter(([name, value]) => /^--[a-z0-9-]+$/i.test(name) && value && !/[;{}<>\\]/.test(value))
    .map(([name, value]) => `${name}:${value}`);
  return `<style>:root{color-scheme:${initial.theme === "dark" ? "dark" : "light"};${declarations.join(";")}}</style>`;
}

export function viewDocument(html: string, initial?: InitialTheme): string {
  const meta = `<meta http-equiv="Content-Security-Policy" content="${VIEW_CSP}"><meta name="color-scheme" content="light dark">${initialStyle(initial)}${THEME_PRELUDE}`;
  const prefix = html.match(/^\s*(<!doctype\s+html\s*>)/i)?.[0] ?? "<!doctype html>";
  return `${prefix}${meta}${html.replace(/^\s*<!doctype\s+html\s*>/i, "")}`;
}

/**
 * CSP alone does not stop a document navigating itself. The trusted outer
 * frame's frame-src policy governs the untrusted inner frame's navigation.
 * srcdoc is allowed; network navigation is blocked before any request.
 * Both frames have opaque origins. The host trusts only the outer source.
 */
export function personalViewDocument(html: string, initial?: InitialTheme): string {
  const source = JSON.stringify(viewDocument(html, initial)).replace(/</g, "\\u003c");
  return viewDocument(`<style>html,body{margin:0;height:100%;overflow:hidden}iframe{display:block;width:100%;height:100%;border:0}</style>
<iframe sandbox="allow-scripts" referrerpolicy="no-referrer" title="App preview"></iframe>
<script>
const inner=document.querySelector('iframe');
addEventListener('message',event=>{
  if(event.source===inner.contentWindow) parent.postMessage(event.data,'*');
  else if(event.source===parent) inner.contentWindow.postMessage(event.data,'*');
});
inner.srcdoc=${source};
</script>`);
}
