import { displayName, normalizeJid, redact } from './privacy.mjs';

const PREFIX = /^(\/ticket|@bridge)\b/i;
export const sourceId = (key) => `${key.remoteJid}:${key.id}`;

export function authorized(key, config) {
  return key?.remoteJid === config.group && Boolean(key.id) && !key.fromMe &&
    [key.participant, key.participantAlt, key.participantPn, key.participantLid]
      .some((jid) => config.senders.has(normalizeJid(jid)));
}

function contentOf(message) {
  for (let i = 0; i < 3; i++) {
    const inner = message?.ephemeralMessage?.message ?? message?.viewOnceMessage?.message ??
      message?.viewOnceMessageV2?.message;
    if (!inner) break;
    message = inner;
  }
  return message;
}

function textOf(message) {
  const content = contentOf(message);
  return content?.conversation ?? content?.extendedTextMessage?.text ?? '';
}

export function toSource(message, config, now = Date.now()) {
  if (!authorized(message?.key, config)) return null;
  const text = textOf(message.message);
  if (typeof text !== 'string' || !text.trim()) return null;
  const timestamp = Number(message.messageTimestamp) * 1000;
  if (!Number.isFinite(timestamp) || timestamp > now + 60_000) return null;
  const context = contentOf(message.message)?.extendedTextMessage?.contextInfo;
  const quotedText = textOf(context?.quotedMessage);
  const quoteAllowed = context && (!context.remoteJid || context.remoteJid === config.group) &&
    config.senders.has(normalizeJid(context.participant));
  return {
    id: sourceId(message.key), key: message.key, timestamp,
    name: displayName(message.pushName), text: redact(text).slice(0, 8000),
    quoted: quoteAllowed && quotedText ? { name: 'Quoted group member', text: redact(quotedText).slice(0, 8000) } : null,
  };
}

export function isTextTrigger(source) { return Boolean(source && PREFIX.test(source.text)); }

// Baileys' outer key identifies the reacted-to message. reaction.key identifies
// the reaction's author: authorizing the outer key would authorize the wrong human.
export function reactionTarget(event, config) {
  if (event?.reaction?.text !== '🐛' || !authorized(event.reaction.key, config) ||
      event.key?.remoteJid !== config.group || !event.key.id || event.key.fromMe) return null;
  return sourceId(event.key);
}

export function buildContext(source, prior = []) {
  const clean = (message) => ({ name: displayName(message.name), text: redact(message.text).slice(0, 8000) });
  return {
    source: clean(source), quoted: source.quoted ? clean(source.quoted) : null,
    prior: prior.filter((message) => message.id !== source.id && message.timestamp <= source.timestamp &&
      message.timestamp >= source.timestamp - 15 * 60 * 1000)
      .sort((a, b) => a.timestamp - b.timestamp).slice(-10).map(clean),
  };
}

export function createHandlers({ config, store, handle, report = () => {} }) {
  const submit = (source) => { void handle(source).catch(() => report('Ticket handler failed')); };
  return {
    upsert({ messages = [], type }) {
      for (const message of messages) {
        const source = toSource(message, config);
        if (!source) continue;
        store.remember(source);
        if (type === 'notify' && Date.now() - source.timestamp <= 15 * 60 * 1000 && isTextTrigger(source)) submit(source);
      }
    },
    reaction(events) {
      for (const event of events) {
        const id = reactionTarget(event, config);
        if (!id) continue;
        const source = store.source(id);
        // Recheck after restart/config changes; old authorization is not a grant.
        if (source && authorized(source.key, config)) submit(source);
      }
    },
  };
}

export async function connectWhatsApp({ config, handlers, report = () => {}, onFatal = () => {} }) {
  const [{ default: makeSocket, useMultiFileAuthState, DisconnectReason }, { default: pino }, { default: qr }] =
    await Promise.all([import('@whiskeysockets/baileys'), import('pino'), import('qrcode-terminal')]);
  const { state, saveCreds } = await useMultiFileAuthState(config.authDir);
  let socket;
  let stopped = false;
  let retry;
  let attempts = 0;
  let saveQueue = Promise.resolve();
  const safeHandler = (fn) => (...args) => {
    try { fn(...args); } catch { report('WhatsApp event failed'); }
  };
  function start() {
    if (stopped) return;
    const current = makeSocket({ auth: state, logger: pino({ level: 'silent' }),
      markOnlineOnConnect: false, syncFullHistory: false, shouldSyncHistoryMessage: () => false });
    socket = current;
    current.ev.on('creds.update', () => {
      saveQueue = saveQueue.then(saveCreds).catch(() => {
        report('Could not persist WhatsApp credentials'); stop(); onFatal();
      });
    });
    current.ev.on('messages.upsert', safeHandler(handlers.upsert));
    current.ev.on('messages.reaction', safeHandler(handlers.reaction));
    current.ev.on('connection.update', ({ connection, lastDisconnect, qr: code }) => {
      if (stopped || current !== socket) return;
      if (code) { report('Link the dedicated WhatsApp account with this QR code'); qr.generate(code, { small: true }); }
      if (connection === 'open') { attempts = 0; report('WhatsApp connected'); }
      if (connection === 'close') {
        const status = lastDisconnect?.error?.output?.statusCode;
        for (const event of ['messages.upsert', 'messages.reaction', 'connection.update', 'creds.update']) current.ev.removeAllListeners(event);
        if ([DisconnectReason.loggedOut, DisconnectReason.badSession, DisconnectReason.connectionReplaced].includes(status)) {
          report('WhatsApp session stopped; relink or stop the other device session'); stop(); onFatal();
        } else {
          report('WhatsApp disconnected; reconnecting');
          retry = setTimeout(start, Math.min(30_000, 1000 * 2 ** Math.min(attempts++, 5)));
        }
      }
    });
  }
  function stop() { stopped = true; clearTimeout(retry); socket?.end(undefined); }
  async function send(content, options) {
    let timeout;
    try {
      return await Promise.race([
        socket.sendMessage(config.group, content, options),
        new Promise((_resolve, reject) => {
          timeout = setTimeout(() => reject(new Error('WhatsApp delivery timed out')), 5000);
        }),
      ]);
    } finally { clearTimeout(timeout); }
  }
  start();
  return {
    react: (source, text) => send({ react: { text, key: source.key } }),
    reply: (source, text) => send({ text }, {
      quoted: { key: source.key, message: { conversation: source.text } },
    }),
    close: async () => { stop(); await saveQueue; },
  };
}
