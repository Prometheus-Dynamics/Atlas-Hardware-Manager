// Errors no page caught: into atlas.log with the route they happened on,
// and a message the error page (routes/+error.svelte) can show.
import { errorText, isTauri, reportUiError } from "#lib/api/client.ts";
import { startCrumbs } from "#lib/api/trace.ts";

export const handleError = ({ error, event }: { error: unknown; event: { url?: URL } }) => {
  const at = event.url?.pathname ?? "unknown page";
  void reportUiError(error, at);
  return { message: errorText(error) };
};

if (typeof window !== "undefined") {
  if (isTauri) startCrumbs();
  window.addEventListener("unhandledrejection", (event) => {
    void reportUiError(event.reason, `${location.pathname} (a promise nobody awaited)`);
  });
  window.addEventListener("error", (event) => {
    // The browser's note that a resize settled over two frames: not an error.
    if (String(event.message).startsWith("ResizeObserver loop")) return;
    void reportUiError(event.error ?? event.message, `${location.pathname} (an event handler)`);
  });
}
