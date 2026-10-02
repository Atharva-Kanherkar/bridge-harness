// Apply both before inference and after inference. JIDs stay in the transport
// and private database; they never belong in prompts, issue text, or logs.
export function redact(text) {
  return String(text ?? '').normalize('NFKC')
    .replace(/[\u200b-\u200f\u202a-\u202e\u2060-\u2069\ufeff]/g, '')
    .replace(/[\w.+:-]+@(?:s\.whatsapp\.net|c\.us|lid|g\.us)/gi, '[redacted]')
    .replace(/\+?\p{Nd}(?:[ \t().-]*\p{Nd}){6,}/gu, '[phone removed]');
}

export function displayName(name) {
  const safe = redact(name).trim();
  return safe && !safe.includes('[phone removed]') && !safe.includes('[redacted]')
    ? safe.slice(0, 80) : 'Group member';
}

export function normalizeJid(jid) {
  return String(jid ?? '').replace(/:\d+@/, '@').replace(/@c\.us$/, '@s.whatsapp.net');
}
