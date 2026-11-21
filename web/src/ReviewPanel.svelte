<script lang="ts">
  import { onMount } from 'svelte';
  import ActionButton from './ActionButton.svelte';
  import { ApiError, control, type Review } from './api';
  import { href, readSession, type Actions } from './view';

  type Pending = { id: string; note: string; version: number };
  let { review, actions, acknowledged }: {
    review: Review;
    actions: Actions;
    acknowledged: () => Promise<void>;
  } = $props();
  const id = $derived('review-' + review.run_id);
  const stash = $derived('rustward-' + id);
  let note = $state('');

  onMount(() => {
    const saved = readSession<Pending>(stash);
    if (saved?.version === review.evidence_version) note = saved.note;
  });

  async function acknowledge() {
    const submitted = note;
    if (!submitted.trim()) throw new ApiError('Enter a review note.');
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
  <a href={href('run', review.run_id)}>View interrupted run</a>
  <p>{review.state.phase} · {review.state.result ?? 'Pending'}</p>
  <p class="muted">Runtime: {review.runtime_id}</p>
  <details>
    <summary>Review evidence</summary>
    <pre>{JSON.stringify({ launch: review.launch, times: review.times }, null, 2)}</pre>
  </details>
  {#if review.state.phase === 'Completed'}
    <label for={id}>Checks, remaining uncertainty, and reason to resume</label>
    <textarea {id} maxlength={4096} bind:value={note}></textarea>
    <ActionButton {actions} action={acknowledge} primary>Acknowledge review</ActionButton>
  {/if}
</div>
