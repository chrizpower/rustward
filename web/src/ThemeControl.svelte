<script lang="ts">
  import { onMount } from 'svelte';

  type Theme = 'system' | 'light' | 'dark';
  const key = 'rustward-theme';
  const preference = matchMedia('(prefers-color-scheme: dark)');
  let theme = $state<Theme>('system');

  function apply() {
    document.documentElement.dataset.theme =
      theme === 'system' ? (preference.matches ? 'dark' : 'light') : theme;
  }

  onMount(() => {
    const saved = localStorage.getItem(key);
    if (saved === 'light' || saved === 'dark') theme = saved;
    apply();
    preference.addEventListener('change', apply);
    return () => preference.removeEventListener('change', apply);
  });
</script>

<label class="theme-control">
  Theme
  <select value={theme} onchange={event => {
    theme = event.currentTarget.value as Theme;
    localStorage.setItem(key, theme);
    apply();
  }}>
    <option value="system">System</option>
    <option value="light">Light</option>
    <option value="dark">Dark</option>
  </select>
</label>
