// Transient notifications. Errors stay longer than confirmations.

export type ToastTone = "success" | "error" | "info" | "warning";

export interface Toast {
  id: number;
  tone: ToastTone;
  message: string;
}

let nextId = 1;

class ToastStore {
  items = $state<Toast[]>([]);

  push(tone: ToastTone, message: string, ms = tone === "error" ? 8000 : 3500) {
    const id = nextId++;
    this.items.push({ id, tone, message });
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
