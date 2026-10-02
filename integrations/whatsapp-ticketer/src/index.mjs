import { chmod } from 'node:fs/promises';
import { createAgent, snapshotRepository } from './agent.mjs';
import { loadConfig } from './config.mjs';
import { createFiler } from './file-issue.mjs';
import { openStore } from './store.mjs';
import { createTicketer } from './service.mjs';
import { connectWhatsApp, createHandlers } from './whatsapp.mjs';

process.umask(0o077);
const report = (message) => console.error(`[whatsapp-ticketer] ${message}`);
let snapshot, store, ticketer, connection;
let closing;
async function close() {
  if (!closing) closing = (async () => {
    await ticketer?.close();
    await connection?.close();
    store?.close();
    await snapshot?.close();
  })();
  return closing;
}
try {
  const config = loadConfig();
  await chmod(config.authDir, 0o700);
  store = await openStore(config.dbPath);
  snapshot = await snapshotRepository(config.repoDir);
  const agent = await createAgent({ root: snapshot.root, model: config.model });
  const filer = createFiler();
  await filer.labels(); // Fail before linking if GitHub auth/label setup is incomplete.
  const transport = {
    react: (...args) => connection.react(...args),
    reply: (...args) => connection.reply(...args),
  };
  ticketer = createTicketer({ config, store, agent, filer, transport, report });
  connection = await connectWhatsApp({ config, report,
    handlers: createHandlers({ config, store, handle: ticketer.handle, report }),
    onFatal: () => { process.exitCode = 1; void close(); },
  });
  for (const event of ['SIGINT', 'SIGTERM']) process.once(event, () => { void close(); });
} catch {
  // SDK/CLI/transport errors can embed credentials and chat text. Keep diagnostics
  // categorical; startup requirements and recovery steps are in the README.
  report('Startup failed. Check Node/gh, configuration paths, main clone, database lock, GitHub auth and from-whatsapp label.');
  process.exitCode = 1;
  await close();
}
