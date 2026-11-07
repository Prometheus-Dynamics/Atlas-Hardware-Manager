// A shared ticking clock for relative times ("5s ago").

class Clock {
  now = $state(Date.now());

  constructor() {
    if (typeof window === "undefined") return;
    // On the whole second, with the live store's redraw (live.svelte.ts), so
    // both land in one frame. Nothing shows while hidden: catch up on the way
    // back instead.
    const tick = () => {
      if (!document.hidden) this.now = Date.now();
      setTimeout(tick, 1000 - (Date.now() % 1000));
    };
    setTimeout(tick, 1000 - (Date.now() % 1000));
    document.addEventListener("visibilitychange", () => document.hidden || (this.now = Date.now()));
  }
}

export const clock = new Clock();
