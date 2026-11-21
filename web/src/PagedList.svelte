<script lang="ts" generics="Item, Cursor">
  import { onMount, type Snippet } from 'svelte';
  import ActionButton from './ActionButton.svelte';
  import type { Actions } from './view';

  let { load, cursor, row, actions, pageSize = 20, empty = 'No items.', more = 'Show more', initial }: {
    load: (after?: Cursor) => Promise<Item[]>;
    cursor: (item: Item) => Cursor;
    row: Snippet<[Item]>;
    actions: Actions;
    pageSize?: number;
    empty?: string;
    more?: string;
    initial?: Item[];
  } = $props();

  let items = $state<Item[]>([]);
  let loaded = $state(false);
  let next = $state<Item>();

  function append(page: Item[]) {
    items.push(...page);
    next = page.length === pageSize ? page.at(-1) : undefined;
    loaded = true;
  }

  onMount(() => {
    let active = true;
    if (initial) append(initial);
    else load().then(page => {
      if (active) append(page);
    }).catch(actions.showError);
    return () => { active = false; };
  });
</script>

<div>
  {#each items as item}
    {@render row(item)}
  {:else}
    {#if loaded}<p class="muted">{empty}</p>{/if}
  {/each}
</div>
{#if next !== undefined}
  {@const last = next}
  <div class="actions">
    <ActionButton {actions} action={async () => append(await load(cursor(last)))}>{more}</ActionButton>
  </div>
{/if}
