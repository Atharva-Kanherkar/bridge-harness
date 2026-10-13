"use client";

import { useEffect, useRef } from "react";
import { OMARCHY_CHANGE_EVENT } from "../../lib/omarchySkin";

/*
 * omarchy.org's backdrop, in Bridge's hero: a field of square pixels, dense at
 * the edges and thinning toward the copy, a few cells lit in the theme's
 * accent and a handful flickering. It replaces the round dot lattice only in
 * Omarchy mode, reads its colours from the live tokens, and repaints when the
 * theme changes. Reduced motion keeps the field and drops the flicker.
 */
const PITCH = 18;
const CELL = 9;

type Cell = { x: number; y: number; level: number; lit: boolean };

export default function OmarchyField() {
  const ref = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = ref.current;
    const context = canvas?.getContext("2d");
    if (!canvas || !context) return;
    const still = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    let cells: Cell[] = [];
    let ink = "#888";
    let accent = "#888";
    let timer = 0;

    const readColors = () => {
      const style = getComputedStyle(document.documentElement);
      ink = style.getPropertyValue("--foreground").trim() || ink;
      accent = style.getPropertyValue("--primary").trim() || accent;
    };

    const draw = (cell: Cell) => {
      context.clearRect(cell.x, cell.y, CELL, CELL);
      if (cell.level <= 0) return;
      context.globalAlpha = cell.lit ? 0.55 : cell.level;
      context.fillStyle = cell.lit ? accent : ink;
      context.fillRect(cell.x, cell.y, CELL, CELL);
    };

    const layout = () => {
      const box = canvas.getBoundingClientRect();
      const ratio = window.devicePixelRatio || 1;
      canvas.width = Math.round(box.width * ratio);
      canvas.height = Math.round(box.height * ratio);
      context.setTransform(ratio, 0, 0, ratio, 0, 0);
      context.clearRect(0, 0, box.width, box.height);
      cells = [];
      const cx = box.width / 2;
      const cy = box.height * 0.36;
      for (let y = 0; y < box.height; y += PITCH) {
        for (let x = 0; x < box.width; x += PITCH) {
          // Distance from the copy: 0 at its centre, 1 at the edge of an ellipse around it.
          // Nothing lands inside the clear zone, and density ramps up toward the edges.
          const d = Math.hypot((x - cx) / Math.max(cx * 0.92, 340), (y - cy) / (box.height * 0.5));
          const density = Math.min(1, Math.max(0, (d - 0.62) / 0.5)) ** 1.4 * 0.85;
          if (Math.random() > density) continue;
          const cell = { x, y, level: 0.06 + Math.random() * 0.16, lit: Math.random() < 0.025 };
          cells.push(cell);
          draw(cell);
        }
      }
    };

    const flicker = () => {
      if (!canvas.offsetWidth) return;
      for (let i = 0; i < 6 && cells.length; i++) {
        const cell = cells[Math.floor(Math.random() * cells.length)];
        cell.level = Math.random() < 0.3 ? 0 : 0.06 + Math.random() * 0.2;
        cell.lit = Math.random() < 0.04;
        draw(cell);
      }
    };

    const repaint = () => {
      readColors();
      cells.forEach(draw);
    };

    readColors();
    layout();
    if (!still) timer = window.setInterval(flicker, 140);
    const observer = new ResizeObserver(layout);
    observer.observe(canvas);
    window.addEventListener(OMARCHY_CHANGE_EVENT, repaint);
    return () => {
      window.clearInterval(timer);
      observer.disconnect();
      window.removeEventListener(OMARCHY_CHANGE_EVENT, repaint);
    };
  }, []);

  return (
    <canvas
      ref={ref}
      aria-hidden="true"
      className="pointer-events-none absolute inset-0 hidden size-full [mask-image:linear-gradient(to_bottom,black_70%,transparent)] omarchy:block"
    />
  );
}
