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

  /** Each apply's turn; a later one makes an earlier, still running one stale. */
  private turn = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;

  /** Applies the choice and follows the window's size; returns the stop. */
  start(): () => void {
    // Once the size settles: going full screen sends a burst of resizes, and
    // the zoom itself sends more.
    const update = () => {
      if (this.timer) clearTimeout(this.timer);
      this.timer = setTimeout(() => void this.apply(), 120);
    };
    window.addEventListener("resize", update);
    void this.apply();
    return () => {
      window.removeEventListener("resize", update);
      if (this.timer) clearTimeout(this.timer);
    };
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

  /** The window's size at 100% (CSS px), whatever the zoom. */
  private async windowSize(): Promise<{ width: number; height: number }> {
    if (isTauri) {
      // The window's own size, not the CSS viewport: the webview's zoom
      // shrinks the viewport, and reading it while a zoom is still being
      // applied gave a size from one zoom times the other, so full screen
      // flipped between two factors.
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const win = getCurrentWindow();
      const [size, ratio] = await Promise.all([win.innerSize(), win.scaleFactor()]);
      return { width: size.width / ratio, height: size.height / ratio };
    }
    // CSS zoom (the browser preview) leaves the viewport as it is.
    return { width: window.innerWidth, height: window.innerHeight };
  }

  private async apply() {
    const turn = ++this.turn;
    let next = Number(this.choice);
    if (this.choice === "auto") {
      const { width, height } = await this.windowSize();
      next = autoScale(width, height);
    }
    if (turn !== this.turn || Math.abs(next - this.factor) < 0.001) return;
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
