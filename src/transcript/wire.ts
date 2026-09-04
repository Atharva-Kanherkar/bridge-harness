/**
 * The brand that keeps raw wire kinds out of the UI.
 *
 * `ReplaySessionEvent.kind` is a bare `string` in the generated contract, so
 * every module in the app used to be free to compare it against a literal and
 * grow one more per-harness branch. Branding it makes that a type error: a
 * `WireKind` and a string literal have no overlap, so `event.kind === "…"`,
 * `switch (event.kind)` and `Record<WireKind, …>` all stop compiling.
 *
 * There are exactly two doors:
 *
 * - `asWireKind` mints one. Used by `src/api.ts` at the decode boundary, by the
 *   mock layer, and by test fixtures. Nothing else should call it.
 * - `readWireKind` opens one. Used by the transcript codec, which is the module
 *   whose whole job is to read wire kinds, and by the handful of non-transcript
 *   surfaces that legitimately watch the raw stream (the worker activity feed,
 *   the usage snapshot). Every call site is named in
 *   `src/transcript/harnessBranchGate.test.ts`.
 *
 * This module deliberately imports nothing, so `src/types.ts` can depend on it
 * without a cycle.
 */

declare const WIRE_KIND: unique symbol;

/**
 * A provider event kind as it came off the wire.
 *
 * Deliberately **not** a subtype of `string`. A branded string
 * (`string & {…}`) would still compare equal to a literal — TypeScript's
 * comparability rule lets `event.kind === "tool.started"` through — which is
 * exactly the line this type exists to stop. Declared as an opaque handle
 * instead, so comparison, `switch`, `startsWith` and template interpolation
 * are all type errors until someone says, in one word, that they are reading
 * the wire. The runtime value is still the plain string it always was.
 */
export type WireKind = { readonly [WIRE_KIND]: "bridge.wire" };

/** Mint a wire kind. Decode boundary, mocks and fixtures only. */
export function asWireKind(kind: string): WireKind {
  return kind as unknown as WireKind;
}

/** Read a wire kind as a string. The one documented escape hatch. */
export function readWireKind(kind: WireKind): string {
  return kind as unknown as string;
}
