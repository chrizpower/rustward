<script lang="ts">
  import { onMount } from 'svelte';
  import ActionButton from './ActionButton.svelte';
  import PagedList from './PagedList.svelte';
  import { control, type Gap, type Occurrence, type ScheduleHistory } from './api';
  import { href, label, utc, type Actions } from './view';

  let { id, actions, updated }: { id: string; actions: Actions; updated: (message: string) => void } = $props();
  let history = $state<ScheduleHistory>();

  onMount(() => {
    control('schedule_history', { schedule_id: id }).then(value => { history = value; }).catch(actions.showError);
  });
</script>

{#if history}
  {@const paused = history.paused}
  <h1>{id}</h1>
  {#if history.active}
  <div class="actions">
    <ActionButton {actions} describedBy="schedule-effect" primary action={async () => {
      await control(paused ? 'schedule_resume' : 'schedule_pause', { schedule_id: id });
      updated(paused ? 'Schedule resumed.' : 'Schedule paused.');
    }}>{paused ? 'Resume schedule' : 'Pause schedule'}</ActionButton>
    {#if history.global_paused}<span>All schedules paused</span>{/if}
  </div>
  <p id="schedule-effect" class="muted">A schedule pause stops new requests from this schedule. Queued runs can still start.</p>
  {:else}
    <p class="notice">This schedule has been removed. Its history is available below.</p>
  {/if}
  <h2>Occurrences</h2>
  <PagedList {actions} initial={history.occurrences} more="Show more occurrences"
    cursor={(occurrence: Occurrence) => occurrence.due_ms}
    load={async after_due_ms => (await control('schedule_history', { schedule_id: id, after_due_ms })).occurrences}>
    {#snippet row(occurrence)}
      <div class="entry">
        <time datetime={new Date(occurrence.due_ms).toISOString()}>{utc(occurrence.due_ms)}</time>
        <span>{label(occurrence.decision)}</span>
        {#if occurrence.run_id}<a href={href('run', occurrence.run_id)}>View run</a>{/if}
      </div>
    {/snippet}
  </PagedList>
  <h2>Schedule gaps</h2>
  <PagedList {actions} initial={history.gaps} more="Show more gaps"
    cursor={(gap: Gap) => gap.id}
    load={async after_gap_id => (await control('schedule_history', { schedule_id: id, after_gap_id })).gaps}>
    {#snippet row(gap)}
      <div class="entry">
        <span><code>{utc(gap.after_ms)}</code> to <time datetime={new Date(gap.through_ms).toISOString()}>{utc(gap.through_ms)}</time></span>
        <span class="status attention">{label(gap.reason)}</span>
        <span>Missed: {gap.missed_count ?? 'Unknown'}</span>
      </div>
    {/snippet}
  </PagedList>
{:else}
  <h1>Schedule details</h1>
{/if}
