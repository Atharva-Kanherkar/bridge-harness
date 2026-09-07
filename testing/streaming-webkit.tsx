/** Synthetic desktop WKWebView replay. No provider, credentials or application database. */
import React from "react";
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { AgentConversation } from "../src/components/AgentConversation";
import { alignTurns, foldWorkerDelegations, groupItems, mergeConversationProjections, projectSessionConversation, reduceConversation } from "../src/conversation";
import { appendAgentEventBatch } from "../src/agentEvents";
import { createDisplayScheduler } from "../src/displayScheduler";
import { asWireKind } from "../src/transcript/wire";
import type { AgentEvent, SessionEntry } from "../src/types";
import "../src/index.css";

declare global { interface Window { webkit?: { messageHandlers: { benchmark: { postMessage(value: unknown): void } } } } }
const pause = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));
const median = (xs: number[]) => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)];
function history(size: number): SessionEntry[] {
  return Array.from({ length: size }, (_, i) => ({ id: `e${i}`, sessionId: "s", parentEntryId: i ? `e${i - 1}` : null,
    sequence: i + 1, semanticSchemaVersion: 2, kind: i % 2 ? "assistant.message" : "user.message",
    payload: { itemId: `m${i}`, text: `Historical message ${i}`, role: i % 2 ? "assistant" : "user", status: "completed" },
    providerEventId: null, contextVisibility: "eligible", tokenEstimate: null, createdAt: "now" }));
}
const event = (kind: string, text: string, sessionId = "s"): AgentEvent => ({ id: 0, sequence: 0, sessionId,
  protocolVersion: 1, kind: asWireKind(kind), itemId: kind.startsWith("reasoning") ? "thought" : "answer", role: "assistant",
  text, status: kind.endsWith("completed") ? "completed" : "streaming", title: null, data: {}, providerMeta: {}, createdAt: "now" });

async function run() {
  const root = createRoot(document.getElementById("root")!);
  const projection = [];
  for (const size of [100, 1000, 5000]) {
    const entries = history(size), leaf = `e${size - 1}`;
    const live = reduceConversation(entries.slice(-1000).map((entry, i) => ({ ...event("message.completed", String(entry.payload.text)),
      id: i + 1, sequence: i + 1, itemId: String(entry.payload.itemId), role: String(entry.payload.role) })));
    const times: number[] = [];
    for (let i = 0; i < 18; i++) {
      const start = performance.now();
      groupItems(alignTurns(foldWorkerDelegations(mergeConversationProjections(projectSessionConversation(entries, leaf), live))));
      if (i >= 3) times.push(performance.now() - start);
    }
    projection.push({ entries: size, medianProjectionMs: median(times) });
  }
  window.webkit?.messageHandlers.benchmark.postMessage({ progress: JSON.stringify(projection) });
  const rendering = [];
  for (const [size, workload] of [[100, "text"], [5000, "text"], [100, "reasoning"], [100, "code"], [100, "workers"]] as const) {
    const entries = history(size), leaf = `e${size - 1}`;
    let live: AgentEvent[] = [], deliveries = 0, maxPending = 0;
    const commits: number[] = [], tickDelays: number[] = [];
    const render = () => flushSync(() => root.render(<AgentConversation preview forestEntries={entries} activeLeafId={leaf}
      events={live.filter(e => e.sessionId === "s")} onResolve={() => undefined} working />));
    render();
    if (!document.querySelector("[data-conversation-content]")?.textContent?.includes(`Historical message ${size - 1}`)) {
      throw new Error("Replay did not mount the actual transcript rows");
    }
    window.webkit?.messageHandlers.benchmark.postMessage({ progress: `mounted ${size} ${workload}` });
    await pause(50);
    const scheduler = createDisplayScheduler(batch => {
      const start = performance.now();
      live = appendAgentEventBatch(live, batch);
      render();
      commits.push(performance.now() - start);
      deliveries++;
    }, { frame: cb => window.requestAnimationFrame(cb), cancelFrame: id => window.cancelAnimationFrame(id),
      timeout: (cb, ms) => window.setTimeout(cb, ms), cancelTimeout: id => window.clearTimeout(id) });
    let nextTick = performance.now();
    const ticker = window.setInterval(() => {
      tickDelays.push(Math.max(0, performance.now() - nextTick - 16));
      nextTick = performance.now();
    }, 16);
    let text = workload === "code" ? "```typescript\n" : "";
    if (text) scheduler.push(event("message.delta", text));
    const started = performance.now();
    for (let i = 0; i < 120; i++) {
      const delta = workload === "code" ? `const row${i} = ${i}; // streamed code sample\n`.repeat(5) : "streamed text ";
      text += delta;
      const kind = workload === "reasoning" ? "reasoning.delta" : "message.delta";
      scheduler.push(event(kind, delta));
      if (workload === "workers") for (const session of ["w1", "w2", "w3"]) scheduler.push(event(kind, delta, session));
      maxPending = Math.max(maxPending, scheduler.pendingCount());
      await pause(8);
    }
    scheduler.push(event(workload === "reasoning" ? "reasoning.completed" : "message.completed", text + (workload === "code" ? "\n```" : "")));
    const remaining = scheduler.pendingCount();
    scheduler.dispose(); clearInterval(ticker);
    rendering.push({ entries: size, workload, elapsedMs: performance.now() - started, deliveries,
      medianCommitMs: median(commits), maxCommitMs: Math.max(...commits), maxEventLoopDelayMs: Math.max(...tickDelays), maxPending, remaining });
  }
  window.webkit?.messageHandlers.benchmark.postMessage({ progress: JSON.stringify(rendering) });
  root.unmount();
  return { userAgent: navigator.userAgent, projection, rendering,
    limitations: "Synthetic WKWebView using production transcript and scheduler. Event-loop delay is a responsiveness proxy; actual composer typing, Stop RPC and scroll anchoring require full-app checks." };
}
run().then(result => {
  document.body.textContent = JSON.stringify(result, null, 2);
  window.webkit?.messageHandlers.benchmark.postMessage(result);
}).catch(error => window.webkit?.messageHandlers.benchmark.postMessage({ error: String(error) }));
