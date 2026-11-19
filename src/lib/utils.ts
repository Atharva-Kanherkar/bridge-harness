import { type ClassValue, clsx } from "clsx";
import { extendTailwindMerge } from "tailwind-merge";

/** Bridge's type scale (src/index.css). Registered so `cn` treats these as
 * font sizes, not colours, and never drops one next to `text-foreground`. */
export const TYPE_SCALE = ["micro", "caption", "meta", "ui", "body", "message", "heading", "title"] as const;

const twMerge = extendTailwindMerge({ extend: { theme: { text: [...TYPE_SCALE] } } });

export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}
