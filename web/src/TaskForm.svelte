<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { api, ApiError, control, inputChoices, type Admission, type Input, type Inputs, type Task } from './api';
  import TaskInput from './TaskInput.svelte';
  import { readSession, type Actions } from './view';

  type Pending = { key: string; inputs: Inputs };
  type Field = { input: Input; value: string; empty: boolean };
  let { id, actions, openRun }: {
    id: string;
    actions: Actions;
    openRun: (run: Admission) => void;
  } = $props();
  let task = $state<Task>();
  let fields = $state<Field[]>([]);
  let pending = $state(false);
  let busy = $state(false);
  let errors = $state<{ id: string; message: string }[]>([]);
  let summary = $state<HTMLDivElement>();

  function validate({ input, value, empty }: Field) {
    if (empty) return '';
    const choices = inputChoices(input);
    if (!value) {
      if (input.required && input.default === null) return choices ? 'Select a value.' : 'Enter a value.';
    } else if (input.type === 'integer' && !choices) {
      if (!/^-?\d+$/.test(value)) return 'Enter a whole number.';
      const number = BigInt(value);
      if (number < -(2n ** 63n) || number >= 2n ** 63n) {
        return 'Enter a whole number from −9223372036854775808 to 9223372036854775807.';
      }
    }
    return '';
  }
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

  async function focusErrors() {
    await tick();
    summary?.focus();
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (busy || !task) return;
    actions.clearError();
    errors = fields.flatMap(field => {
      const message = validate(field);
      return message ? [{ id: field.input.id, message }] : [];
    });
    if (errors.length) {
      await focusErrors();
      return;
    }
    busy = true;
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
      openRun(run);
    } catch (error) {
      if (error instanceof ApiError && error.serviceError && finalErrors.includes(error.message)) {
        sessionStorage.removeItem(stash);
      }
      pending = sessionStorage.getItem(stash) !== null;
      if (error instanceof ApiError && error.fields) {
        const fieldErrors = error.fields;
        errors = fields.flatMap(({ input }) => Object.hasOwn(fieldErrors, input.id)
          ? [{ id: input.id, message: fieldErrors[input.id] }]
          : []);
        if (errors.length) await focusErrors();
      }
      if (!errors.length) actions.showError(error);
    } finally {
      busy = false;
    }
  }
</script>

{#if task}
  <h1>{task.label}</h1>
  <form class="panel" novalidate onsubmit={submit}>
    {#if errors.length}
      <div class="error-summary" role="group" aria-labelledby="input-error-heading" tabindex="-1" bind:this={summary}>
        <h2 id="input-error-heading">Correct the task inputs</h2>
        <ul>
          {#each errors as error}
            <li><a href={'#input-' + error.id} onclick={event => {
              event.preventDefault();
              document.getElementById('input-' + error.id)?.focus();
            }}>{error.id}. {error.message}</a></li>
          {/each}
        </ul>
      </div>
    {/if}
    {#each fields as field (field.input.id)}
      <TaskInput input={field.input} bind:value={field.value} bind:empty={field.empty} error={errors.find(error => error.id === field.input.id)?.message} />
    {/each}
    {#if pending}
      <p class="muted">Retry the pending submission with the same values.</p>
    {/if}
    <button type="submit" class="primary" disabled={busy}>{pending ? 'Retry submission' : 'Run task'}</button>
  </form>
{:else}
  <h1>Run task</h1>
{/if}
