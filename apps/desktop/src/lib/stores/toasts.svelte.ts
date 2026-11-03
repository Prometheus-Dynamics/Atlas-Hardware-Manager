// Transient notifications. Errors stay longer than confirmations.

import { errorId } from "#lib/api/trace.ts";

export type ToastTone = "success" | "error" | "info" | "warning";

export interface Toast {
  id: number;
  tone: ToastTone;
  message: string;
  /** The atlas.log error id of the failure an error is about. */
  ref: string | null;
}

let nextId = 1;

class ToastStore {
  items = $state<Toast[]>([]);

  push(tone: ToastTone, message: string, ms = tone === "error" ? 8000 : 3500) {
    const id = nextId++;
    this.items.push({
      id,
      tone,
      message,
      ref: tone === "error" ? errorId(message) : null,
    });
    setTimeout(() => this.dismiss(id), ms);
  }

  dismiss(id: number) {
    this.items = this.items.filter((t) => t.id !== id);
  }

  success = (message: string) => this.push("success", message);
  error = (message: string) => this.push("error", message);
  info = (message: string) => this.push("info", message);
  warning = (message: string) => this.push("warning", message);
}

export const toasts = new ToastStore();
