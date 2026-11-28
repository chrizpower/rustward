<script lang="ts" generics="Item, Cursor">
  import { onMount, tick, type Snippet } from 'svelte';
  import ActionButton from './ActionButton.svelte';
  import type { Actions } from './view';

  let { load, cursor, row, list, actions, pageSize = 20, empty = 'No items.', more = 'Show more', initial }: {
    load: (after?: Cursor) => Promise<Item[]>;
    cursor: (item: Item) => Cursor;
    row?: Snippet<[Item]>;
    list?: Snippet<[Item[]]>;
    actions: Actions;
    pageSize?: number;
    empty?: string;
    more?: string;
    initial?: Item[];
  } = $props();

  let items = $state<Item[]>([]);
  let loaded = $state(false);
  let failed = $state(false);
  let feedback = $state('');
  let next = $state<Item>();

  function append(page: Item[]) {
    failed = false;
    items.push(...page);
    next = page.length === pageSize ? page.at(-1) : undefined;
    loaded = true;
  }

  onMount(() => {
    let active = true;
    if (initial) append(initial);
    else load().then(page => {
      if (active) append(page);
    }).catch(error => {
      if (!active) return;
      failed = true;
      actions.showError(error);
    });
    return () => { active = false; };
  });
</script>

<div>
  <span class="visually-hidden" role="status">{feedback}</span>
  {#if failed}
    <ActionButton {actions} action={async () => append(await load())}>Retry loading</ActionButton>
  {:else if !loaded}
    <p class="muted">Loading…</p>
  {:else if items.length === 0}
    <p class="muted">{empty}</p>
  {:else if list}
    {@render list(items)}
  {:else if row}
    {#each items as item}
      {@render row(item)}
    {/each}
  {/if}
</div>
{#if next !== undefined}
  {@const last = next}
  <div class="actions">
    <ActionButton {actions} action={async () => {
      const page = await load(cursor(last));
      append(page);
      feedback = page.length ? `${items.length} items shown.` : 'All items shown.';
      if (!next) await tick().then(() => document.getElementById('content')?.focus());
    }}>{more}</ActionButton>
  </div>
{/if}
