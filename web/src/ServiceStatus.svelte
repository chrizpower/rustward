<script lang="ts">
  import { onMount } from 'svelte';
  import { control, type ServiceStatus } from './api';
  import ActionButton from './ActionButton.svelte';
  import type { Actions } from './view';

  let { actions }: { actions: Actions } = $props();
  let status = $state<ServiceStatus>();
  let unavailable = $state(false);

  function describe(value: ServiceStatus) {
    if (value.failed) return 'Service failed';
    if (value.draining) return 'Draining';
    if (value.paused) return 'Dispatch paused';
    if (value.dispatch_block === 'ReviewRequired') return 'Review required';
    if (value.dispatch_block === 'LogCapacity') return 'Log capacity blocked';
    return `${value.active_runs} active`;
  }

  export async function refresh() {
    status = await control('service_status', {});
    unavailable = false;
  }

  onMount(() => {
    let active = true;
    let timer: ReturnType<typeof setTimeout>;
    async function poll() {
      try {
        await refresh();
      } catch {
        status = undefined;
        unavailable = true;
      } finally {
        if (active) timer = setTimeout(poll, 1200);
      }
    }
    void poll();
    return () => { active = false; clearTimeout(timer); };
  });
</script>

<div id="service" aria-live="polite">
  {#if status}
    {@const current = status}
    <span class="service-version">Version {current.version}</span>
    <span>{describe(current)}</span>
    <ActionButton {actions} disabled={current.failed || current.draining} action={async () => {
      await control(current.paused ? 'resume' : 'pause', {});
      await refresh();
    }}>
      {current.paused ? 'Resume dispatch' : 'Pause dispatch'}
    </ActionButton>
  {:else if unavailable}
    Service unavailable
  {/if}
</div>
