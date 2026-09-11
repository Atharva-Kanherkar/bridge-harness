"use client";

import { useEffect, useRef } from "react";

/*
 * A dot lattice that lights up around the pointer: one dim layer everywhere, one bright
 * layer revealed through a radial mask that follows the cursor. Both layers share the same
 * 22px grid, so the lit dots sit exactly on the dim ones rather than beside them.
 *
 * The pointer position rides CSS custom properties written on one rAF, so moving the mouse
 * never re-renders React. Reduced motion leaves the lattice unlit.
 */
export default function HeroBackdrop() {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const host = ref.current;
    if (!host || window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;

    let frame = 0;
    let pending: { x: number; y: number } | null = null;

    const flush = () => {
      frame = 0;
      if (!pending) return;
      const box = host.getBoundingClientRect();
      host.style.setProperty("--mx", `${pending.x - box.left}px`);
      host.style.setProperty("--my", `${pending.y - box.top}px`);
      host.style.setProperty("--glow", pending.y - box.top > box.height ? "0" : "1");
      pending = null;
    };

    const onMove = (event: PointerEvent) => {
      pending = { x: event.clientX, y: event.clientY };
      if (!frame) frame = requestAnimationFrame(flush);
    };
    const onLeave = () => host.style.setProperty("--glow", "0");

    window.addEventListener("pointermove", onMove, { passive: true });
    document.addEventListener("pointerleave", onLeave);
    return () => {
      window.removeEventListener("pointermove", onMove);
      document.removeEventListener("pointerleave", onLeave);
      if (frame) cancelAnimationFrame(frame);
    };
  }, []);

  return (
    <div
      ref={ref}
      aria-hidden="true"
      className="pointer-events-none absolute inset-0 overflow-hidden [--glow:0] [--mx:50%] [--my:34%]"
    >
      <div className="absolute inset-0 bg-dots [mask-image:radial-gradient(85%_75%_at_50%_35%,black_45%,transparent_85%)]" />
      <div
        className="absolute inset-0 bg-dots-lit opacity-[var(--glow)] transition-opacity duration-700 [mask-image:radial-gradient(240px_240px_at_var(--mx)_var(--my),black,transparent_70%)]"
      />
      <div className="absolute inset-x-0 bottom-0 h-32 bg-linear-to-t from-background to-transparent" />
    </div>
  );
}
