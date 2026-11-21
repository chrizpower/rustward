<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { Actions } from './view';

  let { action, actions, children, disabled = false, primary = false }: {
    action: () => Promise<unknown>;
    actions: Actions;
    children: Snippet;
    disabled?: boolean;
    primary?: boolean;
  } = $props();
  let busy = $state(false);

  async function click() {
    actions.clearError();
    busy = true;
    try {
      await action();
    } catch (error) {
      actions.showError(error);
    } finally {
      busy = false;
    }
  }
</script>

<button type="button" class:primary disabled={busy || disabled} onclick={click}>
  {@render children()}
</button>
