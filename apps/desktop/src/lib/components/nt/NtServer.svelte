<script lang="ts">
  // A plain NT4 server on this computer, for testing a PhotonVision or
  // HeliOS camera without a roboRIO: it holds only the topics made here
  // and what clients publish, nothing else. It shows where to point a
  // camera, who is connected, and what they write.
  import { api, errorText, type NtCameraAddress, type NtServerInfo, type NtServerTopic } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import Checkbox from "#lib/components/common/Checkbox.svelte";
  import GlassCard from "#lib/components/common/GlassCard.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import Pill from "#lib/components/common/Pill.svelte";
  import { toasts } from "#lib/stores/toasts.svelte.ts";
  import { onDestroy, onMount } from "svelte";
  import { TYPES, editText, formatNt, parseNt } from "./nt.ts";

  let info = $state<NtServerInfo | null>(null);
  let cameras = $state<NtCameraAddress[]>([]);
  let port = $state(5810);
  let writes = $state<{ at: number; client: string; name: string; value: string }[]>([]);
  let newName = $state("/");
  let newType = $state<string>("double");
  let newValue = $state("");
  /** What is typed into each topic's value box, until it is set. */
  let editing = $state<Record<string, string>>({});
  let refresh: ReturnType<typeof setTimeout> | null = null;
  let poll: ReturnType<typeof setInterval> | null = null;

  async function load() {
    info = await api.ntServerInfo();
  }

  /** Re-reads soon (a burst of changes is one read). */
  function soon() {
    if (refresh) return;
    refresh = setTimeout(() => {
      refresh = null;
      void load();
    }, 150);
  }

  async function watch() {
    await api.ntServerWatch((frame) => {
      if (frame.type === "wrote") {
        writes = [{ at: Date.now(), client: frame.client, name: frame.name, value: formatNt(frame.value) }, ...writes].slice(0, 15);
        soon();
      } else if (frame.type === "warning") {
        toasts.error(frame.message);
      } else {
        soon();
      }
    });
    // Values clients publish without a new topic: keep the table current.
    poll ??= setInterval(() => void load(), 1000);
  }

  onMount(async () => {
    cameras = await api.ntCameraAddresses().catch(() => []);
    await load();
    if (info) {
      port = info.port;
      await watch();
    }
  });
  onDestroy(() => {
    if (poll) clearInterval(poll);
    if (refresh) clearTimeout(refresh);
  });

  async function start() {
    info = await api.ntServerStart(port === 5810 ? null : port);
    cameras = await api.ntCameraAddresses().catch(() => []);
    await watch();
  }

  async function stop() {
    await api.ntServerStop();
    if (poll) clearInterval(poll);
    poll = null;
    info = null;
    writes = [];
  }

  async function setValue(topic: NtServerTopic) {
    const text = editing[topic.name];
    if (text === undefined) return;
    try {
      await api.ntServerSet(topic.name, null, parseNt(topic.type, text));
      delete editing[topic.name];
      await load();
    } catch (error) {
      toasts.error(errorText(error));
    }
  }

  async function add() {
    const name = newName.trim();
    if (!name || name === "/") return;
    try {
      const value = newValue.trim() === "" && newType !== "string" ? null : parseNt(newType, newValue);
      await api.ntServerSet(name.startsWith("/") ? name : `/${name}`, newType, value);
      newName = name.slice(0, name.lastIndexOf("/") + 1) || "/";
      newValue = "";
      await load();
    } catch (error) {
      toasts.error(errorText(error));
    }
  }

  const owner = (topic: NtServerTopic) =>
    topic.owner === "local" ? "Made here" : topic.owner === "client" ? `From ${topic.publisher ?? "a client"}` : "Kept, no publisher";
  const time = (at: number) => new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
</script>

<div class="flex flex-col gap-4">
  <GlassCard title="Local NetworkTables server" subtitle="An NT4 server on this computer, with only the topics you make here and what cameras publish. Not a robot simulator.">
    {#snippet actions()}
      {#if info}<Pill tone="success" label="Running on port {info.port}" />{/if}
    {/snippet}
    <div class="flex flex-wrap items-end gap-3">
      <label class="flex w-28 flex-col gap-1">
        <span class="text-[12px] text-fg-faint">Port</span>
        <input class="input mono" type="number" min="1" max="65535" bind:value={port} disabled={!!info} aria-label="Server port" />
      </label>
      {#if info}
        <Button icon="player-stop" action={stop}>Stop the server</Button>
      {:else}
        <Button variant="primary" icon="player-play" action={start}>Start the server</Button>
      {/if}
      {#if port !== 5810}<span class="text-[12px] text-warn-fg">Cameras expect 5810 unless you change theirs too.</span>{/if}
    </div>
  </GlassCard>

  {#if info}
    <GlassCard title="Point a camera at this computer" subtitle="Give the camera this computer's address as its NetworkTables server. A team number won't work: it means the robot (10.TE.AM.2).">
      {#if cameras.length}
        <ul class="flex flex-col gap-2">
          {#each cameras as camera (camera.name + camera.address)}
            <li class="glass flex flex-wrap items-center gap-x-3 gap-y-1 px-3.5 py-2.5">
              <Icon name="camera" size={16} />
              <span class="font-medium text-fg">{camera.name}</span>
              <span class="text-[12px] text-fg-faint">at {camera.device_ip}: set its server to</span>
              <span class="mono address">{camera.address}</span>
              <Button size="sm" variant="ghost" icon="copy" label="Copy {camera.address}" onclick={() => void navigator.clipboard?.writeText(camera.address)} />
            </li>
          {/each}
        </ul>
      {/if}
      <p class="mt-3 text-[12.5px] text-fg-muted">
        This computer answers on {#each info.addresses as address, i (address)}<span class="mono address-inline">{address}</span>{i < info.addresses.length - 1 ? ", " : ""}{/each}, port {info.port}.
        Use the one on the camera's network (over USB, the address on the board's USB link).
      </p>
      <ol class="steps mt-3 text-[12.5px] text-fg-muted">
        <li>In PhotonVision, open <b>Settings → Networking</b>.</li>
        <li>Set <b>Team Number / NetworkTables Server Address</b> to the address above, and save.</li>
        <li>The camera connects within a few seconds and shows under Clients; its topics appear below.</li>
      </ol>
    </GlassCard>

    <div class="grid gap-4 lg:grid-cols-[minmax(0,1.6fr)_minmax(0,1fr)]">
      <GlassCard title="Topics" subtitle="{info.topics.length} on the server">
        <form class="add mb-3 flex flex-wrap items-end gap-2" onsubmit={(event) => (event.preventDefault(), void add())}>
          <input class="input mono min-w-[12rem] flex-1" bind:value={newName} placeholder="/Camera/exposure" aria-label="New topic name" />
          <select class="input w-28" bind:value={newType} aria-label="New topic type">
            {#each TYPES as type (type)}<option value={type}>{type}</option>{/each}
          </select>
          <input class="input mono w-40" bind:value={newValue} placeholder={newType.endsWith("[]") ? "1, 2, 3" : newType === "boolean" ? "true" : "value"} aria-label="New topic value" />
          <Button size="sm" icon="plus" type="submit" disabled={newName.trim().length < 2}>Add topic</Button>
        </form>
        {#if info.topics.length === 0}
          <p class="text-[13px] text-fg-muted">No topics yet. Add one above, or point a camera here and its topics appear.</p>
        {:else}
          <div class="table">
            {#each info.topics as topic (topic.name)}
              <div class="trow">
                <span class="mono name" title={topic.name}>{topic.name}</span>
                <span class="type">{topic.type}</span>
                {#if topic.type === "raw" || topic.type.startsWith("struct") || topic.type === "msgpack" || topic.type === "protobuf"}
                  <span class="mono value">{formatNt(topic.value)}</span>
                {:else}
                  <input
                    class="input mono value-input"
                    value={editing[topic.name] ?? editText(topic.value)}
                    oninput={(event) => (editing[topic.name] = event.currentTarget.value)}
                    onkeydown={(event) => event.key === "Enter" && void setValue(topic)}
                    onblur={() => void setValue(topic)}
                    aria-label="Value of {topic.name}"
                  />
                {/if}
                <span class="owner">{owner(topic)}</span>
                <Checkbox
                  checked={topic.persistent}
                  label="Keep {topic.name} across restarts"
                  onclick={() => void api.ntServerPersistent(topic.name, !topic.persistent).then(load)}
                />
                <Button size="sm" variant="ghost" icon="trash" label="Delete {topic.name}" action={() => api.ntServerDelete(topic.name).then(load)} />
              </div>
            {/each}
          </div>
        {/if}
      </GlassCard>

      <div class="flex flex-col gap-4">
        <GlassCard title="Clients" subtitle={info.clients.length ? `${info.clients.length} connected` : "None yet"}>
          {#if info.clients.length === 0}
            <p class="text-[13px] text-fg-muted">Waiting for a camera or a dashboard to connect.</p>
          {:else}
            <ul class="flex flex-col gap-1.5 text-[13px]">
              {#each info.clients as client (client.id)}
                <li class="flex items-center gap-2">
                  <span class="dot"></span><span class="font-medium text-fg">{client.name}</span>
                  <span class="ml-auto text-[12px] text-fg-faint">{client.publications} published · {client.subscriptions} subscribed</span>
                </li>
              {/each}
            </ul>
          {/if}
        </GlassCard>
        <GlassCard title="What clients wrote" subtitle="The latest 15">
          {#if writes.length === 0}
            <p class="text-[13px] text-fg-muted">Nothing yet.</p>
          {:else}
            <ul class="flex flex-col gap-1 text-[12.5px]">
              {#each writes as write (write.at + write.name)}
                <li class="flex gap-2">
                  <span class="text-fg-faint">{time(write.at)}</span>
                  <span class="text-fg-muted">{write.client}</span>
                  <span class="mono min-w-0 truncate text-fg" title={write.name}>{write.name}</span>
                  <span class="mono ml-auto text-fg">{write.value}</span>
                </li>
              {/each}
            </ul>
          {/if}
        </GlassCard>
      </div>
    </div>
  {/if}
</div>

<style>
  .address {
    font-size: 14px;
    font-weight: 600;
    color: var(--fg);
  }
  .address-inline {
    color: var(--fg);
  }
  .steps {
    list-style: decimal;
    padding-left: 1.25rem;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .table {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .trow {
    display: grid;
    grid-template-columns: minmax(0, 1.6fr) 5rem minmax(0, 1fr) 8rem auto auto;
    align-items: center;
    gap: 8px;
    font-size: 13px;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--fg);
  }
  .type,
  .owner {
    font-size: 11.5px;
    color: var(--fg-faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .value-input {
    height: 28px;
    font-size: 12.5px;
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--ok);
  }
</style>
