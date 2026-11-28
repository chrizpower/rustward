<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { Actions } from './view';

  let { action, actions, children, disabled = false, primary = false, destructive = false, describedBy }: {
    action: () => Promise<unknown>;
    actions: Actions;
    children: Snippet;
    disabled?: boolean;
    primary?: boolean;
    destructive?: boolean;
    describedBy?: string;
  } = $props();
  let busy = $state(false);

  async function click() {
    if (busy) return;
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

<button type="button" class:primary class:destructive disabled={disabled} aria-disabled={busy || undefined} aria-describedby={describedBy} onclick={click}>
  {@render children()}
</button>
