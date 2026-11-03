// Errors no page caught: into atlas.log with the route they happened on,
// and a message the error page (routes/+error.svelte) can show.
import { errorText, reportUiError } from "#lib/api/client.ts";

export const handleError = ({ error, event }: { error: unknown; event: { url?: URL } }) => {
  const at = event.url?.pathname ?? "unknown page";
  void reportUiError(error, at);
  return { message: errorText(error) };
};

if (typeof window !== "undefined") {
  window.addEventListener("unhandledrejection", (event) => {
    void reportUiError(event.reason, `${location.pathname} (a promise nobody awaited)`);
  });
  window.addEventListener("error", (event) => {
    void reportUiError(event.error ?? event.message, `${location.pathname} (an event handler)`);
  });
}
