<script lang="ts">
  import { inputChoices, type Input } from './api';

  let { input, value = $bindable(''), empty = $bindable(false), error }: {
    input: Input;
    value?: string;
    empty?: boolean;
    error?: string;
  } = $props();
  const id = $derived('input-' + input.id);
  const required = $derived(input.required && input.default === null);
  const options = $derived(inputChoices(input));
</script>

<div class="field">
  <div>
    <label for={id}>{input.id}</label>
    {#if required}<span class="muted" aria-hidden="true"> (required)</span>{/if}
  </div>
  {#if error}<span id={id + "-error"} class="error-text">{error}</span>{/if}
  {#if options}
    <select {id} {required} bind:value aria-invalid={!!error} aria-describedby={error ? id + "-error" : undefined}>
      <option value="">{required ? 'Select a value' : input.default === null ? 'Leave unset' : 'Use default'}</option>
      {#each options as option, index}
        <option value={String(index)}>{String(option)}</option>
      {/each}
    </select>
  {:else}
    <input {id} type="text" inputmode={input.type === 'integer' ? 'numeric' : undefined}
      required={required && !empty} disabled={empty} bind:value
      aria-invalid={!!error} aria-describedby={error ? id + "-error" : undefined} />
    {#if input.type === 'string'}
      <label class="empty-choice">
        <input type="checkbox" bind:checked={empty} /> Use an empty string
      </label>
    {/if}
  {/if}
</div>
