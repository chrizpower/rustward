<script lang="ts">
  import { onMount } from 'svelte';
  import { api, ApiError, control, inputChoices, type Input, type Inputs, type Task } from './api';
  import TaskInput from './TaskInput.svelte';
  import { readSession, type Actions } from './view';

  type Pending = { key: string; inputs: Inputs };
  type Field = { input: Input; value: string; empty: boolean };
  let { id, actions, openRun }: {
    id: string;
    actions: Actions;
    openRun: (id: string) => void;
  } = $props();
  let task = $state<Task>();
  let fields = $state<Field[]>([]);
  let pending = $state(false);
  let busy = $state(false);
  const stash = $derived('rustward-submit-' + id);
  const finalErrors = ['InvalidInputs', 'QueueFull', 'TooLarge', 'InvalidKey', 'PayloadConflict'];

  onMount(() => {
    api('task', { task_id: id }).then(value => {
      const previous = readSession<Pending>(stash);
      fields = value.inputs.map(input => {
        const initial = previous ? (previous.inputs[input.id]?.value ?? null) : input.default;
        const choices = inputChoices(input);
        const index = initial === null ? -1 : (choices?.indexOf(initial) ?? -1);
        return {
          input,
          value: choices ? (index < 0 ? '' : String(index)) : String(initial ?? ''),
          empty: !choices && initial === '',
        };
      });
      pending = previous !== null;
      task = value;
    }).catch(actions.showError);
  });

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (busy || !task) return;
    busy = true;
    actions.clearError();
    try {
      const inputs: Inputs = Object.create(null);
      for (const { input, value, empty } of fields) {
        if (empty) {
          inputs[input.id] = { type: 'string', value: '' };
          continue;
        }
        if (value === '') continue;
        const choices = inputChoices(input);
        const selected = choices ? choices[Number(value)] : value;
        if (input.type === 'boolean' && typeof selected === 'boolean') {
          inputs[input.id] = { type: 'boolean', value: selected };
        } else if (input.type !== 'boolean' && typeof selected === 'string') {
          if (input.type === 'integer' && !/^-?\d+$/.test(selected)) {
            throw new ApiError('Enter a whole number.');
          }
          inputs[input.id] = { type: input.type, value: selected };
        }
      }
      let saved = readSession<Pending>(stash);
      if (saved && JSON.stringify(saved.inputs) !== JSON.stringify(inputs)) {
        throw new ApiError('Restore the pending submission values before retrying.');
      }
      if (!saved) {
        saved = { key: (await control('issue_key', {})).key, inputs };
        sessionStorage.setItem(stash, JSON.stringify(saved));
      }
      const run = await api('submit', { key: saved.key, task_id: id, inputs: saved.inputs });
      sessionStorage.removeItem(stash);
      openRun(run.run_id);
    } catch (error) {
      if (error instanceof ApiError && error.serviceError && finalErrors.includes(error.message)) {
        sessionStorage.removeItem(stash);
      }
      pending = sessionStorage.getItem(stash) !== null;
      actions.showError(error);
    } finally {
      busy = false;
    }
  }
</script>

{#if task}
  <h1>{task.label}</h1>
  <form class="panel" onsubmit={submit}>
    {#each fields as field (field.input.id)}
      <TaskInput input={field.input} bind:value={field.value} bind:empty={field.empty} />
    {/each}
    {#if pending}
      <p class="muted">Retry the pending submission with the same values.</p>
    {/if}
    <button type="submit" class="primary" disabled={busy}>{pending ? 'Retry submission' : 'Run task'}</button>
  </form>
{/if}
