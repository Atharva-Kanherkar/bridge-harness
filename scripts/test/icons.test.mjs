import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { deflateSync } from "node:zlib";
import { canonicalizeIcns, verifyVisibleMark, verifyIcns, verifyIconDirectory } from "../verify-icons.mjs";

const root = fileURLToPath(new URL("../../", import.meta.url));
function crc32(bytes) {
  let value = 0xffffffff;
  for (const byte of bytes) {
    value ^= byte;
    for (let bit = 0; bit < 8; bit++) value = (value >>> 1) ^ ((value & 1) ? 0xedb88320 : 0);
  }
  return (value ^ 0xffffffff) >>> 0;
}
function chunk(type, bytes) {
  const result = Buffer.alloc(bytes.length + 12);
  result.writeUInt32BE(bytes.length, 0);
  result.write(type, 4);
  bytes.copy(result, 8);
  result.writeUInt32BE(crc32(result.subarray(4, bytes.length + 8)), bytes.length + 8);
  return result;
}
function solidPng(size, color) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0); ihdr.writeUInt32BE(size, 4); ihdr[8] = 8; ihdr[9] = 6;
  const pixels = Buffer.alloc(size * (size * 4 + 1));
  for (let y = 0; y < size; y++)
    for (let x = 0; x < size; x++) pixels.set(typeof color === "function" ? color(x, y) : color, y * (size * 4 + 1) + 1 + x * 4);
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", ihdr), chunk("IDAT", deflateSync(pixels)), chunk("IEND", Buffer.alloc(0))]);
}

test("committed icons contain the Bridge mark at every macOS scale", () => {
  verifyIconDirectory(resolve(root, "src-tauri/icons"));
});

test("black tile regression and invisible transparent mark fail the release gate", () => {
  assert.throws(() => verifyVisibleMark(solidPng(32, [17, 20, 17, 255]), "black"), /visible Span mark/);
  assert.throws(() => verifyVisibleMark(solidPng(32, [68, 227, 164, 0]), "transparent"), /visible Span mark/);
  assert.throws(() => verifyVisibleMark(solidPng(32, [68, 227, 164, 255]), "solid"), /dark background/);
});

test("the Span gate rejects exports missing the deck or either support", () => {
  const fixture = (missing) => solidPng(32, (x, y) => {
    if (missing !== "deck" && x >= 6 && x < 26 && y >= 9 && y < 13) return [68, 227, 164, 255];
    if (y >= 13 && y < 23 && (
      (missing !== "left" && x >= 8 && x < 11) ||
      (missing !== "right" && x >= 21 && x < 24)
    )) return [242, 241, 236, 255];
    return [17, 20, 17, 255];
  });
  assert.doesNotThrow(() => verifyVisibleMark(fixture(), "Span"));
  for (const missing of ["deck", "left", "right"])
    assert.throws(() => verifyVisibleMark(fixture(missing), missing), /visible Span mark/);
});

test("the macOS ICNS gate checks embedded pixels, not just file existence", () => {
  const original = readFileSync(resolve(root, "src-tauri/icons/icon.icns"));
  const parts = [];
  let replaced = false;
  for (let offset = 8; offset < original.length;) {
    const size = original.readUInt32BE(offset + 4);
    let part = original.subarray(offset, offset + size);
    if (!replaced && original.toString("ascii", offset, offset + 4) === "ic11") {
      const png = solidPng(32, [0, 0, 0, 255]);
      part = Buffer.alloc(8 + png.length);
      part.write("ic11", 0); part.writeUInt32BE(part.length, 4); png.copy(part, 8);
      replaced = true;
    }
    parts.push(part);
    offset += size;
  }
  assert.ok(replaced);
  const corrupt = Buffer.concat([Buffer.alloc(8), ...parts]);
  corrupt.write("icns", 0); corrupt.writeUInt32BE(corrupt.length, 4);
  assert.throws(() => verifyIcns(corrupt), /visible Span mark/);
});

test("ICNS output is canonical even when Tauri changes entry order", () => {
  const original = readFileSync(resolve(root, "src-tauri/icons/icon.icns"));
  const entries = [];
  for (let offset = 8; offset < original.length;) {
    const size = original.readUInt32BE(offset + 4);
    entries.push(original.subarray(offset, offset + size));
    offset += size;
  }
  const reversed = Buffer.concat([original.subarray(0, 8), ...entries.reverse()]);
  assert.deepEqual(canonicalizeIcns(original), canonicalizeIcns(reversed));
  assert.deepEqual(canonicalizeIcns(original), canonicalizeIcns(canonicalizeIcns(original)));
});
