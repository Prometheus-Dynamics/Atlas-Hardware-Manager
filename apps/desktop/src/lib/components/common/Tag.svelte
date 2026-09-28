<script lang="ts">
  import { toneText, type Tone } from "$lib/format";

  let { tone = "neutral", label, icon, title }: { tone?: Tone; label: string; icon?: string; title?: string } =
    $props();
</script>

<span class="tag {toneText[tone]}" {title}>
  {#if icon}<i class="fa-solid {icon} text-[0.55rem]" aria-hidden="true"></i>{/if}{label}
</span>
