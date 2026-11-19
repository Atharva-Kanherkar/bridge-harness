import { createContext, useContext, useState } from "react";
import { cn } from "@/lib/utils";

/** Avatars GitHub itself returned for the accounts on this pull request, by
 *  login. Only these are ever loaded: guessing an address from a login would
 *  show a stranger's face for a bot that shares a name with a user. */
export const AvatarScope = createContext<ReadonlyMap<string, string>>(new Map());

/** An account's initial on a neutral tile: the fallback, and the whole avatar
 *  for anyone GitHub sent no picture for. */
export function Monogram({ login, className }: { login?: string | null; className?: string }) {
  const name = login ?? "ghost";
  return <span aria-hidden="true" className={cn("grid size-5 shrink-0 place-items-center rounded-full bg-muted text-caption font-semibold text-muted-foreground ring-1 ring-border", className)}>{name.replace(/[^a-z0-9]/gi, "").charAt(0) || "?"}</span>;
}

export function Avatar({ login, className }: { login?: string | null; className?: string }) {
  const url = useContext(AvatarScope).get(login ?? "");
  const [broken, setBroken] = useState(false);
  if (!url || broken || !url.startsWith("https://")) return <Monogram login={login} className={className} />;
  return <img
    src={url}
    alt=""
    aria-hidden="true"
    loading="lazy"
    decoding="async"
    referrerPolicy="no-referrer"
    onError={() => setBroken(true)}
    className={cn("size-5 shrink-0 rounded-full bg-muted object-cover ring-1 ring-border", className)}
  />;
}
