<script lang="ts">
  import { onMount } from 'svelte';
  import ActionButton from './ActionButton.svelte';
  import PagedList from './PagedList.svelte';
  import { control, type Gap, type Occurrence, type ScheduleHistory } from './api';
  import { href, label, utc, type Actions } from './view';

  let { id, actions, reload }: { id: string; actions: Actions; reload: () => void } = $props();
  let history = $state<ScheduleHistory>();

  onMount(() => {
    control('schedule_history', { schedule_id: id }).then(value => { history = value; }).catch(actions.showError);
  });
</script>

{#if history}
  {@const paused = history.paused}
  <h1>{id}</h1>
  <div class="actions">
    <ActionButton {actions} action={async () => {
      await control(paused ? 'schedule_resume' : 'schedule_pause', { schedule_id: id });
      reload();
    }}>{paused ? 'Resume schedule' : 'Pause schedule'}</ActionButton>
    {#if history.global_paused}<span>All schedules paused</span>{/if}
  </div>
  <h2>Occurrences</h2>
  <PagedList {actions} initial={history.occurrences} more="Show more occurrences"
    cursor={(occurrence: Occurrence) => occurrence.due_ms}
    load={async after_due_ms => (await control('schedule_history', { schedule_id: id, after_due_ms })).occurrences}>
    {#snippet row(occurrence)}
      <div class="entry">
        <span>{utc(occurrence.due_ms)}</span>
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
      <div class="entry">{utc(gap.after_ms)} to {utc(gap.through_ms)} · {label(gap.reason)} · Missed: {gap.missed_count ?? 'unknown'}</div>
    {/snippet}
  </PagedList>
{/if}
