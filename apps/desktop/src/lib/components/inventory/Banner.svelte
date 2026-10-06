<script lang="ts">
  import Button from "#lib/components/common/Button.svelte";
  import IconTile from "#lib/components/common/IconTile.svelte";
  import type { IconName } from "#lib/ui/icons.ts";

  let {
    icon,
    tone,
    title,
    text,
    action,
    run,
    ondismiss,
  }: {
    icon: IconName;
    tone: "accent" | "err" | "warn";
    title: string;
    text: string;
    action: string;
    run: () => unknown;
    ondismiss?: () => void;
  } = $props();
</script>

<div class="banner {tone}" role={tone === "err" ? "alert" : "status"}>
  <IconTile {icon} {tone} size={36} />
  <div class="min-w-0 flex-1">
    <p class="text-[14px] font-semibold text-fg">{title}</p>
    <p class="mt-0.5 text-[13px] text-fg-muted">{text}</p>
  </div>
  <Button variant={tone === "accent" ? "tint" : "glass"} action={async () => run()}>{action}</Button>
  {#if ondismiss}
    <Button variant="ghost" size="sm" icon="x" label="Dismiss" onclick={ondismiss} />
  {/if}
</div>

<style>
  .banner {
    position: relative;
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 12px 14px 12px 16px;
    border-radius: var(--r-card);
    background: var(--glass);
    border: 1px solid var(--glass-border);
    overflow: hidden;
  }
  .banner::before {
    content: "";
    position: absolute;
    left: 0;
    top: 10px;
    bottom: 10px;
    width: 3px;
    border-radius: 0 3px 3px 0;
  }
  .accent {
    background: linear-gradient(100deg, var(--accent-tint), var(--glass) 55%);
    border-color: var(--accent-tint-strong);
  }
  .accent::before {
    background: var(--accent);
  }
  .err::before {
    background: var(--err);
  }
  .warn::before {
    background: var(--warn);
  }
</style>
