#!/usr/bin/env node
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { inflateSync } from "node:zlib";

const pngSignature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);

// Tauri emits 8-bit RGBA PNGs. Decode their scanline filters without adding an
// image library to production dependencies.
export function decodePng(bytes) {
  if (!bytes.subarray(0, 8).equals(pngSignature)) throw new Error("Invalid PNG signature");
  let width, height, channels;
  const compressed = [];
  for (let offset = 8; offset < bytes.length;) {
    const size = bytes.readUInt32BE(offset);
    if (offset + 12 + size > bytes.length) throw new Error("Truncated PNG chunk");
    const type = bytes.toString("ascii", offset + 4, offset + 8);
    const chunk = bytes.subarray(offset + 8, offset + 8 + size);
    if (type === "IHDR") {
      width = chunk.readUInt32BE(0);
      height = chunk.readUInt32BE(4);
      if (chunk[8] !== 8 || ![2, 6].includes(chunk[9]) || chunk[12] !== 0)
        throw new Error("Expected a non-interlaced 8-bit RGB/RGBA PNG");
      channels = chunk[9] === 6 ? 4 : 3;
    } else if (type === "IDAT") compressed.push(chunk);
    offset += size + 12;
  }
  if (!width || !height || !channels || width > 4096 || height > 4096)
    throw new Error("Invalid icon dimensions");
  const stride = width * channels;
  const raw = inflateSync(Buffer.concat(compressed), { maxOutputLength: (stride + 1) * height });
  if (raw.length !== (stride + 1) * height) throw new Error("Invalid PNG scanline length");
  const pixels = Buffer.alloc(stride * height);
  const paeth = (a, b, c) => {
    const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c);
    return pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
  };
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)];
    if (filter > 4) throw new Error("Invalid PNG filter");
    for (let x = 0; x < stride; x++) {
      const index = y * stride + x;
      const left = x >= channels ? pixels[index - channels] : 0;
      const up = y ? pixels[index - stride] : 0;
      const corner = y && x >= channels ? pixels[index - stride - channels] : 0;
      const predictor = [0, left, up, Math.floor((left + up) / 2), paeth(left, up, corner)][filter];
      pixels[index] = (raw[y * (stride + 1) + 1 + x] + predictor) & 255;
    }
  }
  return { width, height, channels, pixels };
}

export function verifyVisibleMark(bytes, label) {
  const { width, height, channels, pixels } = decodePng(bytes);
  let bright = 0, dark = 0;
  for (let i = 0; i < pixels.length; i += channels) {
    if (channels === 4 && pixels[i + 3] < 128) continue;
    const [r, g, b] = pixels.subarray(i, i + 3);
    // The lime mark on the dark tile must survive export at every scale.
    if (r > 100 && g > 150 && b < g - 20) bright++;
    if (Math.max(r, g, b) < 80) dark++;
  }
  const count = width * height;
  if (bright / count < 0.08 || dark / count < 0.2)
    throw new Error(`${label}: Bridge's visible lime mark and dark background are missing (bright=${bright}, dark=${dark})`);
  return { width, height };
}

export function verifyIcns(bytes, label = "icon.icns") {
  if (bytes.toString("ascii", 0, 4) !== "icns" || bytes.readUInt32BE(4) !== bytes.length)
    throw new Error(`${label}: invalid ICNS container`);
  const sizes = new Set();
  for (let offset = 8; offset < bytes.length;) {
    const size = bytes.readUInt32BE(offset + 4);
    if (size < 8 || offset + size > bytes.length) throw new Error(`${label}: truncated ICNS entry`);
    const type = bytes.toString("ascii", offset, offset + 4);
    const payload = bytes.subarray(offset + 8, offset + size);
    if (payload.subarray(0, 8).equals(pngSignature)) {
      const { width, height } = verifyVisibleMark(payload, `${label}:${type}`);
      if (width !== height) throw new Error(`${label}:${type}: expected a square icon`);
      sizes.add(width);
    }
    offset += size;
  }
  for (const size of [32, 64, 128, 256, 512, 1024])
    if (!sizes.has(size)) throw new Error(`${label}: missing ${size}px icon`);
}

export function canonicalizeIcns(bytes) {
  verifyIcns(bytes);
  const entries = [];
  for (let offset = 8; offset < bytes.length;) {
    const size = bytes.readUInt32BE(offset + 4);
    entries.push(bytes.subarray(offset, offset + size));
    offset += size;
  }
  // Tauri's ICNS encoder iterates an unordered map. Entry ordering has no
  // semantic meaning, but sorting it prevents a dirty icon on every build.
  entries.sort((left, right) => Buffer.compare(left.subarray(0, 4), right.subarray(0, 4)));
  return Buffer.concat([bytes.subarray(0, 8), ...entries]);
}

export function verifyIconDirectory(directory) {
  for (const [name, size] of [["32x32.png", 32], ["64x64.png", 64], ["128x128.png", 128], ["128x128@2x.png", 256], ["icon.png", 512]]) {
    const path = resolve(directory, name);
    const actual = verifyVisibleMark(readFileSync(path), name);
    if (actual.width !== size || actual.height !== size) throw new Error(`${name}: wrong dimensions`);
  }
  verifyIcns(readFileSync(resolve(directory, "icon.icns")));
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const target = resolve(process.argv[2] ?? "src-tauri/icons");
  if (target.endsWith(".icns")) verifyIcns(readFileSync(target), target);
  else verifyIconDirectory(target);
  console.log("Verified Bridge icon: visible mark in PNGs and macOS ICNS representations.");
}
