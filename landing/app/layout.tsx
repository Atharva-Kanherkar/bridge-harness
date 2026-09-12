import type { Metadata } from "next";
import { Doto, Geist_Mono, Inter } from "next/font/google";
import "./globals.css";

const inter = Inter({
  variable: "--font-inter",
  subsets: ["latin"],
  axes: ["opsz"],
});

const doto = Doto({
  variable: "--font-doto",
  subsets: ["latin"],
  weight: "900",
});

const geistMono = Geist_Mono({
  variable: "--font-geist-mono",
  subsets: ["latin"],
});

export const metadata: Metadata = {
  title: {
    default: "Bridge · Delegate the coding. Keep the judgment.",
    template: "%s · Bridge",
  },
  description:
    "Bridge is a native desktop control room for supervised coding-agent work. It runs Codex, Claude Code, and OpenCode as one team, with a policy engine on every gate and a Git worktree for every worker.",
  applicationName: "Bridge",
};

export default function RootLayout({ children }: LayoutProps<"/">) {
  return (
    <html lang="en" className={`${inter.variable} ${doto.variable} ${geistMono.variable} h-full antialiased`}>
      <body className="min-h-full flex flex-col">{children}</body>
    </html>
  );
}
