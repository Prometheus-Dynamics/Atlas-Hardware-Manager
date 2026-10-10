// Masonry in a CSS grid: put `use:masonry` on each item of a grid with
// `grid-auto-rows: 4px; row-gap: 0` and the item spans as many rows as its
// height (plus the gap) needs, so items of different heights pack under
// each other, in order, across the columns. Follows the item's size.
const ROW = 4;

export function masonry(node: HTMLElement, gap = 10) {
  let current = gap;
  const fit = () => {
    // offsetHeight: layout px, unaffected by zoom.
    node.style.gridRowEnd = `span ${Math.max(1, Math.ceil((node.offsetHeight + current) / ROW))}`;
  };
  node.style.alignSelf = "start";
  const observer = new ResizeObserver(fit);
  observer.observe(node);
  fit();
  return {
    update(next: number) {
      current = next;
      fit();
    },
    destroy() {
      observer.disconnect();
    },
  };
}
