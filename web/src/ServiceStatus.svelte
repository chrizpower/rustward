<script lang="ts">
  import { onMount } from 'svelte';
  import { control, type ServiceStatus } from './api';
  import ActionButton from './ActionButton.svelte';
  import type { Actions } from './view';

  let { actions, paused = $bindable(false), version = $bindable('') }: {
    actions: Actions;
    paused?: boolean;
    version?: string;
  } = $props();
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
    paused = status.paused;
    version = status.version;
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
        paused = false;
        unavailable = true;
      } finally {
        if (active) timer = setTimeout(poll, 1200);
      }
    }
    void poll();
    return () => { active = false; clearTimeout(timer); };
  });
</script>

<div id="service">
  {#if status}
    {@const current = status}
    <span role="status" class:attention={current.paused || current.draining || current.dispatch_block} class:failure={current.failed} class="status">{describe(current)}</span>
    <ActionButton {actions} describedBy="dispatch-effect" disabled={current.failed || current.draining} action={async () => {
      await control(current.paused ? 'resume' : 'pause', {});
      await refresh();
    }}>
      {current.paused ? 'Resume dispatch' : 'Pause dispatch'}
    </ActionButton>
    <span id="dispatch-effect" class="service-note">{current.paused
      ? 'Queued runs can start when dispatch blocks clear.'
      : 'Queued runs wait. Active runs continue.'}</span>
  {:else if unavailable}
    <span role="status">Service unavailable</span>
  {/if}
</div>
