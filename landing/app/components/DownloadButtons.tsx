"use client";

import { useSyncExternalStore } from "react";
import ActionButton from "./ActionButton";
import { downloadPath, linuxDownloadPath, macDownloadPath } from "../content/site";

type Platform = "macos" | "linux" | "other";

/*
 * Both stable builds ship: the macOS DMG straight from the latest release, and the Linux
 * AppImage through the /download/linux redirect that resolves the current asset. The
 * primary button is the visitor's own platform; everyone always gets the same second link
 * to /download, where every package, checksum, and requirement is listed.
 */
const primaryFor: Record<Platform, { href: string; label: string }> = {
  macos: { href: macDownloadPath, label: "Download for macOS" },
  linux: { href: linuxDownloadPath, label: "Download for Linux" },
  other: { href: downloadPath, label: "Download Bridge" },
};

function detectPlatform(): Platform {
  const nav = navigator as Navigator & { userAgentData?: { platform?: string } };
  const hint = (nav.userAgentData?.platform ?? navigator.platform ?? "").toLowerCase();
  const agent = navigator.userAgent.toLowerCase();
  if (hint.includes("mac") || agent.includes("macintosh")) return "macos";
  // Android also reports Linux; only desktop Linux gets the AppImage.
  if ((hint.includes("linux") || agent.includes("linux")) && !agent.includes("android")) return "linux";
  return "other";
}

const noop = () => () => {};

export default function DownloadButtons() {
  // Server markup advertises macOS, the platform the signed build targets first; the client
  // reads its own platform once on hydration, so the two never disagree mid-render.
  const platform = useSyncExternalStore(noop, detectPlatform, () => "macos" as Platform);
  const primary = primaryFor[platform];

  return (
    <div className="flex flex-col items-center justify-center gap-4 sm:flex-row">
      <ActionButton href={primary.href} label={primary.label} variant="primary" external={platform !== "other"} />
      {platform !== "other" && <ActionButton href={downloadPath} label="All platforms" />}
    </div>
  );
}
