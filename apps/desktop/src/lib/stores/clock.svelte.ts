// A shared ticking clock for relative times ("5s ago").

class Clock {
  now = $state(Date.now());

  constructor() {
    if (typeof window !== "undefined") setInterval(() => (this.now = Date.now()), 1000);
  }
}

export const clock = new Clock();
