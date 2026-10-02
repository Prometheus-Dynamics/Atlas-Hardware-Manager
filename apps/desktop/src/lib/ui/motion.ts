// The motion system. Every animation in Atlas uses these, so things move
// the same way everywhere:
//
// - Appearing: `rise` (fade and lift), staggered with `stagger(i)`.
// - Reordering: `flip` with `DUR.med`.
// - Swapping content (tabs, verdicts): `softFade`.
// - Live values snap to each reading: no tweens, no scrolling, so a slow
//   machine never spends frames on numbers.
// - Sheets and popovers: `slideIn`, `popover`.
//
// Durations and easing match the CSS tokens in themes/glass.css.

import { quintOut } from "svelte/easing";
import { fade, fly, scale, type TransitionConfig } from "svelte/transition";

export const reducedMotion =
  typeof window !== "undefined" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;

/** Mirrors --t-* in glass.css. */
export const DUR: { fast: number; med: number; enter: number; data: number } = { fast: 90, med: 140, enter: 180, data: 0 };

/** The one easing curve; matches --ease-out. */
export const ease = quintOut;

/** A duration, or nearly nothing when motion is reduced. */
export const ms = (duration: number) => (reducedMotion ? 0 : duration);

/** Delay for the i-th item of a list appearing; capped so long lists don't drag. */
export const stagger = (i: number) => (reducedMotion ? 0 : Math.min(i, 6) * 20);

/** Fade and rise a few pixels: anything appearing. */
export function rise(node: Element, { y = 4, duration = DUR.enter, delay = 0 } = {}): TransitionConfig {
  if (reducedMotion) return fade(node, { duration: 90 });
  return fly(node, { y, duration, delay, easing: ease });
}

/** Slide in from the right: sheets and side panels. */
export function slideIn(node: Element, { x = 16, duration = DUR.enter }: { x?: number; duration?: number } = {}): TransitionConfig {
  if (reducedMotion) return fade(node, { duration: 90 });
  return fly(node, { x, duration, easing: ease, opacity: 0 });
}

/** A quick pop: check marks and badges. */
export function pop(node: Element, { duration = DUR.med, delay = 0 } = {}): TransitionConfig {
  if (reducedMotion) return fade(node, { duration: 90 });
  return scale(node, { start: 0.5, duration, delay, easing: ease });
}

/** Content swapping in place: tabs, verdicts. */
export function softFade(node: Element, { duration = DUR.med, delay = 0 } = {}): TransitionConfig {
  return fade(node, { duration: reducedMotion ? 80 : duration, delay });
}

/** Popovers: fade and grow slightly from their anchor. */
export function popover(node: Element, { duration = DUR.med } = {}): TransitionConfig {
  if (reducedMotion) return fade(node, { duration: 80 });
  return scale(node, { start: 0.96, duration, easing: ease, opacity: 0 });
}
