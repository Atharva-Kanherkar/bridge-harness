import type { GetPersonalAppDraftParams, LockPersonalAppParams, LockPersonalAppResult, PersonalAppDraft, PreviewPersonalAppToolParams } from "./protocol/generated/protocol";

export const MOCK_APP_DRAFT_ID = "personal-app-demo";
export const MOCK_APP_HASH = `sha256:${"a".repeat(64)}`;
const html = `<!doctype html><html><head><meta charset="utf-8"><style>
body{margin:0;padding:20px;font:13px system-ui;color:var(--color-text-primary,#282828);background:var(--color-background-primary,#fff)}
h2{font-size:17px;margin:0 0 6px}p{margin:0 0 18px;opacity:.65}label{display:block;margin:12px 0 6px}
input,textarea,button{font:inherit;color:inherit;border:1px solid var(--color-border-primary,#ccc);border-radius:8px;padding:9px 11px;background:transparent}
input,textarea{box-sizing:border-box;width:100%}textarea{resize:vertical;min-height:68px}button{cursor:pointer;margin:10px 8px 0 0}
pre{font:12px ui-monospace;white-space:pre-wrap;word-break:break-word;margin:14px 0 0}
</style></head><body><h2>Message a teammate</h2><p>Find someone, then preview your message.</p>
<label for="query">Name or email</label><input id="query" placeholder="Search teammates"><button id="search">Find teammate</button>
<label for="message">Message</label><textarea id="message">The preview is ready for a look.</textarea><button id="send">Preview message</button><pre id="result" role="status"></pre>
<script>
let next=0;const waiting=new Map();const call=(method,params)=>new Promise((resolve,reject)=>{const id=++next;waiting.set(id,{resolve,reject});parent.postMessage({jsonrpc:'2.0',id,method,params},'*')});
const apply=context=>{if(context?.styles?.variables)for(const [key,value]of Object.entries(context.styles.variables))document.documentElement.style.setProperty(key,value)};
addEventListener('message',event=>{if(event.source!==parent)return;const msg=event.data;if(waiting.has(msg.id)){const task=waiting.get(msg.id);waiting.delete(msg.id);msg.error?task.reject(new Error(msg.error.message)):task.resolve(msg.result)}
if(msg.method==='ui/notifications/tool-input')document.querySelector('#query').value=msg.params.arguments.query??'';
if(msg.method==='ui/notifications/host-context-changed')apply(msg.params)});
const show=async(name,args)=>{try{const result=await call('tools/call',{name,arguments:args});document.querySelector('#result').textContent=result.content.map(part=>part.text??'').join('\\n')}catch(error){document.querySelector('#result').textContent=error.message}};
document.querySelector('#search').onclick=()=>show('slack_search_users',{query:document.querySelector('#query').value});
document.querySelector('#send').onclick=()=>show('slack_send_message',{recipient:document.querySelector('#query').value,text:document.querySelector('#message').value});
call('ui/initialize',{appInfo:{name:'slack-dm',version:'1'},appCapabilities:{},protocolVersion:'2026-01-26'}).then(result=>{apply(result.hostContext);parent.postMessage({jsonrpc:'2.0',method:'ui/notifications/initialized',params:{}},'*');new ResizeObserver(()=>parent.postMessage({jsonrpc:'2.0',method:'ui/notifications/size-changed',params:{height:document.body.scrollHeight}},'*')).observe(document.body)});
</script></body></html>`;
let locked = false;
export function mockGetPersonalAppDraft(params: GetPersonalAppDraftParams): PersonalAppDraft {
  if (params.sessionId !== "session-1" || params.draftId !== MOCK_APP_DRAFT_ID || params.contentHash !== MOCK_APP_HASH) throw new Error("This app preview is unavailable in this chat");
  return { draftId: MOCK_APP_DRAFT_ID, appId: "slack-dm", contentHash: MOCK_APP_HASH, locked, name: "Slack DM", intent: "Find and message a teammate without leaving the chat", harness: "claude", server: "claude.ai Slack", reads: ["slack_search_users"], writes: ["slack_send_message"], viewHtml: html, sampleArgs: { query: "Ada" } };
}
export function mockLockPersonalApp(params: LockPersonalAppParams): LockPersonalAppResult {
  const draft = mockGetPersonalAppDraft({ sessionId: "session-1", ...params });
  const kind = locked ? "unchanged" : "locked"; locked = true;
  return { kind, app: { id: draft.appId, version: 1, name: draft.name, intent: draft.intent, harness: draft.harness, server: draft.server, reads: draft.reads, writes: draft.writes, contentHash: draft.contentHash, viewBytes: html.length, lockedAt: new Date().toISOString() } };
}
export function mockPreviewPersonalAppTool(params: PreviewPersonalAppToolParams): unknown {
  const draft = mockGetPersonalAppDraft(params);
  if (draft.writes.includes(params.tool)) return { content: [{ type: "text", text: `Preview only — would send ${params.tool} to ${draft.server}. Nothing was sent.\n${JSON.stringify(params.arguments, null, 2)}` }], structuredContent: { preview: true, wouldCall: { server: draft.server, tool: params.tool, arguments: params.arguments } }, isError: false };
  if (draft.reads.includes(params.tool)) return { content: [{ type: "text", text: "Ada Lovelace · @ada · Engineering (demo result)" }], isError: false };
  throw new Error("This tool is outside the preview's read/write allowlist");
}
