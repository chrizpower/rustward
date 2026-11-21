<script lang="ts">
  import { inputChoices, type Input } from './api';

  let { input, value = $bindable(''), empty = $bindable(false) }: {
    input: Input;
    value?: string;
    empty?: boolean;
  } = $props();
  const id = $derived('input-' + input.id);
  const required = $derived(input.required && input.default === null);
  const options = $derived(inputChoices(input));
</script>

<div class="field">
  <label for={id}>{input.id}</label>
  {#if options}
    <select {id} {required} bind:value>
      <option value="">{required ? 'Select a value' : 'Use default'}</option>
      {#each options as option, index}
        <option value={String(index)}>{String(option)}</option>
      {/each}
    </select>
  {:else}
    <input {id} type="text" inputmode={input.type === 'integer' ? 'numeric' : undefined}
      required={required && !empty} disabled={empty} bind:value />
    {#if input.type === 'string'}
      <label class="empty-choice">
        <input type="checkbox" bind:checked={empty} /> Use an empty string
      </label>
    {/if}
  {/if}
</div>
