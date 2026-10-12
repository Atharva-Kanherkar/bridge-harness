import test from 'node:test';
import assert from 'node:assert/strict';
import install, { READY } from '../../src-tauri/bridge-core/src/pi_adapter/permission.mjs';
function fixture(confirm) {
  const handlers = new Map();
  install({ on: (name, handler) => handlers.set(name, handler) });
  return { handlers, ctx: { mode: 'rpc', hasUI: true, ui: { confirm } } };
}
for (const toolName of ['read', 'bash', 'edit', 'write', 'unknown']) {
  test(`gates ${toolName} before execution and grants only literal true`, async () => {
    for (const answer of [true, false, undefined, 'true']) {
      let asked;
      const { handlers, ctx } = fixture(async (...args) => { asked = args; return answer; });
      const result = await handlers.get('tool_call')({ toolName, toolCallId: 'call-1', input: { command: 'echo ok' } }, ctx);
      assert.equal(result?.block, answer === true ? undefined : true);
      assert.deepEqual(JSON.parse(asked[0]), { bridgePermission: 1, toolCallId: 'call-1', toolName });
      assert.deepEqual(JSON.parse(asked[1]), { command: 'echo ok' });
      assert.equal(asked[2].timeout, 120000);
    }
  });
}
test('missing UI, wrong mode, and thrown UI all deny', async () => {
  const { handlers, ctx } = fixture(async () => { throw Error('private diagnostic'); });
  for (const candidate of [ctx, { ...ctx, hasUI: false }, { ...ctx, mode: 'tui' }]) {
    assert.equal((await handlers.get('tool_call')({ toolName: 'bash', input: {} }, candidate)).block, true);
  }
});
test('readiness is announced only on the RPC UI channel', () => {
  const { handlers, ctx } = fixture();
  const notices = [];
  ctx.ui.notify = (...args) => notices.push(args);
  handlers.get('session_start')({}, ctx);
  handlers.get('session_start')({}, { ...ctx, hasUI: false });
  assert.deepEqual(notices, [[READY, 'info']]);
});
test('user input cannot reload the gate or bypass it with direct bash', () => {
  const { handlers } = fixture();
  for (const text of ['/reload', ' !rm file', '/model test']) assert.equal(handlers.get('input')({ text }).action, 'handled');
  assert.equal(handlers.get('input')({ text: 'Review /file and explain' }).action, 'continue');
  assert.equal(handlers.get('user_bash')().result.exitCode, 1);
});
