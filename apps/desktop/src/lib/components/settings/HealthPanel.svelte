<script lang="ts">
  import { api, errorText } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import GlassCard from "#lib/components/common/GlassCard.svelte";
  import IconTile from "#lib/components/common/IconTile.svelte";
  import Skeleton from "#lib/components/common/Skeleton.svelte";
  import { system } from "#lib/stores/system.svelte.ts";
  import { toasts } from "#lib/stores/toasts.svelte.ts";
  import type { IconName } from "#lib/ui/icons.ts";
  import { rise, stagger } from "#lib/ui/motion.ts";

  let fixing = $state<string | null>(null);

  async function runFix(action: string) {
    if (fixing) return;
    fixing = action;
    try {
      toasts.success(await api.fixHealth(action));
      await system.checkHealth();
    } catch (error) {
      toasts.error(errorText(error));
    } finally {
      fixing = null;
    }
  }

  const style: Record<string, { icon: IconName; tone: "ok" | "warn" | "err" }> = {
    ok: { icon: "circle-check", tone: "ok" },
    warning: { icon: "alert-triangle", tone: "warn" },
    error: { icon: "circle-x", tone: "err" },
  };
</script>

<GlassCard
  title={system.healthProblems === 0 ? "This computer is ready" : `${system.healthProblems} thing${system.healthProblems === 1 ? "" : "s"} to fix`}
  subtitle="Checks Atlas runs on this computer"
  icon="shield-check"
  large
  fill
>
  {#snippet actions()}
    <Button variant="ghost" size="sm" icon="refresh" busy={system.checking} onclick={() => system.checkHealth()}>Check again</Button>
  {/snippet}
  <ul class="flex flex-col gap-2">
    {#each system.healthSorted as check, i (check.id)}
      <li class="glass flex items-start gap-3 px-4 py-3" in:rise={{ delay: stagger(i) }}>
        <IconTile icon={style[check.status].icon} tone={style[check.status].tone} size={30} />
        <div class="min-w-0 flex-1">
          <p class="text-[13px] font-semibold text-fg">{check.label}</p>
          <p class="text-[13px] text-fg-muted">{check.detail}</p>
          {#if check.fix && check.status !== "ok"}<p class="mt-1 text-[12.5px] text-fg-faint">{check.fix}</p>{/if}
        </div>
        {#if check.fix_action}
          {@const action = check.fix_action}
          <Button variant="tint" size="sm" icon="tool" busy={fixing === action} disabled={fixing !== null && fixing !== action} onclick={() => runFix(action)}>
            Fix
          </Button>
        {/if}
      </li>
    {:else}
      {#if system.checking}
        <li><Skeleton height={56} /></li>
      {:else}
        <li class="text-[13px] text-fg-muted">No checks reported.</li>
      {/if}
    {/each}
  </ul>
</GlassCard>
