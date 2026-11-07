// A shared ticking clock for relative times ("5s ago").

class Clock {
  now = $state(Date.now());

  constructor() {
    if (typeof window === "undefined") return;
    // Nothing shows while hidden: catch up on the way back instead.
    setInterval(() => document.hidden || (this.now = Date.now()), 1000);
    document.addEventListener("visibilitychange", () => document.hidden || (this.now = Date.now()));
  }
}

export const clock = new Clock();
