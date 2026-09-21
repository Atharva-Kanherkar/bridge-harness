#!/usr/bin/env node

import {
  createHash,
  createPublicKey,
  verify as verifyEd25519,
} from "node:crypto";
import { readFileSync } from "node:fs";
import { basename } from "node:path";

const BASE64 = /^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/;
const ED25519_SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex");

function decodeBase64(value, label) {
  if (typeof value !== "string") throw new Error(`${label} is not a string`);
  const encoded = value.trim();
  if (!encoded || !BASE64.test(encoded)) throw new Error(`${label} is not valid base64`);
  const decoded = Buffer.from(encoded, "base64");
  if (decoded.toString("base64") !== encoded) throw new Error(`${label} is not canonical base64`);
  return decoded;
}

function decodeTextBox(encoded, label, lineCount) {
  let text;
  try {
    text = new TextDecoder("utf-8", { fatal: true }).decode(decodeBase64(encoded, label));
  } catch (error) {
    throw new Error(`${label} cannot be decoded: ${error.message}`);
  }
  const lines = text.replaceAll("\r\n", "\n").replace(/\n+$/, "").split("\n");
  if (lines.length !== lineCount) throw new Error(`${label} has an invalid Minisign box`);
  return lines;
}

function parsePublicKey(encodedPublicKey) {
  const lines = decodeTextBox(encodedPublicKey, "updater public key", 2);
  if (!lines[0].startsWith("untrusted comment: ")) {
    throw new Error("updater public key has no Minisign comment");
  }
  const payload = decodeBase64(lines[1], "Minisign public key payload");
  if (payload.length !== 42 || payload[0] !== 0x45 || !new Set([0x44, 0x64]).has(payload[1])) {
    throw new Error("updater public key has an unsupported Minisign payload");
  }
  return { keyId: payload.subarray(2, 10), key: payload.subarray(10) };
}

function parseSignature(encodedSignature) {
  const lines = decodeTextBox(encodedSignature, "updater signature", 4);
  if (!lines[0].startsWith("untrusted comment: ")) {
    throw new Error("updater signature has no Minisign comment");
  }
  if (!lines[2].startsWith("trusted comment: ")) {
    throw new Error("updater signature has no trusted Minisign comment");
  }
  const payload = decodeBase64(lines[1], "Minisign signature payload");
  const globalSignature = decodeBase64(lines[3], "Minisign global signature");
  if (payload.length !== 74 || globalSignature.length !== 64 || payload[0] !== 0x45) {
    throw new Error("updater signature has an invalid Minisign payload");
  }
  if (!new Set([0x44, 0x64]).has(payload[1])) {
    throw new Error("updater signature uses an unsupported Minisign algorithm");
  }
  return {
    prehashed: payload[1] === 0x44,
    keyId: payload.subarray(2, 10),
    signature: payload.subarray(10),
    trustedComment: lines[2].slice("trusted comment: ".length),
    globalSignature,
  };
}

export function verifyUpdaterSignatureBytes(data, encodedSignature, encodedPublicKey) {
  if (!Buffer.isBuffer(data)) throw new Error("updater archive is not a byte buffer");
  const publicKey = parsePublicKey(encodedPublicKey);
  const signature = parseSignature(encodedSignature);
  if (!publicKey.keyId.equals(signature.keyId)) {
    throw new Error("updater signature was made by a different key");
  }
  const key = createPublicKey({
    key: Buffer.concat([ED25519_SPKI_PREFIX, publicKey.key]),
    format: "der",
    type: "spki",
  });
  const payload = signature.prehashed
    ? createHash("blake2b512").update(data).digest()
    : data;
  if (!verifyEd25519(null, payload, key, signature.signature)) {
    throw new Error("updater archive signature did not verify");
  }
  const trustedPayload = Buffer.concat([
    signature.signature,
    Buffer.from(signature.trustedComment, "utf8"),
  ]);
  if (!verifyEd25519(null, trustedPayload, key, signature.globalSignature)) {
    throw new Error("updater signature trusted comment did not verify");
  }
}

export function verifyUpdaterSignature(configPath, archivePath, signaturePath) {
  const config = JSON.parse(readFileSync(configPath, "utf8"));
  const encodedPublicKey = config.plugins?.updater?.pubkey;
  verifyUpdaterSignatureBytes(
    readFileSync(archivePath),
    readFileSync(signaturePath, "utf8"),
    encodedPublicKey,
  );
  console.log(`Verified updater signature with committed public key: ${basename(archivePath)}`);
}

function runCli() {
  if (process.argv.length !== 5) {
    throw new Error("Usage: verify-updater-signature.mjs TAURI_CONFIG ARCHIVE SIGNATURE");
  }
  verifyUpdaterSignature(process.argv[2], process.argv[3], process.argv[4]);
}

if (process.argv[1] && import.meta.url === new URL(`file://${process.argv[1]}`).href) {
  try {
    runCli();
  } catch (error) {
    console.error(`verify updater signature: ${error.message}`);
    process.exitCode = 1;
  }
}
