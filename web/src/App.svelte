<script lang="ts">
  import { onMount } from 'svelte';
  import { api, control } from './api';
  import ActionButton from './ActionButton.svelte';
  import PagedList from './PagedList.svelte';
  import ReviewPanel from './ReviewPanel.svelte';
  import RunDetails from './RunDetails.svelte';
  import ScheduleDetails from './ScheduleDetails.svelte';
  import ServiceStatus from './ServiceStatus.svelte';
  import TaskForm from './TaskForm.svelte';
  import { errorMessage, href, label, utc, type Actions } from './view';

  const navigation = [
    { id: 'tasks', label: 'Tasks' },
    { id: 'runs', label: 'Runs' },
    { id: 'schedules', label: 'Schedules' },
    { id: 'reviews', label: 'Review' },
  ];
  let section = $state('');
  let id = $state('');
  let version = $state(0);
  let error = $state('');
  let service: ReturnType<typeof ServiceStatus>;
  const serviceActions: Actions = {
    clearError: () => { error = ''; },
    showError: issue => { error = errorMessage(issue); },
  };
  const activeSection = $derived(section === 'task' ? 'tasks' : section === 'run' ? 'runs' :
    section === 'schedule' ? 'schedules' : section);

  function render() {
    version++;
    error = '';
    try {
      [section, id = ''] = (location.hash.slice(1) || 'tasks').split('/').map(decodeURIComponent);
    } catch (issue) {
      section = '';
      serviceActions.showError(issue);
    }
  }

  function viewActions(at: number): Actions {
    return {
      clearError: () => { if (at === version) serviceActions.clearError(); },
      showError: issue => { if (at === version) serviceActions.showError(issue); },
    };
  }

  onMount(() => {
    render();
    window.addEventListener('hashchange', render);
    return () => window.removeEventListener('hashchange', render);
  });
</script>

<header class="top">
  <a class="brand" href="#tasks">Rustward</a>
  <nav aria-label="Console">
    {#each navigation as item}
      <a href={href(item.id)} aria-current={activeSection === item.id ? 'page' : undefined}>{item.label}</a>
    {/each}
  </nav>
  <ServiceStatus bind:this={service} actions={serviceActions} />
</header>
<main>
  <p id="error" role="alert" hidden={!error}>{error}</p>
  <section id="content" aria-label="Current view">
    {#key version}
      {@const at = version}
      {@const actions = viewActions(at)}
      {@const reload = () => { if (at === version) render(); }}
      {#if section === 'task'}
        <TaskForm {id} {actions} openRun={runId => { if (at === version) location.hash = href('run', runId); }} />
      {:else if section === 'runs'}
        {@const queued = id === 'queued'}
        <h1>{queued ? 'Queue' : 'Run history'}</h1>
        <div class="actions"><a href={href('runs', queued ? 'all' : 'queued')}>{queued ? 'All runs' : 'Queued runs'}</a></div>
        <PagedList {actions} load={(before?: number) => api('runs', { queued, before })} cursor={run => run.sequence}>
          {#snippet row(run)}
            <div class="entry">
              <a href={href('run', run.run_id)}>{run.task_id}</a>
              <span class="status">{label(run.result ?? run.phase)}</span>
              <span class="muted">{run.schedule_id ? `${run.schedule_id} · ${utc(run.due_ms)}` : 'Manual'}</span>
              {#if run.phase === 'queued'}
                <span class="muted run-runtime">Runtime: <code>{run.runtime_id}</code></span>
              {/if}
            </div>
          {/snippet}
        </PagedList>
      {:else if section === 'run'}
        <RunDetails {id} {actions} />
      {:else if section === 'schedules'}
        <h1>Schedules</h1>
        <div class="actions">
          <ActionButton {actions} action={async () => { await control('schedule_pause', {}); reload(); }}>Pause all schedules</ActionButton>
          <ActionButton {actions} action={async () => { await control('schedule_resume', {}); reload(); }}>Resume all schedules</ActionButton>
        </div>
        <PagedList {actions} load={(after?: string) => api('schedules', { after })} cursor={schedule => schedule.id}>
          {#snippet row(schedule)}
            <div class="entry">
              <a href={href('schedule', schedule.id)}>{schedule.id}</a>
              <span class="muted">{schedule.cron} UTC</span>
              <span class="status">{schedule.paused ? 'Paused' : 'Enabled'}</span>
              <span class="muted">Last: {utc(schedule.last_ms)}</span>
              <span>Next: {utc(schedule.next_ms)}</span>
            </div>
          {/snippet}
        </PagedList>
      {:else if section === 'schedule'}
        <ScheduleDetails {id} {actions} {reload} />
      {:else if section === 'reviews'}
        <h1>Review</h1>
        <PagedList {actions} empty="No pending reviews." more="Show more reviews" pageSize={10}
          load={(after?: string) => control('reviews', { after: after ?? null })} cursor={review => review.run_id}>
          {#snippet row(review)}
            <ReviewPanel {review} {actions} acknowledged={async () => { await service.refresh(); reload(); }} />
          {/snippet}
        </PagedList>
      {:else if section !== ''}
        <h1>Tasks</h1>
        <PagedList {actions} load={(after?: string) => api('tasks', { after })} cursor={task => task.id}>
          {#snippet row(task)}
            <div class="entry">
              <a href={href('task', task.id)}>{task.label}</a>
              <span class="muted">{task.id}</span>
            </div>
          {/snippet}
        </PagedList>
      {/if}
    {/key}
  </section>
</main>
