<script lang="ts">
  // The status ring's looks through Lemnos (looks.* and light.brightness over
  // Orion): the look presets, one active (the board keeps it), each viewable
  // and editable as its look file, saved under a name; the ring's
  // brightness; and Find it / Clear my looks.
  import { api, errorText, type DeviceKey, type HardwareDevice } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import ConfirmButton from "#lib/components/common/ConfirmButton.svelte";
  import { toasts } from "#lib/stores/toasts.svelte.ts";

  let { device, deviceKey }: { device: HardwareDevice; deviceKey: DeviceKey } = $props();

  const BUILTIN = new Set(["scheme-a", "scheme-b", "scheme-c"]);

  let presets = $state<string[]>([]);
  let active = $state<string | null>(null);
  let error = $state<string | null>(null);
  /** The preset open in the editor: its name and its look file. */
  let editing = $state<{ name: string; body: string; saveAs: string } | null>(null);
  /** The brightness set here (the board's own isn't read yet). */
  let brightness = $state<number | null>(null);
  let keepBrightness = $state(false);

  const call = (name: string, args: Record<string, string | number | boolean> = {}) => api.hardwareAction(deviceKey, device.id, name, args);

  /** looks.preset.list's text: one per line, "* " marks the active one. */
  function readList(text: unknown) {
    const lines = String(text ?? "")
      .split("\n")
      .map((l) => l.trimEnd())
      .filter((l) => l.trim());
    presets = lines.map((l) => l.replace(/^\*?\s*/, ""));
    active = lines.find((l) => l.startsWith("*"))?.replace(/^\*\s*/, "") ?? null;
  }

  async function load() {
    try {
      readList((await call("looks.preset.list")).text);
      error = null;
    } catch (e) {
      error = errorText(e);
    }
  }
  void load();

  async function apply(name: string) {
    readList((await call("looks.preset.apply", { name })).text);
    toasts.success(`The ring uses ${name} now; the board keeps it.`);
  }

  async function open(name: string) {
    const body = String((await call("looks.preset.show", { name })).text ?? "");
    editing = { name, body, saveAs: BUILTIN.has(name) ? `${name}-mine` : name };
  }

  async function save() {
    if (!editing) return;
    const name = editing.saveAs.trim();
    if (!name) throw new Error("Give the preset a name.");
    if (BUILTIN.has(name)) throw new Error(`${name} is built in; save under another name.`);
    readList((await call("looks.preset.save", { name, body: editing.body })).text);
    toasts.success(`Saved ${name}.`);
    editing = null;
  }

  async function remove(name: string) {
    readList((await call("looks.preset.delete", { name })).text);
    toasts.success(`Deleted ${name}.`);
    if (editing?.name === name) editing = null;
  }

  async function setBrightness(value: number) {
    try {
      const out = await call("light.brightness", { value, persist: keepBrightness });
      brightness = typeof out.brightness === "number" ? out.brightness : value;
    } catch (e) {
      toasts.error(errorText(e));
    }
  }
</script>

<section class="ring" aria-label="Looks of {device.id}">
  {#if error}
    <p class="text-[12.5px] text-fg-muted">{error}</p>
  {:else}
    <div class="row">
      <span class="label">Preset</span>
      <div class="presets" role="radiogroup" aria-label="Look preset">
        {#each presets as name (name)}
          <button type="button" class="preset" class:on={name === active} role="radio" aria-checked={name === active} onclick={() => name !== active && apply(name).catch((e) => toasts.error(errorText(e)))}>
            {name}
          </button>
        {/each}
      </div>
      {#if active}<Button size="sm" variant="ghost" icon="pencil" action={() => open(active!)}>View</Button>{/if}
    </div>

    <div class="row">
      <span class="label">Brightness</span>
      <input
        class="slider min-w-0 flex-1"
        type="range"
        min="0"
        max="1"
        step="0.01"
        value={brightness ?? 1}
        aria-label="Ring brightness"
        onchange={(e) => setBrightness(Number(e.currentTarget.value))}
      />
      <span class="val">{brightness === null ? "not read" : `${Math.round(brightness * 100)}%`}</span>
      <label class="keep" title="Keep this brightness after the board restarts"><input type="checkbox" bind:checked={keepBrightness} /> Keep</label>
    </div>

    <div class="flex flex-wrap gap-2">
      <Button size="sm" icon="focus-2" action={() => call("looks.locate", { seconds: 10 }).then(() => toasts.success(`${device.id} shows Find it for 10 s.`))}>Find it</Button>
      <Button size="sm" variant="ghost" icon="x" action={() => call("looks.off").then(() => toasts.success("Cleared the looks set from here."))}>Clear my looks</Button>
    </div>

    {#if editing}
      <div class="editor">
        <p class="text-[12px] text-fg-faint">{editing.name}: its look file (TOML, Lemnos docs/looks.md).</p>
        <textarea class="input mono" rows="10" bind:value={editing.body} spellcheck="false" aria-label="Look file"></textarea>
        <div class="flex flex-wrap items-center gap-2">
          <input class="input name" bind:value={editing.saveAs} aria-label="Save as" placeholder="Preset name" />
          <Button size="sm" variant="primary" icon="device-floppy" action={save}>Save</Button>
          <Button size="sm" variant="ghost" onclick={() => (editing = null)}>Close</Button>
          {#if !BUILTIN.has(editing.name)}
            <span class="ml-auto"><ConfirmButton size="sm" variant="ghost" icon="trash" prompt="Delete {editing.name}?" confirmLabel="Delete" action={() => remove(editing!.name)}>Delete</ConfirmButton></span>
          {/if}
        </div>
      </div>
    {/if}
  {/if}
</section>

<style>
  .ring {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }
  .label {
    width: 5.5rem;
    flex-shrink: 0;
    font-size: 12px;
    color: var(--fg-faint);
  }
  .presets {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    flex: 1;
    min-width: 0;
  }
  .preset {
    height: 26px;
    padding: 0 10px;
    font-size: 12.5px;
    color: var(--fg-muted);
    border: 1px solid var(--glass-border);
    border-radius: var(--r-sm);
  }
  .preset:hover {
    color: var(--fg);
    background: var(--glass-hover);
  }
  .preset.on {
    color: var(--fg);
    border-color: var(--accent-ring);
    background: var(--accent-tint);
  }
  .slider {
    accent-color: var(--accent);
  }
  .val {
    width: 4.5rem;
    text-align: right;
    font-size: 12.5px;
    font-variant-numeric: tabular-nums;
    color: var(--fg);
  }
  .keep {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 12px;
    color: var(--fg-muted);
  }
  .editor {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 12px;
    background: var(--inset);
    border: 1px solid var(--hairline);
    border-radius: var(--r-card);
  }
  textarea {
    resize: vertical;
    font-size: 12px;
  }
  .name {
    width: 12rem;
  }
</style>
