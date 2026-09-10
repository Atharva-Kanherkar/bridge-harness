import { createInterface } from 'node:readline';
import { TerminalStateStore } from './state.mjs';

const store = new TerminalStateStore(process.argv[2]);
const input = createInterface({ input: process.stdin, crlfDelay: Infinity });
for await (const line of input) {
  try {
    const { op, key, ...args } = JSON.parse(line);
    let result;
    switch (op) {
      case 'create': result = await store.create(key, args.record); break;
      case 'write': case 'resize': result = await store.increment(key, { ...args, kind: op === 'write' ? 'data' : 'resize' }); break;
      case 'describe': result = await store.describe(key); break;
      case 'snapshot': result = await store.snapshot(key); break;
      case 'update': result = await store.update(key, args.changes); break;
      case 'list': result = await store.inventory(args.workspaceId); break;
      case 'layout': result = await store.layout(args.workspaceId, args.value); break;
      default: throw new Error('Unknown terminal state operation');
    }
    process.stdout.write(JSON.stringify({ result }) + '\n');
  } catch (error) { process.stdout.write(JSON.stringify({ error: String(error.message ?? error) }) + '\n'); }
}
store.dispose();
