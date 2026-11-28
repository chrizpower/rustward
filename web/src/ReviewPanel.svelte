<script lang="ts">
  import { onMount, tick } from 'svelte';
  import ActionButton from './ActionButton.svelte';
  import { control, type Review } from './api';
  import { href, label, readSession, type Actions } from './view';

  type Pending = { id: string; note: string; version: number };
  let { review, actions, acknowledged }: {
    review: Review;
    actions: Actions;
    acknowledged: () => Promise<void>;
  } = $props();
  const id = $derived('review-' + review.run_id);
  const stash = $derived('rustward-' + id);
  let note = $state('');
  let noteError = $state(false);
  let field = $state<HTMLTextAreaElement>();

  onMount(() => {
    const saved = readSession<Pending>(stash);
    if (saved?.version === review.evidence_version) note = saved.note;
  });

  async function acknowledge() {
    const submitted = note;
    noteError = !submitted.trim();
    if (noteError) {
      await tick();
      field?.focus();
      return;
    }
    let pending = readSession<Pending>(stash);
    if (pending?.version !== review.evidence_version || pending.note !== submitted) {
      const id = Array.from(crypto.getRandomValues(new Uint8Array(32)),
        byte => byte.toString(16).padStart(2, '0')).join('');
      pending = { id, note: submitted, version: review.evidence_version };
      sessionStorage.setItem(stash, JSON.stringify(pending));
    }
    await control('acknowledge', { acknowledgement_id: pending.id, review, note: pending.note });
    sessionStorage.removeItem(stash);
    await acknowledged();
  }
</script>

<div class="panel">
  <h2>{review.task_id}</h2>
  <a href={href('run', review.run_id)}>View run</a>
  <p class="muted run-id">{review.run_id}</p>
  <p class="status attention">{label(review.state.phase)} · {review.state.result ?? 'Pending'}</p>
  <p class="muted">Runtime: {review.runtime_id}</p>
  <details>
    <summary>Review evidence</summary>
    <pre>{JSON.stringify({ launch: review.launch, times: review.times }, null, 2)}</pre>
  </details>
  {#if review.state.phase === 'Completed'}
    <p id={id + "-effect"}>Acknowledgement allows dispatch to resume when other dispatch blocks clear.</p>
    <label for={id}>Checks, remaining uncertainty, and reason to resume</label>
    {#if noteError}<p id={id + '-error'} class="error-text" role="alert">Enter a review note.</p>{/if}
    <textarea {id} maxlength={4096} bind:this={field} bind:value={note}
      aria-invalid={noteError} aria-describedby={noteError ? id + '-error' : undefined}></textarea>
    <ActionButton {actions} describedBy={id + "-effect"} action={acknowledge}>Acknowledge review</ActionButton>
  {/if}
</div>
