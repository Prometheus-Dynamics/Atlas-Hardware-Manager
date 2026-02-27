# Helios Frontend Design Language

This document captures the visual and interaction vocabulary used across the Helios Svelte frontends. Use it as a reference when extending or reviewing UI so that new work feels native to the existing application.

## Foundations

- **Framework stack** — SvelteKit + Tailwind CSS v4 with the Skeleton UI plugin (`src/app.css`). Tailwind utility classes provide layout and spacing, while Skeleton presets handle component primitives such as buttons, modals, and toasts.
- **Theme binding** — The global layout pins `data-theme="helios"` on `<body>` (`src/routes/+layout.svelte`), activating the custom CSS variable palette defined in `src/themes/helios.css`.
- **Color tokens** — The `helios` theme supplies OKLCH-based scales for primary, secondary, tertiary, and semantic colors (`--color-*` vars). Surfaces sit on a charcoal gradient (`--color-surface-50`…`950`) that produces the app’s high-contrast dark UI. Use `*-contrast-light/dark` tokens when you need text or icon colors that adapt to surface brightness.
- **Spacing & radii** — The theme narrows the global spacing unit to `0.22rem`, keeping controls compact. Border radii (`--radius-base`, `--radius-container`) are small to reinforce the tool-like aesthetic.

## Typography

- **Typeface** — `Geologica` (and `Geologica Auto`) is embedded through `src/fonts.css` and set as the body font in `src/app.css`. Headings inherit the family and rely on weight for hierarchy.
- **Tone** — Titles and key metrics use bold weights (`font-semibold`/`font-bold`). Supporting labels often switch to uppercase with pronounced tracking (`tracking-[0.3em]`…`0.4em`) to emulate instrumentation readouts.
- **Scale** — Tailwind’s default scale is retained, but headings typically top out at `text-3xl`, tiles at `text-3xl`, and auxiliary metadata at `text-xs` or smaller (`text-[0.6rem]`).

## Layout Patterns

- **Workspace shell** — A sticky vertical navigation rail sits on the left, rendered as a tinted surface with thin borders (`border-surface-800`, `bg-surface-900/80`). The main content column (`<main>`) uses a lighter surface tint (`bg-surface-900/30`) to create depth (`src/routes/+layout.svelte`).
- **Section framing** — Content is grouped inside bordered panels (`Panel.svelte`), which apply subtle background alpha (`bg-surface-900/40`) and `space-y` separators.
- **Whitespace rhythm** — Panels use `p-4` padding, while grids and stacks rely on utility spacing (`gap-3`, `space-y-4`) to keep dense telemetry legible without feeling cluttered.

## Component Grammar

- **Page header** — `PageHeader.svelte` combines an uppercase eyebrow, bold H1, and optional subtitle plus action group. Actions align to the right in a flex container for balanced scaffolding.
- **Panels** — `Panel.svelte` standardizes bordered cards with optional eyebrow, title, subtitle, and actions snippet. Tone variants (`default`, `subtle`, `contrast`) select different surface transparencies.
- **Tiles & metrics** — `UsageTile.svelte` renders metric cards with uppercase labels, oversized values, and sparklines (`InlineSparkline.svelte`). Accent colors map to data type palettes so CPU/GPU/etc. stay consistent.
- **Segmented visuals** — Components such as `SegmentedBar.svelte` and `UsageTrendChart.svelte` combine accent backgrounds with uppercase microcopy to express distribution and trend states.
- **Media & flow** — Pipeline views, stream previews, and flow editors rely on neutral surfaces to foreground complex content (e.g., `PipelineDetailPanel.svelte`, `StreamPreview.svelte`). Buttons inside these areas use tonal presets to avoid overwhelming primary accents.

## Controls & Interactions

- **Buttons** — Skeleton’s `btn` class with size modifiers (`btn-3xs`…`btn-sm`) and presets (`preset-filled-primary-500`, `preset-outline`, `preset-tonal`) define call-to-action hierarchy. Uppercase, tracked labels emphasize decisiveness.
- **Forms** — Rely on Tailwind form plugin styling via Skeleton defaults. Input surfaces stay dark (`bg-surface-900`/`800`) with thin borders and contrast text.
- **Feedback** — Toasts are handled by Skeleton’s `<Toaster>` instance registered in the root layout. Panel states use subtle text color changes (`text-surface-500`, `text-primary-200`) and border tints to indicate success, warnings, or actionability.
- **Empty states** — Typically framed with dashed borders and muted copy (`border-dashed border-surface-700/60`, `text-surface-500`) to cue potential action without alarm.

## Color Usage Guidelines

| Token family | Primary usage | Notes |
| --- | --- | --- |
| `--color-primary-*` | Core actions, selection, active nav | Warm amber accent used sparingly for focus. |
| `--color-secondary-*` | Secondary metrics, graph overlays | Cool violet for contrast against primary. |
| `--color-tertiary-*` | Tertiary data channels | Magenta tones to expand visualization palette. |
| `--color-success/*` | Healthy status, uptime metrics | Muted green balanced for dark backgrounds. |
| `--color-warning/*` | Latency spikes, caution banners | High-lightness yellow maintains readability. |
| `--color-error/*` | Failures, alerts | Rich red with clear contrast tokens. |
| `--color-surface-*` | Background layers | Step down opacity (`/30`, `/40`, `/60`) for depth. |

## Working Guidelines

1. **Start from existing primitives** — Reach for `PageHeader`, `Panel`, `UsageTile`, and Skeleton buttons before inventing new shells.
2. **Respect accent scarcity** — Keep `primary` fills for the most important actions or live statuses; lean on tonal or outline variants elsewhere.
3. **Maintain typographic rhythm** — Use uppercase + tracking for navigational hints, filters, and labels; preserve sentence case for descriptive copy.
4. **Design for dense telemetry** — Prefer compact spacing and grid layouts that accommodate dashboards without overwhelming; escape hasty scroll by chunking into panels.
5. **Check contrast** — When customizing colors, pick from provided `contrast-light/dark` tokens to ensure text remains legible on tinted surfaces.

## File Reference Index

- Theme tokens: `src/themes/helios.css`
- Global layout & shell: `src/routes/+layout.svelte`
- Global styles: `src/app.css`, `src/fonts.css`
- Core primitives: `src/lib/components/PageHeader.svelte`, `src/lib/components/Panel.svelte`, `src/lib/components/UsageTile.svelte`, `src/lib/components/SegmentedBar.svelte`

Keep this document updated as new primitives are introduced so that Helios retains a cohesive, instrument-grade interface.
