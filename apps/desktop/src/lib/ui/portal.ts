// Moves an element to <body> while it exists, so no ancestor's transform,
// clipping (a scroll region) or stacking context can hide or misplace it:
// for hover tips and anything else that floats over the page.

export function portal(node: HTMLElement) {
  document.body.appendChild(node);
  return {
    destroy() {
      node.remove();
    },
  };
}
