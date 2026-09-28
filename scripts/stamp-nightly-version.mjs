#!/usr/bin/env node
// Stamp the build before release-dmg.sh so Cargo and Tauri embed the same version.
import { readFileSync, writeFileSync } from "node:fs";

const date = process.argv[2];
if (!/^20\d{2}-[01]\d-[0-3]\d$/.test(date ?? "")) throw new Error("Expected nightly date YYYY-MM-DD");
const configPath = "src-tauri/tauri.conf.json";
const config = JSON.parse(readFileSync(configPath, "utf8"));
const match = /^(\d+)\.(\d+)\.(\d+)$/.exec(config.version);
if (!match) throw new Error("Base version must be a stable semantic version");
const published = process.argv[3];
if (published && !/^\d+\.\d+\.\d+$/.test(published)) throw new Error("Published stable version must be semantic");
const base = [match[1], match[2], match[3]].map(Number);
const released = published?.split(".").map(Number) ?? base;
const newer = base.findIndex((part, index) => part !== released[index]);
const highest = newer < 0 || base[newer] > released[newer] ? base : released;
const version = `${highest[0]}.${highest[1]}.${highest[2] + 1}-nightly.${date.replaceAll("-", "")}`;
config.version = version;
writeFileSync(configPath, `${JSON.stringify(config, null, 2)}\n`);
const cargoPath = "src-tauri/Cargo.toml";
const cargo = readFileSync(cargoPath, "utf8");
writeFileSync(cargoPath, cargo.replace(/^(version = ")\d+\.\d+\.\d+("$)/m, `$1${version}$2`));
const packagePath = "package.json";
const packageJson = JSON.parse(readFileSync(packagePath, "utf8"));
packageJson.version = version;
writeFileSync(packagePath, `${JSON.stringify(packageJson, null, 2)}\n`);
console.log(version);
