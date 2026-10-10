// The interface's size. Atlas is laid out for a laptop window; on a big
// screen, full screen, the same layout is small and leaves half the window
// empty. Auto scales the whole interface (the webview's zoom, as Ctrl +
// does in a browser) with the window, so a 2560x1440 window looks like the
// laptop layout, only larger. A set size overrides it. Kept per computer.
import { isTauri } from "#lib/api/client.ts";

const KEY = "atlas.ui.scale";
/** The window size (CSS px at 100%) the layout is made for. */
const BASE = { width: 1700, height: 1000 };
const MAX_AUTO = 1.6;

export type ScaleChoice = "auto" | "1" | "1.1" | "1.25" | "1.5";

function remembered(): ScaleChoice {
  try {
    return (localStorage.getItem(KEY) as ScaleChoice | null) ?? "auto";
  } catch {
    return "auto";
  }
}

/** Auto's factor for a window this size (CSS px at 100%). */
export function autoScale(width: number, height: number): number {
  const fit = Math.min(width / BASE.width, height / BASE.height);
  return Math.min(MAX_AUTO, Math.max(1, Math.floor(fit * 20) / 20));
}

class Scale {
  choice = $state<ScaleChoice>(remembered());
  /** The factor in effect. */
  factor = $state(1);

  /** Applies the choice and follows the window's size; returns the stop. */
  start(): () => void {
    const update = () => void this.apply();
    window.addEventListener("resize", update);
    update();
    return () => window.removeEventListener("resize", update);
  }

  set(choice: ScaleChoice) {
    this.choice = choice;
    try {
      localStorage.setItem(KEY, choice);
    } catch {
      // Not kept; it still applies now.
    }
    void this.apply();
  }

  private async apply() {
    // The webview's zoom shrinks the CSS viewport, so the window's own size
    // is the viewport times the zoom in effect; CSS zoom (the browser
    // preview) leaves the viewport as it is.
    const zoomed = isTauri ? this.factor : 1;
    const next =
      this.choice === "auto" ? autoScale(window.innerWidth * zoomed, window.innerHeight * zoomed) : Number(this.choice);
    if (Math.abs(next - this.factor) < 0.001) return;
    this.factor = next;
    if (isTauri) {
      const { getCurrentWebview } = await import("@tauri-apps/api/webview");
      await getCurrentWebview().setZoom(next);
    } else {
      document.documentElement.style.setProperty("zoom", String(next));
    }
  }
}

export const scale = new Scale();
