<script lang="ts">
  // Orion: the management agent on running devices. Atlas connects to one
  // node as an operator (never a cluster member); an administrator enrolls
  // it once per node. Everything else in Atlas works without it.
  import { api, errorText, type ImageServerStatus, type OrionConnection } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import Disclosure from "#lib/components/device/Disclosure.svelte";
  import Field from "#lib/components/common/Field.svelte";
  import GlassCard from "#lib/components/common/GlassCard.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import Pill from "#lib/components/common/Pill.svelte";
  import type { Tone } from "#lib/format.ts";
  import { toasts } from "#lib/stores/toasts.svelte.ts";

  let connection = $state<OrionConnection | null | undefined>(undefined);
  let url = $state("");
  let key = $state("");
  let showKey = $state(false);

  // The HTTP server boards download update images from.
  let images = $state<ImageServerStatus | null>(null);
  let imagePort = $state("7700");
  let imageHost = $state("");

  $effect(() => {
    void api.orionConnection().then((value) => {
      connection = value;
      url = value?.url ?? "";
    });
    void api.imageServerStatus().then((value) => {
      images = value;
      if (value) {
        imagePort = String(value.port);
        imageHost = value.host ?? "";
      }
    });
  });

  const imagesLine = $derived.by((): string => {
    if (!images) return "";
    if (images.error) return images.error;
    if (!images.listening) return `Starts on port ${images.port} when an update through Orion needs it.`;
    const where = images.addresses.length ? images.addresses.join(", ") : "no network address yet";
    return `Listening on port ${images.port}: ${where}.`;
  });

  async function saveImages() {
    const port = Number(imagePort.trim());
    if (!Number.isInteger(port) || port < 1 || port > 65535) {
      toasts.error("Choose a port from 1 to 65535.");
      return;
    }
    images = await api.setImageServer(port, imageHost.trim() || null);
    if (images?.error) toasts.error(images.error);
    else toasts.success("Image server saved.");
  }

  const status = $derived.by((): { tone: Tone; label: string } => {
    if (!connection?.url) return { tone: "neutral", label: "Not set up" };
    if (connection.error) return { tone: "error", label: "Can't connect" };
    if (!connection.connected) return { tone: "neutral", label: "Not connected yet" };
    if (!connection.enrolled) return { tone: "warning", label: "Waiting for enrollment" };
    return { tone: "success", label: "Connected" };
  });

  async function save() {
    connection = await api.setOrionUrl(url.trim() || null);
    toasts.success(url.trim() ? "Connecting to Orion." : "Orion turned off.");
  }

  async function check() {
    connection = await api.checkOrion();
  }

  async function enroll() {
    connection = await api.enrollOrionWithKey(key);
    key = "";
    showKey = false;
    toasts.success("Enrolled with the node's key.");
  }

  async function copy(text: string, what: string) {
    try {
      await navigator.clipboard.writeText(text);
      toasts.success(`${what} copied.`);
    } catch (error) {
      toasts.error(errorText(error));
    }
  }
</script>

<GlassCard title="Orion" subtitle="Readings, actions, and updates through the agent on your devices" icon="router" large fill>
  {#if connection === undefined}
    <p class="hint">Checking…</p>
  {:else if connection === null}
    <p class="flex items-start gap-2 text-[13px] text-fg-muted">
      <Icon name="info-circle" size={15} class="mt-0.5 shrink-0" />
      Orion is off while Atlas uses simulated devices.
    </p>
  {:else}
    <div class="flex flex-col gap-4">
      <div class="flex items-center gap-2">
        <Pill tone={status.tone} label={status.label} />
        {#if connection.node_id}<span class="mono truncate text-[12px] text-fg-faint">{connection.node_id}</span>{/if}
      </div>

      <Field label="Orion node" hint="Any node of the cluster works; it forwards to the others it's peered with.">
        <div class="flex gap-2">
          <input class="input mono flex-1" bind:value={url} placeholder="orion+tcp://raze-8f3a1c2d.local:9200" aria-label="Orion node address" />
          <Button action={save}>{connection.url ? "Save" : "Connect"}</Button>
          {#if connection.url}<Button variant="ghost" icon="refresh" action={check}>Check</Button>{/if}
        </div>
      </Field>

      {#if connection.error}
        <p class="flex items-start gap-2 text-[12.5px] text-err-fg"><Icon name="alert-circle" size={15} class="mt-0.5 shrink-0" />{connection.error}</p>
      {/if}

      <div class="flex flex-col gap-1.5">
        <span class="text-[13px] font-medium text-fg">Atlas's identity</span>
        <p class="mono break-all text-[12px] text-fg-muted">{connection.operator_id}<br />{connection.fingerprint}</p>
        {#if connection.node_fingerprint}
          <span class="hint">The node's key is pinned: <span class="mono">{connection.node_fingerprint}</span></span>
        {/if}
      </div>

      {#if connection.url && !connection.enrolled}
        <div class="flex flex-col gap-2">
          <span class="text-[13px] font-medium text-fg">Let Atlas in</span>
          <span class="hint">Run this on the device (check the fingerprint matches the one above), then press Check.</span>
          <div class="command">
            <code class="mono">{connection.enroll_command}</code>
            <Button size="sm" variant="ghost" icon="copy" onclick={() => copy(connection!.enroll_command, "Command")}>Copy</Button>
          </div>
          {#if showKey}
            <div class="flex gap-2">
              <input class="input mono flex-1" type="password" bind:value={key} placeholder="Enrollment key" aria-label="Enrollment key" />
              <Button action={enroll} disabled={!key.trim()}>Enroll</Button>
            </div>
            <span class="hint">Key enrollment gives read access plus the node's default actions only.</span>
          {:else}
            <button type="button" class="link self-start text-[12.5px]" onclick={() => (showKey = true)}>Use the node's enrollment key instead</button>
          {/if}
        </div>
      {/if}

      {#if images}
        <Disclosure title="Image server">
          <div class="flex flex-col gap-3">
            <p class="text-[12.5px] {images.error ? 'text-err-fg' : 'text-fg-muted'}">
              Boards download updates from Atlas over HTTP. {imagesLine}
            </p>
            <div class="flex flex-wrap items-end gap-2">
              <Field label="Port" class="w-24">
                <input class="input mono" inputmode="numeric" bind:value={imagePort} aria-label="Image server port" />
              </Field>
              <Field label="Host in URLs" class="min-w-0 flex-1">
                <input class="input mono" bind:value={imageHost} placeholder="automatic" aria-label="Image server host" />
              </Field>
              <Button action={saveImages}>Save</Button>
            </div>
            <span class="hint">
              The host is used only when Atlas can't tell which of its addresses reaches a board. If downloads fail, allow
              this TCP port through the computer's firewall.
            </span>
          </div>
        </Disclosure>
      {/if}
    </div>
  {/if}
</GlassCard>

<style>
  .command {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 10px 12px;
    border-radius: var(--r-md);
    background: var(--inset);
    border: 1px solid var(--hairline);
  }
  code {
    flex: 1;
    min-width: 0;
    font-size: 11.5px;
    line-height: 1.5;
    color: var(--fg-muted);
    word-break: break-all;
  }
</style>
