import { prepareDraft } from './file-issue.mjs';
import { authorized, buildContext } from './whatsapp.mjs';

export function createTicketer({ config, store, agent, filer, transport, report = () => {}, timeoutMs = 80000 }) {
  const running = new Map();
  const controllers = new Set();
  let queue = Promise.resolve();
  let accepting = true;

  async function receipt(source, issue) {
    await transport.react(source, '🎫');
    await transport.reply(source, `#${issue.number} ${issue.title} ${issue.url}`);
  }
  async function processSource(source) {
    const controller = new AbortController();
    controllers.add(controller);
    const timer = setTimeout(() => controller.abort(), timeoutMs);
    timer.unref?.();
    const signal = controller.signal;
    try {
      const previous = store.get(source.id);
      if (previous?.state === 'filed') { await receipt(source, previous.issue); return previous.issue; }
      await transport.react(source, '⏳');
      // A CLI timeout/crash can happen after GitHub has accepted the issue. Never
      // retry that write blindly: reconcile by a non-identifying receipt marker.
      if (previous?.state === 'creating') {
        const issue = await filer.find(source.id, signal);
        if (issue) {
          store.set(source.id, 'filed', issue);
          await receipt(source, issue);
          return issue;
        }
        await transport.react(source, '❌');
        await transport.reply(source, 'The previous filing is still unconfirmed. An operator needs to check GitHub before retrying.');
        return null;
      }
      store.set(source.id, 'drafting');
      const context = buildContext(source, store.prior(source).filter((message) => authorized(message.key, config)));
      const labels = await filer.labels(signal);
      const draft = await prepareDraft(agent, context, signal);
      signal.throwIfAborted();
      store.set(source.id, 'creating');
      const issue = await filer.create(draft, source.id, labels, signal);
      store.set(source.id, 'filed', issue);
      await receipt(source, issue);
      return issue;
    } catch {
      const state = store.get(source.id)?.state;
      if (state === 'filed') {
        report('Issue filed; WhatsApp receipt delivery failed. Re-trigger to resend it.');
        return store.get(source.id).issue;
      }
      if (state !== 'creating') store.set(source.id, 'failed');
      report(state === 'creating' ? 'GitHub filing is unconfirmed; automatic recreation is blocked' : 'Ticket drafting failed');
      try {
        await transport.react(source, '❌');
        await transport.reply(source, state === 'creating'
          ? 'Could not confirm filing. Re-trigger this message to check for the existing issue.'
          : "couldn't file this, try rephrasing");
      } catch { report('Could not deliver the failure receipt'); }
      return null;
    } finally { clearTimeout(timer); controllers.delete(controller); }
  }
  return {
    handle(source) {
      if (!accepting || !authorized(source?.key, config)) return Promise.resolve(null);
      if (running.has(source.id)) return running.get(source.id);
      if (running.size >= 20) return Promise.resolve(null);
      const task = queue.then(() => accepting ? processSource(source) : null);
      running.set(source.id, task);
      queue = task.catch(() => {});
      void task.finally(() => running.delete(source.id)).catch(() => {});
      return task;
    },
    async close() {
      accepting = false;
      for (const controller of controllers) controller.abort();
      await queue;
    },
  };
}
