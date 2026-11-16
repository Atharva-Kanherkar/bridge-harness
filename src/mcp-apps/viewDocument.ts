import policy from "./view-csp.txt?raw";

export const VIEW_CSP = policy.trim();

/** Policy precedes all model markup, including documents without a head. */
export function viewDocument(html: string): string {
  const meta = `<meta http-equiv="Content-Security-Policy" content="${VIEW_CSP}">`;
  const prefix = html.match(/^\s*(<!doctype\s+html\s*>)/i)?.[0] ?? "<!doctype html>";
  return `${prefix}${meta}${html.replace(/^\s*<!doctype\s+html\s*>/i, "")}`;
}

/**
 * CSP alone does not stop a document navigating itself. The trusted outer
 * frame's frame-src policy governs the untrusted inner frame's navigation.
 * srcdoc is allowed; network navigation is blocked before any request.
 * Both frames have opaque origins. The host trusts only the outer source.
 */
export function personalViewDocument(html: string): string {
  const source = JSON.stringify(viewDocument(html)).replace(/</g, "\\u003c");
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
