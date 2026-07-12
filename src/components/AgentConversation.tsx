import { AlertTriangle, Bot, Check, ChevronRight, Circle, FileDiff, FileText, LoaderCircle, LockKeyhole, Search, Sparkles, TerminalSquare, Wrench, X } from "lucide-react";
import { reduceConversation, type ConversationItem } from "../conversation";
import type { AgentEvent, Session } from "../types";

export function AgentConversation({ session, events, onResolve }: { session?: Session; events: AgentEvent[]; onResolve: (eventId:number,decision:string)=>void }) {
  const items = reduceConversation(events);
  if (!session) return <div className="conversation-empty"><Bot size={24}/><h2>Choose a structured agent</h2><p>Harness adapters turn their native event streams into one Bridge conversation protocol.</p></div>;
  if (!items.length) return <div className="conversation-empty"><div className={`harness-icon ${session.harness}`}><Bot size={15}/></div><h2>Start with a task</h2><p>{session.label} is connected through a structured adapter. Messages, tools, approvals, plans, and diffs will appear here—never its terminal UI.</p><div className="primitive-row"><span>MESSAGES</span><span>TOOLS</span><span>PLANS</span><span>APPROVALS</span><span>DIFFS</span></div></div>;
  return <div className="conversation-scroll"><div className="adapter-banner"><Sparkles size={12}/><span>{session.label} · STRUCTURED ADAPTER</span><i/>Protocol v1</div>{items.map(item=><ConversationItemView key={item.key} item={item} onResolve={onResolve}/>)}</div>;
}

function ConversationItemView({ item, onResolve }: { item: ConversationItem; onResolve:(eventId:number,decision:string)=>void }) {
  if (item.type === "message") return <article className={`chat-message ${item.role ?? "assistant"}`}><div className="message-author">{item.role === "user" ? "YOU" : "BRIDGE AGENT"}{item.status === "streaming" && <LoaderCircle className="spin" size={11}/>}</div><div className="message-text">{item.text}</div></article>;
  if (item.type === "reasoning") return <details className="reasoning-card"><summary><Sparkles size={13}/><span>Reasoning</span><small>{item.status === "streaming" ? "thinking…" : "summary"}</small><ChevronRight size={13}/></summary><div>{item.text || stringList(item.data.summary)}</div></details>;
  if (item.type === "plan") return <article className="plan-card"><header><FileText size={14}/><div><b>{item.title || "Plan"}</b><small>LIVE PLAN</small></div></header>{planSteps(item.data).map((step,index)=><div className="plan-step" key={`${step.step}-${index}`}>{step.status === "completed" ? <Check size={12}/> : step.status === "inProgress" ? <LoaderCircle className="spin" size={12}/> : <Circle size={9}/>}<span>{step.step}</span><small>{step.status}</small></div>)}</article>;
  if (item.type === "approval") return <article className="approval-card"><header><LockKeyhole size={15}/><div><b>{item.title}</b><small>NEEDS YOUR DECISION</small></div></header>{item.text && <p>{item.text}</p>}<ApprovalDetails data={item.data}/>{item.status === "pending" ? <div className="approval-actions"><button onClick={()=>onResolve(item.eventId,"decline")}><X size={12}/> Decline</button><button onClick={()=>onResolve(item.eventId,"acceptForSession")}>Allow for session</button><button className="approve" onClick={()=>onResolve(item.eventId,"accept")}><Check size={12}/> Allow once</button></div> : <div className="approval-resolved"><Check size={12}/> Resolved · {item.status}</div>}</article>;
  if (item.type === "error") return <article className="error-card"><AlertTriangle size={15}/><div><b>Agent error</b><p>{item.text || "The adapter reported an error."}</p></div></article>;
  if (item.type === "diff") return <ActivityCard item={item} icon={<FileDiff size={14}/>} label="FILE CHANGES"/>;
  if (item.type === "artifact") return <ActivityCard item={item} icon={<FileText size={14}/>} label="ARTIFACT"/>;
  const isCommand = item.data.type === "commandExecution" || item.title?.includes("/");
  return <ActivityCard item={item} icon={isCommand?<TerminalSquare size={14}/>:item.data.type === "webSearch"?<Search size={14}/>:<Wrench size={14}/>} label={isCommand?"COMMAND":"TOOL"}/>;
}

function ActivityCard({item,icon,label}:{item:ConversationItem;icon:React.ReactNode;label:string}) {
  const output = String(item.data.aggregatedOutput ?? item.data.output ?? "");
  return <article className="activity-card"><header><span>{icon}</span><div><small>{label}</small><b>{item.title || readableType(String(item.data.type ?? "activity"))}</b></div><em className={item.status}>{item.status ?? "complete"}</em></header>{item.text && <p>{item.text}</p>}{item.data.cwd ? <code>{String(item.data.cwd)}</code>:null}{output && <pre>{output.slice(-4000)}</pre>}</article>;
}

function ApprovalDetails({data}:{data:Record<string,unknown>}) { return <div className="approval-details">{data.command ? <><label>COMMAND</label><code>{String(data.command)}</code></>:null}{data.cwd ? <><label>WORKING DIRECTORY</label><code>{String(data.cwd)}</code></>:null}</div>; }
function readableType(value:string){return value.replace(/([a-z])([A-Z])/g,"$1 $2").replaceAll("_"," ");}
function stringList(value:unknown){return Array.isArray(value)?value.join("\n"):"";}
function planSteps(data:Record<string,unknown>):Array<{step:string;status:string}>{return Array.isArray(data.plan)?data.plan.filter((v):v is {step:string;status:string}=>!!v&&typeof v==="object"&&"step" in v&&"status" in v):[];}
