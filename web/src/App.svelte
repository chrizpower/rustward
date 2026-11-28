<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { api, control } from './api';
  import ActionButton from './ActionButton.svelte';
  import PagedList from './PagedList.svelte';
  import ReviewPanel from './ReviewPanel.svelte';
  import RunTable from './RunTable.svelte';
  import RunDetails from './RunDetails.svelte';
  import ScheduleDetails from './ScheduleDetails.svelte';
  import ServiceStatus from './ServiceStatus.svelte';
  import ThemeControl from './ThemeControl.svelte';
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
  let dispatchPaused = $state(false);
  let serviceVersion = $state('');
  let feedback = $state('');
  let main: HTMLElement;
  let admittedRun: string | undefined;
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

  function navigate() {
    render();
    if (section !== 'run' || id !== admittedRun) feedback = '';
    admittedRun = undefined;
    main.focus();
  }

  function finish(at: number, message: string) {
    if (at !== version) return;
    feedback = message;
    render();
    void tick().then(() => main.focus());
  }

  function viewActions(at: number): Actions {
    return {
      clearError: () => { if (at === version) serviceActions.clearError(); },
      showError: issue => { if (at === version) serviceActions.showError(issue); },
    };
  }

  onMount(() => {
    render();
    window.addEventListener('hashchange', navigate);
    return () => window.removeEventListener('hashchange', navigate);
  });
</script>

<a class="skip-link" href="#content" onclick={event => {
  event.preventDefault();
  main.focus();
}}>Skip to content</a>
<header class="top">
  <div class="top-navigation">
    <a class="brand" href="#tasks">Rustward</a>
    <nav aria-label="Console">
      {#each navigation as item}
        <a href={href(item.id)} aria-current={activeSection === item.id ? 'page' : undefined}>{item.label}</a>
      {/each}
    </nav>
    <ThemeControl />
  </div>
  <ServiceStatus bind:this={service} bind:paused={dispatchPaused} bind:version={serviceVersion} actions={serviceActions} />
</header>
<main id="content" tabindex="-1" bind:this={main}>
  <div class:visually-hidden={!feedback} class="feedback" role="status">{feedback}</div>
  <div id="error" role="alert" class:visually-hidden={!error}>{error}</div>
  <section aria-label="Current view">
    {#key version}
      {@const at = version}
      {@const actions = viewActions(at)}
      {#if section === 'task'}
        <TaskForm {id} {actions} openRun={run => {
          if (at !== version) return;
          if (run.status === 'history_removed') {
            feedback = `Request accepted. Run ${run.run_id} has expired from history.`;
            return;
          }
          admittedRun = run.run_id;
          feedback = 'Request accepted.';
          location.hash = href('run', run.run_id);
        }} />
      {:else if section === 'runs'}
        {@const queued = id === 'queued'}
        <div class="page-heading">
          <h1>{queued ? 'Queue' : 'Runs'}</h1>
          <a class="button primary" href={href('tasks')}>Choose task</a>
        </div>
        {#if dispatchPaused}
          <p class="notice"><strong>Dispatch paused.</strong> Queued runs will wait.</p>
        {/if}
        {#if !queued}<h2>Run history</h2>{/if}
        <div class="actions"><a href={href('runs', queued ? 'all' : 'queued')}>{queued ? 'All runs' : 'Queued runs'}</a></div>
        <PagedList {actions} load={(before?: number) => api('runs', { queued, before })} cursor={run => run.sequence} empty={queued ? "No queued runs." : "No runs yet."}>
          {#snippet list(runs)}
            <RunTable {runs} {queued} />
          {/snippet}
        </PagedList>
      {:else if section === 'run'}
        <RunDetails {id} {actions} />
      {:else if section === 'schedules'}
        <h1>Schedules</h1>
        <div class="actions">
          <ActionButton {actions} describedBy="schedules-effect" action={async () => {
            await control('schedule_pause', {});
            finish(at, 'All schedules paused.');
          }}>Pause all schedules</ActionButton>
          <ActionButton {actions} describedBy="schedules-effect" action={async () => {
            await control('schedule_resume', {});
            finish(at, 'All schedules resumed.');
          }}>Resume all schedules</ActionButton>
        </div>
        <p id="schedules-effect" class="muted">A schedule pause stops new scheduled requests. Queued runs can still start.</p>
        <PagedList {actions} load={(after?: string) => api('schedules', { after })} cursor={schedule => schedule.id}>
          {#snippet list(schedules)}
            <!-- Keyboard users need to focus the scroll region at narrow widths. -->
            <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
            <div class="table-frame" role="region" aria-label="Schedule list" tabindex="0">
              <table aria-label="Schedules">
                <thead>
                  <tr><th scope="col">Schedule</th><th scope="col">Task</th><th scope="col">State</th>
                    <th scope="col">Last occurrence (UTC)</th><th scope="col">Next due (UTC)</th></tr>
                </thead>
                <tbody>
                  {#each schedules as schedule (schedule.id)}
                    <tr>
                      <td><a href={href('schedule', schedule.id)}>{schedule.id}</a>
                        <code class="table-secondary">{schedule.cron} UTC</code></td>
                      <td>{schedule.task_id}</td>
                      <td><span class="status" class:attention={schedule.paused}>{schedule.paused ? 'Paused' : 'Enabled'}</span></td>
                      <td><code>{utc(schedule.last_ms)}</code></td>
                      <td><code>Next: {utc(schedule.next_ms)}</code></td>
                    </tr>
                  {/each}
                </tbody>
              </table>
            </div>
          {/snippet}
        </PagedList>
      {:else if section === 'schedule'}
        <ScheduleDetails {id} {actions} updated={message => finish(at, message)} />
      {:else if section === 'reviews'}
        <h1>Review</h1>
        <PagedList {actions} empty="No pending reviews." more="Show more reviews" pageSize={10}
          load={(after?: string) => control('reviews', { after: after ?? null })} cursor={review => review.run_id}>
          {#snippet row(review)}
            <ReviewPanel {review} {actions} acknowledged={async () => {
              finish(at, 'Review acknowledged.');
              await service.refresh().catch(serviceActions.showError);
            }} />
          {/snippet}
        </PagedList>
      {:else if section === 'tasks'}
        <h1>Tasks</h1>
        <PagedList {actions} load={(after?: string) => api('tasks', { after })} cursor={task => task.id}>
          {#snippet row(task)}
            <div class="entry">
              <a href={href('task', task.id)}>{task.label}</a>
              <span class="muted">{task.id}</span>
            </div>
          {/snippet}
        </PagedList>
      {:else}
        <h1>View unavailable</h1>
        <a href={href('tasks')}>Return to tasks</a>
      {/if}
    {/key}
  </section>
</main>
{#if serviceVersion}
  <footer class="console-footer">Version {serviceVersion}</footer>
{/if}
