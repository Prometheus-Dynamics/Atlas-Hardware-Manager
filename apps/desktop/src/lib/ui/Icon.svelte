<script lang="ts" module>
  // The mockups' icons, path for path (docs/mockups). 24 px grid, drawn as strokes.
  export const ICONS = {
    chip: "M7 7h10v10H7z M10 4v3 M14 4v3 M10 17v3 M14 17v3 M4 10h3 M4 14h3 M17 10h3 M17 14h3",
    robot: "M5 8h14v11H5z M12 4v4 M9 13h.01 M15 13h.01 M9 16h6",
    bolt: "M13 3L5 14h6l-1 7 8-11h-6z",
    gear: "M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z M12 3v3 M12 18v3 M3 12h3 M18 12h3 M5.6 5.6l2 2 M16.4 16.4l2 2 M5.6 18.4l2-2 M16.4 7.6l2-2",
    locate: "M12 3v3 M12 18v3 M3 12h3 M18 12h3 M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8z",
    power: "M12 3v8 M7.5 6.2a7 7 0 1 0 9 0",
    plus: "M12 5v14 M5 12h14",
    pin: "M9 3h6l-1 6 3 3v1H7v-1l3-3z M12 13v8",
    download: "M12 4v11 M8 11l4 4 4-4 M5 20h14",
    folder: "M3 6h6l2 2h10v11H3z",
    file: "M6 3h8l4 4v14H6z M14 3v4h4",
    log: "M6 3h9l3 3v15H6z M9 11h6 M9 15h6",
    retry: "M19 11a7 7 0 1 0-2 5 M19 5v6h-6",
    stop: "M7 7h10v10H7z",
    bundle: "M4 4h16v16H4z M8 9h8 M8 13h8 M8 17h5",
    plug: "M12 3v12 M9 6l3-3 3 3 M7 12h10 M6 16h3v3 M15 16h3v3",
  } as const;
  export type IconName = keyof typeof ICONS;
</script>

<script lang="ts">
  let { name, sm = true }: { name: IconName; sm?: boolean } = $props();
</script>

<svg class={sm ? "i sm" : "i"} viewBox="0 0 24 24" aria-hidden="true"><path d={ICONS[name]} /></svg>
