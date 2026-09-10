/*
 * Achromatic hero backdrop: a sparse starfield plus a drifting field of hex-dump glyphs
 * on the right, masked so it fades into the page. Deterministic text keeps server and
 * client markup identical.
 */

const glyphs = "0123456789ABCDEF ....  ,,;:    ";

function line(seed: number, length: number) {
  let out = "";
  let x = seed;
  for (let i = 0; i < length; i += 1) {
    x = (x * 1103515245 + 12345) % 2147483648;
    out += glyphs[x % glyphs.length];
  }
  return out;
}

const rows = Array.from({ length: 34 }, (_, i) => line(i * 7919 + 17, 64));

export default function HeroBackdrop() {
  return (
    <div aria-hidden="true" className="pointer-events-none absolute inset-0 overflow-hidden">
      <div className="absolute inset-0 bg-stars" />
      <pre className="absolute -right-6 top-6 hidden select-none font-mono text-[11px] leading-[1.35] tracking-[0.18em] text-foreground/[0.22] [mask-image:radial-gradient(60%_70%_at_70%_35%,black,transparent)] lg:block">
        {rows.join("\n")}
      </pre>
      <pre className="absolute -left-10 top-40 hidden select-none font-mono text-[10px] leading-[1.4] tracking-[0.2em] text-foreground/[0.12] [mask-image:radial-gradient(50%_60%_at_30%_40%,black,transparent)] xl:block">
        {rows.slice(8, 26).join("\n")}
      </pre>
      <div className="absolute inset-x-0 bottom-0 h-40 bg-linear-to-t from-background to-transparent" />
    </div>
  );
}
