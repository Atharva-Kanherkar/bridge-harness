import test from 'node:test';
import assert from 'node:assert/strict';
import { authorized, buildContext, createHandlers, isTextTrigger, reactionTarget, toSource } from '../src/whatsapp.mjs';
import { redact } from '../src/privacy.mjs';

const config = { group: '123-456@g.us', senders: new Set(['919876543210@s.whatsapp.net']) };
const now = Date.now();
const key = { remoteJid: config.group, participant: [...config.senders][0], id: 'SOURCE', fromMe: false };
const message = (text = '/ticket sidebar flickers', overrides = {}) => ({ key, pushName: 'Ayush',
  message: { conversation: text }, messageTimestamp: Math.floor(now / 1000), ...overrides });

test('explicit prefixes only, authorized group and author, device and LID keys', () => {
  for (const text of ['/ticket sidebar flickers', '@bridge please add this', '/ticket\nBug']) {
    assert.equal(isTextTrigger(toSource(message(text), config, now)), true);
  }
  for (const text of ['normal chat', 'please /ticket this', '/ticketing something', '@bridges hello', ' /ticket bug']) {
    assert.equal(isTextTrigger(toSource(message(text), config, now)), false);
  }
  for (const change of [{ remoteJid: '999@g.us' }, { participant: '111@s.whatsapp.net' }, { fromMe: true }, { id: null }]) {
    assert.equal(toSource(message(undefined, { key: { ...key, ...change } }), config, now), null);
  }
  assert.equal(authorized({ ...key, participant: '919876543210:9@s.whatsapp.net' }, config), true);
  assert.equal(authorized({ ...key, participant: '12345@lid', participantPn: key.participant }, config), true);
  assert.equal(toSource(message(undefined, { message: { imageMessage: { caption: '/ticket bug' } } }), config, now), null);
});

test('reaction author is reaction.key, target is the outer key', () => {
  const event = { key, reaction: { key: { ...key, id: 'REACTION' }, text: '🐛' } };
  assert.equal(reactionTarget(event, config), `${config.group}:SOURCE`);
  for (const text of ['', '🎫', '⏳', '❌']) assert.equal(reactionTarget({ ...event, reaction: { ...event.reaction, text } }, config), null);
  for (const change of [{ participant: '111@s.whatsapp.net' }, { remoteJid: '999@g.us' }, { fromMe: true }]) {
    assert.equal(reactionTarget({ ...event, reaction: { ...event.reaction, key: { ...event.reaction.key, ...change } } }, config), null);
  }
  assert.equal(reactionTarget({ ...event, key: { ...key, remoteJid: '999@g.us' } }, config), null);
});

test('context includes quote, nearest ten prior messages, and no phone numbers/JIDs', () => {
  const source = toSource(message(undefined, { pushName: '+91 98765 43210', message: { extendedTextMessage: {
    text: '/ticket call +1 (415) 555-0123 or 919876543210@s.whatsapp.net',
    contextInfo: { participant: key.participant, quotedMessage: { conversation: 'Reply to +44 7700 900123' } },
  } } }), config, now);
  const prior = Array.from({ length: 15 }, (_, i) => ({ id: String(i), timestamp: source.timestamp - (15 - i) * 1000,
    name: 'Name', text: String(i) }));
  prior.push({ id: 'old', timestamp: source.timestamp - 900001, text: 'old' });
  prior.push({ id: 'future', timestamp: source.timestamp + 1, text: 'future' });
  const context = buildContext(source, prior);
  assert.equal(context.source.name, 'Group member');
  assert.equal(context.prior.length, 10);
  assert.equal(context.prior[0].text, '5');
  assert.ok(context.quoted.text.includes('[phone removed]'));
  assert.doesNotMatch(JSON.stringify(context), /98765|415|7700|s\.whatsapp|g\.us/);
  assert.equal(redact('９１９８７６５４３２１０ ९१९८७६५४३२१०'), '[phone removed]');
});

test('unauthorized quotes and quoted messages from other groups are excluded', () => {
  for (const context of [{ participant: '111@s.whatsapp.net' }, { participant: key.participant, remoteJid: '999@g.us' }]) {
    const source = toSource(message(undefined, { message: { extendedTextMessage: { text: '/ticket bug',
      contextInfo: { ...context, quotedMessage: { conversation: 'private' } } } } }), config, now);
    assert.equal(source.quoted, null);
  }
});

test('normal chat is cached without inference; append/history never files; reaction requires a cached authorized source', async () => {
  const cache = new Map();
  const calls = [];
  const handlers = createHandlers({ config,
    store: { remember: (source) => cache.set(source.id, source), source: (id) => cache.get(id) },
    handle: async (source) => calls.push(source.id),
  });
  handlers.upsert({ type: 'notify', messages: [message('normal chat')] });
  assert.equal(calls.length, 0);
  handlers.upsert({ type: 'append', messages: [message('/ticket old')] });
  assert.equal(calls.length, 0);
  handlers.upsert({ type: 'notify', messages: [message('/ticket bug', { key: { ...key, participant: '111@s.whatsapp.net' } })] });
  handlers.reaction([{ key, reaction: { text: '🐛', key: { ...key, participant: '111@s.whatsapp.net' } } }]);
  assert.equal(calls.length, 0);
  handlers.reaction([{ key, reaction: { text: '🐛', key: { ...key, id: 'REACTION' } } }]);
  assert.equal(calls.length, 1);
  handlers.reaction([{ key: { ...key, id: 'MISSING' }, reaction: { text: '🐛', key } }]);
  assert.equal(calls.length, 1);
});
