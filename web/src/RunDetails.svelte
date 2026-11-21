<script lang="ts">
  import { onMount } from 'svelte';
  import { api, control, type StreamName } from './api';
  import ActionButton from './ActionButton.svelte';
  import type { Actions } from './view';

  let { id, actions }: { id: string; actions: Actions } = $props();
  const names: StreamName[] = ['stdout', 'stderr'];
  const limit = 262144;
  const stream = () => ({ offset: 0, decoder: new TextDecoder(), text: '', info: '' });
  let streams = $state({ stdout: stream(), stderr: stream() });
  let phase = $state('Loading run');
  let canCancel = $state(false);

  onMount(() => {
    let active = true;
    let timer: ReturnType<typeof setTimeout>;
    async function poll() {
      try {
        const run = await api('output', {
          run_id: id, stdout: streams.stdout.offset, stderr: streams.stderr.offset,
        });
        if (!active) return;
        const completed = run.state.phase === 'Completed';
        phase = [run.state.phase, run.state.result, run.record?.review_required ? 'Review required' : null]
          .filter(Boolean).join(' · ');
        canCancel = !completed;
        let more = false;
        for (const name of names) {
          const output = streams[name];
          const chunk = run.streams[name];
          const evidence = run.record?.output_evidence?.[name];
          if (!chunk) {
            streams[name] = { ...stream(), info: evidence?.retained_bytes ? 'Output reclaimed' : 'No output available' };
            continue;
          }
          output.offset = chunk.next;
          more ||= chunk.more;
          const text = output.decoder.decode(new Uint8Array(chunk.bytes), { stream: !completed || chunk.more });
          const combined = output.text + text;
          output.text = combined.slice(-limit);
          output.info = [
            evidence?.truncated ? 'Output truncated' : null,
            combined.length > limit || output.offset > output.text.length + limit ? 'Showing recent output' : null,
          ].filter(Boolean).join(' · ');
        }
        timer = setTimeout(poll, more ? 40 : completed ? 3000 : 700);
      } catch (error) {
        if (!active) return;
        actions.showError(error);
        timer = setTimeout(poll, 1500);
      }
    }
    void poll();
    return () => { active = false; clearTimeout(timer); };
  });
</script>

<h1>Run details</h1>
<p class="muted">{id}</p>
<p class="status" aria-live="polite">{phase}</p>
<div class="actions">
  {#if canCancel}
    <ActionButton {actions} action={() => control('cancel', { run_id: id })}>Cancel run</ActionButton>
  {/if}
</div>
<div class="output">
  {#each names as name}
    <div>
      <h2>{name}</h2>
      <p class="muted">{streams[name].info}</p>
      <pre aria-label={name}>{streams[name].text}</pre>
    </div>
  {/each}
</div>
