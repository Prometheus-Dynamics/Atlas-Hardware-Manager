# Atlas `next`: design language

This is the visual and interaction vocabulary for the rebuilt app. The screens in
[`docs/mockups/`](mockups/index.html) are the reference. Build from them, not from memory of
`dev`. The rules come from `docs/design.md` (rule 7: fast and plain) and from the user's
feedback on `dev`.

`dev` has a `docs/design-language.md`, but it describes the Helios web frontends (Svelte,
Skeleton, a warm charcoal theme). Atlas keeps its own palette, the "glass" tokens in
`apps/desktop/src/themes/glass.css` on `dev`, and drops what the user rejected: capsules,
floating glass cards, backdrop blur, entrance animation, and scrolling pages.

## The rules

1. **Sharp and grounded.** Corners are 2 px (controls, chips, rows) or 3 px (panels). Panels have
   a 1 px hairline and a short shadow. Nothing floats far off the page. Round shapes exist only for
   status dots, a switch knob, and the spinner.
2. **Dense, instrument-like.** Text is 12 px. Rows are 26 px, device rows 34 px, slots 44 to 56 px.
   Numbers are tabular. Labels above panels are small uppercase.
3. **No page scroll, ever.** Every screen is a full-height grid. Each panel fills its grid cell and
   its contents truncate with an ellipsis or are hidden. Nothing spills past its parent. This is
   checked at 1024x680, 1320x860 and 1830x1030 (see Verification).
4. **Data is on screen.** Readings are sparklines with a big number. Charts are plain SVG. The
   sidebar is a tree: robot, then SystemCore, then the boards behind it.
5. **One record per board.** A board reached by USB and by network is one row. Its links are shown
   as chips in the row ("USB 1-2", "10.42.1.14"), not as two rows.
6. **Show only what applies now.** A button for an action that cannot happen now is not shown. A
   pinned slot shows Unpin, not Remove. A running slot shows no Remove. The SystemCore's second
   offer says "after Orion" as a chip, not as a disabled button.
7. **No confirmation on anything reversible.** Reboot, Locate, Pin, Set default, Confirm and
   Retry happen at once. Irreversible actions (remove a slot, erase a disk) confirm **in place**:
   the row turns into a confirmation line of the same height, and the layout does not move.
8. **Pages appear at once.** No entrance animation, no fade or slide of large regions, no tweens on
   values. Live numbers update in place. Hover and press change the background only.
9. **Plain words.** Sentence case for text. Uppercase only for panel headers and sidebar sections.
   "Seen, not reached" and "Needs Orion" say what is true. No jargon in a label where a plain word
   works.
10. **Sample data only.** Board, robot and product names, versions, fingerprints and sizes in the
    mockups are placeholders. Code never reads them as names (see `AGENTS.md`).

## Tokens

All colours are custom properties on `:root` in [`mockups/atlas.css`](mockups/atlas.css). Dark is the
default under `prefers-color-scheme: dark`, redefined for `:root[data-theme="dark"]` too. Light is
the pair. No component hardcodes a colour.

| Token family | Use |
|---|---|
| `--bg`, `--rail`, `--glass`, `--glass-hover`, `--glass-strong` | page, sidebar, panel, hover, table group rows |
| `--hairline`, `--glass-border`, `--glass-border-strong` | rules, panel edges, control edges |
| `--fg`, `--fg-muted`, `--fg-faint`, `--fg-disabled` | text in four steps |
| `--accent` (and `-text`, `-tint`) | primary action, live work (writing, progress), "needs you" (boot ROM waiting, offer) |
| `--ok`, `--warn`, `--err`, `--info`, `--offline` (each with `-bg` and `-fg`) | status chips and dots |
| `--series-1` to `--series-7` | chart lines, in this order |
| `--font-ui`, `--font-code` | IBM Plex Sans and Plex Mono, with system fallbacks. No font is loaded from the network |

Accent is scarce. Use it for the one primary action in an area, for progress that is running, and
for a thing that waits on the person. Pinned, fallback and removable are neutral.

## Type

| Use | Size and weight |
|---|---|
| Body, table cells | 11.5 to 12 px |
| Panel header | 10.5 px, uppercase, letter-spacing 0.09em, semibold, muted |
| Reading value | 20 px, tabular |
| Log and terminal | 10.5 to 11 px, Plex Mono, 17 px line |
| Hints and metadata | 10.5 to 11 px, faint |

## Shell and layout

- Sidebar 212 px, topbar 38 px, body padding 8 px, gap 8 px.
- Sidebar sections are uppercase labels with 22 px rows. Nested rows indent 14 px per level. The
  selected row has the accent tint and a 2 px left bar.
- Each screen declares its grid in the page (`grid-template-rows`, `grid-template-columns`). Panels
  use `minmax(0, 1fr)` so they shrink, never grow past their cell.
- Short screens: under 760 px tall, the extra rows of a long list (log lines, last boots, signature
  checks, install details) are hidden with the `more-s` class. Taller screens show them. Nothing is
  cut in the middle of a line.

## Components

**Panel.** Header 28 px (`.ph`), body (`.pb`) with 8 px and 10 px padding, or flush for tables.
Headers may hold a count chip, a kicker (faint text), and a button on the right.

**Button.** 22 px tall, 2 px radius, 11.5 px text. `.pri` (accent fill) is the one primary action in
an area. Default is outlined. `.quiet` has no border. `.danger` is red text on a red outline, used
only inside a confirmation. Small (`.sm`) is 18 px. Disabled text is `--fg-disabled`.

**Chip.** 17 px tall, square-ish, 10.5 px. Words first, dot second. Colour follows the state:
ok (running), warn (needs you later: trial, waiting for Orion, recovery), err (failed, over a
limit), info (seen, removable, scanning), acc (needs you now: boot ROM waiting, offer), off (offline,
never seen: dashed outline), neutral (a fact: a version, a link). Pinned is neutral.

**Dot.** 7 px. Ok, warn, err, info, acc, off (solid grey), ring (dashed, never seen), spin
(scanning). Dots never carry meaning without a chip word next to them.

**Row and table.** A grid with a fixed height (26 px, or 34 px for device records). Cells truncate.
Header row 22 px uppercase. Group rows (robot, boards, disks, offline, never seen) are 22 px on the
muted surface. Selected rows take the accent tint with a left bar. Child rows indent 14 px.

**Slot.** Two lines: name and state chip, then a detail line (or a progress bar while writing).
Actions sit on the right. The left bar shows the state: warn for a trial, accent for a write, ok for
the default, red for a confirmation. Five states are in the mockups: writing, trial, fallback,
pinned, and removal in place.

**Keep-last-N line.** Under the slots: "Keep last [3] · pinned and default slots stay". The default
is also set in Settings. A board can differ.

**Reading tile.** Label, big number with its unit, a foot line (limit, range, source), and a 34 px
sparkline across the bottom. Five tiles across the top of the board page.

**Stepper.** Six steps in a column: done (green tick), current (accent ring with a dot), pending
(grey number), failed (red "!"). Each step has one line of detail that says what it is for now.

**Tabs.** Plain radio inputs with labels (`.dock`). No script. The active tab has the accent
underline and bold text. Only the active pane is displayed.

**Logs and terminal.** Monospace, one line per row, with time, level and source. Terminal output is
plain text with a cursor. Logs are newest first on the board page.

**Switch and select.** A 30 by 16 px switch, off by default for security. Selects are 22 px.

**Confirmation in place.** A row or footer line turns the error tint, shows the question and two
buttons (Cancel, and the danger action). Same height, same place.

**Notes.** A 2 px left bar, muted text. `warn`, `err`, `ok`, `info`, `acc` variants.

**Empty box.** A dashed frame with one line of plain text and at most one action.

## States

[`mockups/states.html`](mockups/states.html) shows every component in default, hover, selected,
offline, empty, loading and error, in one matrix. Use it to check alignment. Hover is a background
tint; selected is the accent tint and left bar; offline is the grey dot and muted text; loading is
a grey skeleton bar (the only pulsing thing besides spinners); error is the red tint or red text.

## Motion

- No entrance animation on anything.
- No transitions on values. Live numbers and sparklines snap to new data.
- Spinners rotate (1 s, linear). Nothing else loops.
- Hover and press change colour instantly.

## Charts

- Sparklines are SVG paths in a `preserveAspectRatio="none"` viewBox, with a 14 percent area fill
  and a 1.5 px non-scaling line. Use them for trends in the last 60 seconds or the last day.
- Larger charts (SystemCore) are 100 px tall and have their axis values in the row text, not inside
  the SVG, so stretching never distorts text.
- Colour order: series 1 (blue) first, then teal, amber, violet, grey, rose, green.

## Verification

Measured with headless Chromium (chrome-headless-shell 1248) through the DevTools protocol.
For each screen, at 1024x680, 1320x860 and 1830x1030, in light and dark:

- the document is not scrollable (scroll size equals the viewport);
- no element's box extends past its parent's box, or past the viewport;
- no overflow-hidden element clips content vertically or horizontally, except text that is
  truncated on purpose with an ellipsis (counted separately).

## Open for the user's ruling

These are choices made in the mockups. Each is easy to change in the mockups before the screens are
built.

1. **Board page: one page with a tabbed dock.** Readings and slots are always visible; Logs, Files
   and Terminal share the lower right panel as tabs. The alternative is one page per tab.
2. **Removing a slot confirms in place.** The row turns red with Cancel and Remove slot. No modal.
3. **Reboot has no confirmation.** It is recoverable: a trial falls back, and a plain reboot comes
   back on the same slot.
4. **SystemCore before Orion.** Boards behind the controller show as "Seen, not reached" until
   Orion is installed there. Nothing about them is shown as running before then.
5. **Robot is a group in the tree and in the list, read-only.** The robot's devices are shown with
   readings but no controls.
6. **Scanning is a header chip, never a blocking overlay.** The list stays live while it scans.
7. **Short screens hide the extra rows** (under 760 px tall). The first rows of each list stay.
8. **Pages list boards by record, not by link.** Links are chips in the row. There is no separate
   "USB" and "network" list.
9. **Security is one switch in Settings, off.** The "What is checked when on" text states what it
   covers, and a failed check keeps the current slot.
10. **Install sizes are shown on the Settings and SystemCore pages** (lean rule: report the flasher
    image, the SystemCore package, and the app).
