// Shared motion: short, cubic-out, and calm when the OS asks for less motion.

import { cubicOut } from "svelte/easing";
import { fade, fly, scale, type TransitionConfig } from "svelte/transition";

export const reducedMotion =
  typeof window !== "undefined" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;

/** A duration, or nearly nothing when motion is reduced. */
export const ms = (duration: number) => (reducedMotion ? 0 : duration);

/** Fade and rise a few pixels: cards and rows appearing. */
export function rise(node: Element, { y = 6, duration = 180, delay = 0 } = {}): TransitionConfig {
  if (reducedMotion) return fade(node, { duration: 90 });
  return fly(node, { y, duration, delay, easing: cubicOut });
}

/** Slide in from the right: sheets and side panels. */
export function slideIn(node: Element, { x = 28, duration = 220 } = {}): TransitionConfig {
  if (reducedMotion) return fade(node, { duration: 90 });
  return fly(node, { x, duration, easing: cubicOut, opacity: 0 });
}

/** A quick pop: check marks and badges. */
export function pop(node: Element, { duration = 200, delay = 0 } = {}): TransitionConfig {
  if (reducedMotion) return fade(node, { duration: 90 });
  return scale(node, { start: 0.4, duration, delay, easing: cubicOut });
}

/** A plain fade, shortened when motion is reduced. */
export function softFade(node: Element, { duration = 140 } = {}): TransitionConfig {
  return fade(node, { duration: reducedMotion ? 80 : duration });
}

/** Popovers: fade and grow slightly from their anchor. */
export function popover(node: Element, { duration = 160 } = {}): TransitionConfig {
  if (reducedMotion) return fade(node, { duration: 80 });
  return scale(node, { start: 0.96, duration, easing: cubicOut, opacity: 0 });
}
