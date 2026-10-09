// Whether CPU tiles show each core instead of the trend; one choice for
// every device, remembered across launches.

const KEY = "atlas.metrics.cpu-view";

function saved(): boolean {
  try {
    return localStorage.getItem(KEY) === "cores";
  } catch {
    return false;
  }
}

class CoreView {
  perCore = $state(saved());

  toggle() {
    this.perCore = !this.perCore;
    try {
      localStorage.setItem(KEY, this.perCore ? "cores" : "trend");
    } catch {
      // Private mode or blocked storage: the choice lasts this session.
    }
  }
}

export const coreView = new CoreView();
