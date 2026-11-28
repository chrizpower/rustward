<script lang="ts">
  import type { RunRow } from './api';
  import { href, label, utc } from './view';

  let { runs, queued }: { runs: RunRow[]; queued: boolean } = $props();
</script>

<!-- Children can overflow at narrow widths. Keyboard users need to focus the scroll region. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div class="table-frame" role="region" aria-label={queued ? 'Queued runs' : 'Run history'} tabindex="0">
  <table aria-label={queued ? 'Queued runs' : 'Run history'}>
    <thead>
      <tr><th scope="col">Run ID</th><th scope="col">Task</th><th scope="col">State</th>
        <th scope="col">Trigger</th><th scope="col">Latest event (UTC)</th></tr>
    </thead>
    <tbody>
      {#each runs as run (run.run_id)}
        {@const event = run.completed_ms ?? run.dispatched_ms ?? run.admitted_ms ?? run.due_ms}
        <tr>
          <td><code title={run.run_id}>{run.run_id.slice(0, 12)}</code></td>
          <td>
            <a href={href('run', run.run_id)}>{run.task_id}</a>
            {#if run.phase === 'queued'}
              <span class="table-secondary">Runtime: <code>{run.runtime_id}</code></span>
            {/if}
          </td>
          <td>
            <span class="status" class:failure={run.result === 'failure' || run.result === 'timeout'}>
              {label(run.phase)}{run.result ? ` · ${label(run.result)}` : ''}
            </span>
            {#if run.review_required}
              <div><span class="status attention">Review required</span></div>
            {/if}
          </td>
          <td>{run.schedule_id ?? 'Manual'}</td>
          <td>
            {#if event === null}
              <span class="muted">Unknown</span>
            {:else}
              <time datetime={new Date(event).toISOString()}>{utc(event)}</time>
              <span class="table-secondary">{run.completed_ms !== null ? 'Completed' :
                run.dispatched_ms !== null ? 'Dispatched' : run.admitted_ms !== null ? 'Admitted' : 'Due'}</span>
            {/if}
          </td>
        </tr>
      {/each}
    </tbody>
  </table>
</div>
