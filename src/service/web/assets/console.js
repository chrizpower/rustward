'use strict';

const content = document.querySelector('#content');
const errorBox = document.querySelector('#error');
const serviceBox = document.querySelector('#service');
let viewVersion = 0;
let timer;
let lastService;

class ApiError extends Error {
  constructor(message, serviceError = false) {
    super(message);
    this.serviceError = serviceError;
  }
}

async function api(operation, fields = {}) {
  const response = await fetch('/api', {
    method: 'POST',
    headers: {'Content-Type': 'application/json', 'X-Rustward-CSRF': '1'},
    body: JSON.stringify({operation, ...fields}),
  });
  if (response.headers.get('content-type')?.startsWith('application/json')) {
    const value = await response.json();
    if (Object.hasOwn(value, 'error')) throw new ApiError(value.error, true);
    return value.ok;
  }
  throw new ApiError(await response.text() || `Request failed (${response.status})`);
}

const control = (command, fields = {}) => api('control', {request: {command, ...fields}});
const utc = ms => ms == null ? 'Unknown' : new Date(ms).toISOString().replace('.000Z', ' UTC');
const label = value => String(value).replaceAll('_', ' ').replace(/^[a-z]/, c => c.toUpperCase());
const href = (section, id) => '#' + section + (id === undefined ? '' : '/' + encodeURIComponent(id));

function element(tag, text, css) {
  const result = document.createElement(tag);
  if (text !== undefined) result.textContent = text;
  if (css) result.className = css;
  return result;
}

function link(text, target) {
  const node = element('a', text);
  node.href = target;
  return node;
}

function showError(error) {
  const messages = {InvalidInputs: 'The submitted values do not match the task input schema.',
    PayloadConflict: 'This request key belongs to a different submission.',
    QueueFull: 'The queue is full.', Closed: 'Admission is closed.',
    ReviewChanged: 'The review changed. Open it again before acknowledging.',
    NotCompleted: 'Local cleanup must finish before acknowledgement.'};
  errorBox.textContent = messages[error.message] || error.message;
  errorBox.hidden = false;
}

function button(text, action, css) {
  const node = element('button', text, css);
  node.type = 'button';
  node.addEventListener('click', async () => {
    errorBox.hidden = true;
    node.disabled = true;
    try { await action(); } catch (error) { showError(error); }
    finally { node.disabled = false; }
  });
  return node;
}

function heading(text) { content.append(element('h1', text)); }

async function serviceStatus() {
  const status = await control('service_status');
  const value = JSON.stringify(status);
  if (value === lastService) return;
  lastService = value;
  const message = status.failed ? 'Service failed' : status.draining ? 'Draining' :
    status.paused ? 'Dispatch paused' : status.dispatch_block === 'ReviewRequired' ?
      'Review required' : status.dispatch_block === 'LogCapacity' ?
        'Log capacity blocked' : `${status.active_runs} active`;
  const toggle = button(status.paused ? 'Resume dispatch' : 'Pause dispatch', async () => {
    await control(status.paused ? 'resume' : 'pause');
    await serviceStatus();
  });
  toggle.disabled = status.failed || status.draining;
  serviceBox.replaceChildren(element('span', message), toggle);
}

async function list(operation, fields, key, row, current) {
  const area = element('div');
  const more = element('div', undefined, 'actions');
  content.append(area, more);
  async function page(cursor) {
    const items = await api(operation, {...fields, ...cursor});
    if (!current()) return;
    if (!items.length && !area.children.length) area.append(element('p', 'No items.', 'muted'));
    area.append(...items.map(row));
    more.replaceChildren();
    if (items.length === 20) {
      const last = items.at(-1);
      more.append(button('Show more', () => page(typeof last[key] === 'number' ? {before: last[key]} : {after: last[key]})));
    }
  }
  await page({});
}

function taskList(current) {
  heading('Tasks');
  return list('tasks', {}, 'id', task => {
    const row = element('div', undefined, 'entry');
    row.append(link(task.label, href('task', task.id)), element('span', task.id, 'muted'));
    return row;
  }, current);
}

function field(input, initial) {
  const group = element('div', undefined, 'field');
  const caption = element('label', input.id);
  let widget;
  const options = input.choices || (input.type === 'boolean' ? [true, false] : null);
  if (options) {
    widget = element('select');
    widget.append(new Option(input.required && input.default == null ? 'Select a value' : 'Use default', ''));
    options.forEach((value, index) => widget.append(new Option(String(value), String(index))));
    const index = options.indexOf(initial);
    if (index >= 0) widget.value = String(index);
  } else {
    widget = element('input');
    widget.type = 'text';
    if (input.type === 'integer') widget.inputMode = 'numeric';
    if (initial != null) widget.value = String(initial);
  }
  widget.id = 'input-' + input.id;
  caption.htmlFor = widget.id;
  const required = input.required && input.default == null;
  widget.required = required;
  group.append(caption, widget);
  let empty;
  if (input.type === 'string' && !options) {
    empty = element('input');
    empty.type = 'checkbox';
    empty.checked = initial === '';
    const choice = element('label', undefined, 'empty-choice');
    choice.append(empty, document.createTextNode(' Use an empty string'));
    group.append(choice);
    const update = () => {
      widget.disabled = empty.checked;
      widget.required = required && !empty.checked;
    };
    empty.addEventListener('change', update);
    update();
  }
  const read = () => {
    if (empty?.checked) return {type: 'string', value: ''};
    if (widget.value === '') return undefined;
    const value = options ? options[Number(widget.value)] : widget.value;
    if (input.type === 'integer' && !/^-?\d+$/.test(value)) throw new ApiError('Enter a whole number.');
    return {type: input.type, value};
  };
  return {group, read, id: input.id};
}

async function taskForm(id, current) {
  const task = await api('task', {task_id: id});
  if (!current()) return;
  heading(task.label);
  const stash = 'rustward-submit-' + task.id;
  const previous = JSON.parse(sessionStorage.getItem(stash) || 'null');
  const form = element('form', undefined, 'panel');
  const fields = task.inputs.map(input => field(input, previous?.inputs[input.id]?.value ?? input.default));
  form.append(...fields.map(input => input.group));
  if (previous) form.append(element('p', 'Retry the pending submission with the same values.', 'muted'));
  const submit = element('button', previous ? 'Retry submission' : 'Run task', 'primary');
  submit.type = 'submit';
  form.append(submit);
  form.addEventListener('submit', async event => {
    event.preventDefault();
    if (submit.disabled) return;
    submit.disabled = true;
    errorBox.hidden = true;
    try {
      const inputs = Object.fromEntries(fields.map(input => [input.id, input.read()]).filter(pair => pair[1] !== undefined));
      let pending = JSON.parse(sessionStorage.getItem(stash) || 'null');
      if (pending && JSON.stringify(pending.inputs) !== JSON.stringify(inputs)) {
        throw new ApiError('Restore the pending submission values before retrying.');
      }
      if (!pending) {
        pending = {key: (await control('issue_key')).key, inputs};
        sessionStorage.setItem(stash, JSON.stringify(pending));
      }
      const run = await api('submit', {key: pending.key, task_id: task.id, inputs: pending.inputs});
      sessionStorage.removeItem(stash);
      if (current()) location.hash = href('run', run.run_id);
    } catch (error) {
      if (error.serviceError && ['InvalidInputs', 'QueueFull', 'TooLarge', 'InvalidKey', 'PayloadConflict'].includes(error.message)) {
        sessionStorage.removeItem(stash);
      }
      submit.textContent = sessionStorage.getItem(stash) ? 'Retry submission' : 'Run task';
      if (current()) showError(error);
    } finally { submit.disabled = false; }
  });
  content.append(form);
}

function runList(current, queued) {
  heading(queued ? 'Queue' : 'Run history');
  const actions = element('div', undefined, 'actions');
  actions.append(link(queued ? 'All runs' : 'Queued runs', href('runs', queued ? 'all' : 'queued')));
  content.append(actions);
  return list('runs', {queued}, 'sequence', run => {
    const row = element('div', undefined, 'entry');
    row.append(link(run.task_id, href('run', run.run_id)),
      element('span', label(run.result || run.phase), 'status'),
      element('span', run.schedule_id ? `${run.schedule_id} · ${utc(run.due_ms)}` : 'Manual', 'muted'));
    return row;
  }, current);
}

async function runDetails(id, current) {
  heading('Run details');
  content.append(element('p', id, 'muted'));
  const phase = element('p', 'Loading run', 'status');
  phase.setAttribute('aria-live', 'polite');
  const cancel = button('Cancel run', async () => { await control('cancel', {run_id: id}); });
  const actions = element('div', undefined, 'actions');
  actions.append(cancel);
  cancel.hidden = true;
  const panes = element('div', undefined, 'output');
  const streams = Object.fromEntries(['stdout', 'stderr'].map(name => {
    const pane = element('div');
    const title = element('h2', name);
    const info = element('p', '', 'muted');
    const pre = element('pre');
    pre.setAttribute('aria-label', name);
    pane.append(title, info, pre);
    panes.append(pane);
    return [name, {info, pre, offset: 0, decoder: new TextDecoder()}];
  }));
  content.append(phase, actions, panes);
  const poll = async () => {
    try {
      const run = await api('output', {run_id: id, stdout: streams.stdout.offset, stderr: streams.stderr.offset});
      if (!current()) return;
      phase.textContent = [run.state.phase, run.state.result, run.record?.review_required ? 'Review required' : null].filter(Boolean).join(' · ');
      cancel.hidden = run.state.phase === 'Completed';
      let more = false;
      for (const [name, stream] of Object.entries(streams)) {
        const chunk = run.streams[name];
        const evidence = run.record?.output_evidence?.[name];
        if (!chunk) {
          stream.pre.textContent = '';
          stream.offset = 0;
          stream.decoder = new TextDecoder();
          stream.info.textContent = evidence?.retained_bytes ? 'Output reclaimed' : 'No output available';
          continue;
        }
        stream.offset = chunk.next;
        more ||= chunk.more;
        const text = stream.decoder.decode(new Uint8Array(chunk.bytes), {stream: run.state.phase !== 'Completed' || chunk.more});
        const combined = stream.pre.textContent + text;
        stream.pre.textContent = combined.slice(-262144);
        stream.info.textContent = [
          evidence?.truncated ? 'Output truncated' : null,
          combined.length > 262144 || stream.offset > stream.pre.textContent.length + 262144 ? 'Showing recent output' : null,
        ].filter(Boolean).join(' · ');
      }
      if (run.state.phase !== 'Completed' || more) timer = setTimeout(poll, more ? 40 : 700);
      else timer = setTimeout(poll, 3000);
    } catch (error) {
      if (current()) {
        showError(error);
        timer = setTimeout(poll, 1500);
      }
    }
  };
  await poll();
}

function scheduleList(current) {
  heading('Schedules');
  const actions = element('div', undefined, 'actions');
  for (const [text, command] of [['Pause all schedules', 'schedule_pause'], ['Resume all schedules', 'schedule_resume']]) {
    actions.append(button(text, async () => { await control(command); await render(); }));
  }
  content.append(actions);
  return list('schedules', {}, 'id', schedule => {
    const row = element('div', undefined, 'entry');
    row.append(link(schedule.id, href('schedule', schedule.id)), element('span', schedule.cron + ' UTC', 'muted'),
      element('span', schedule.paused ? 'Paused' : 'Enabled', 'status'),
      element('span', `Last: ${utc(schedule.last_ms)}`, 'muted'),
      element('span', `Next: ${utc(schedule.next_ms)}`));
    return row;
  }, current);
}

async function scheduleDetails(id, current) {
  const initial = await control('schedule_history', {schedule_id: id});
  if (!current()) return;
  heading(id);
  const actions = element('div', undefined, 'actions');
  actions.append(button(initial.paused ? 'Resume schedule' : 'Pause schedule', async () => {
    await control(initial.paused ? 'schedule_resume' : 'schedule_pause', {schedule_id: id});
    await render();
  }));
  if (initial.global_paused) actions.append(element('span', 'All schedules paused'));
  content.append(actions, element('h2', 'Occurrences'));
  const occurrences = element('div');
  const moreOccurrences = element('div', undefined, 'actions');
  content.append(occurrences, moreOccurrences, element('h2', 'Schedule gaps'));
  const gaps = element('div');
  const moreGaps = element('div', undefined, 'actions');
  content.append(gaps, moreGaps);
  const pages = {
    occurrences: {area: occurrences, more: moreOccurrences, cursor: 'after_due_ms', key: 'due_ms',
      draw: occurrence => {
        const row = element('div', undefined, 'entry');
        row.append(element('span', utc(occurrence.due_ms)), element('span', label(occurrence.decision)));
        if (occurrence.run_id) row.append(link('View run', href('run', occurrence.run_id)));
        return row;
      }},
    gaps: {area: gaps, more: moreGaps, cursor: 'after_gap_id', key: 'id',
      draw: gap => element('div', `${utc(gap.after_ms)} to ${utc(gap.through_ms)} · ${label(gap.reason)} · Missed: ${gap.missed_count ?? 'unknown'}`, 'entry')},
  };
  function page(name, items) {
    const target = pages[name];
    if (!items.length && !target.area.children.length) target.area.append(element('p', 'No items.', 'muted'));
    target.area.append(...items.map(target.draw));
    target.more.replaceChildren();
    if (items.length === 20) target.more.append(button('Show more ' + name, async () => {
      const data = await control('schedule_history', {schedule_id: id, [target.cursor]: items.at(-1)[target.key]});
      if (current()) page(name, data[name]);
    }));
  }
  page('occurrences', initial.occurrences);
  page('gaps', initial.gaps);
}

async function reviewList(current) {
  heading('Review');
  const area = element('div');
  const more = element('div', undefined, 'actions');
  content.append(area, more);
  async function page(after) {
    const reviews = await control('reviews', {after});
    if (!current()) return;
    if (!reviews.length && !area.children.length) area.append(element('p', 'No pending reviews.', 'muted'));
    for (const review of reviews) {
      const row = element('div', undefined, 'panel');
      row.append(element('h2', review.task_id), link('View interrupted run', href('run', review.run_id)),
        element('p', `${review.state.phase} · ${review.state.result || 'Pending'}`),
        element('p', `Runtime: ${review.runtime_id}`, 'muted'));
      const details = element('details');
      details.append(element('summary', 'Review evidence'),
        element('pre', JSON.stringify({launch: review.launch, times: review.times}, null, 2)));
      row.append(details);
      if (review.state.phase === 'Completed') {
        const note = element('textarea');
        const caption = element('label', 'Checks, remaining uncertainty, and reason to resume');
        note.id = 'review-' + review.run_id;
        caption.htmlFor = note.id;
        note.maxLength = 4096;
        const stash = 'rustward-review-' + review.run_id;
        const saved = JSON.parse(sessionStorage.getItem(stash) || 'null');
        if (saved?.version === review.evidence_version) note.value = saved.note;
        row.append(caption, note, button('Acknowledge review', async () => {
          if (!note.value.trim()) throw new ApiError('Enter a review note.');
          let pending = JSON.parse(sessionStorage.getItem(stash) || 'null');
          if (pending?.version !== review.evidence_version || pending.note !== note.value) {
            const id = Array.from(crypto.getRandomValues(new Uint8Array(32)), byte => byte.toString(16).padStart(2, '0')).join('');
            pending = {id, note: note.value, version: review.evidence_version};
            sessionStorage.setItem(stash, JSON.stringify(pending));
          }
          await control('acknowledge', {acknowledgement_id: pending.id, review, note: pending.note});
          sessionStorage.removeItem(stash);
          await serviceStatus();
          if (current()) await render();
        }, 'primary'));
      }
      area.append(row);
    }
    more.replaceChildren();
    if (reviews.length === 10) more.append(button('Show more reviews', () => page(reviews.at(-1).run_id)));
  }
  await page(null);
}

async function render() {
  clearTimeout(timer);
  const version = ++viewVersion;
  const current = () => version === viewVersion;
  content.replaceChildren();
  errorBox.hidden = true;
  try {
    const [section, id] = (location.hash.slice(1) || 'tasks').split('/').map(decodeURIComponent);
    const nav = {task: 'tasks', run: 'runs', schedule: 'schedules'}[section] || section;
    for (const item of document.querySelectorAll('nav a')) {
      if (item.dataset.section === nav) item.setAttribute('aria-current', 'page');
      else item.removeAttribute('aria-current');
    }
    switch (section) {
      case 'task': await taskForm(id, current); break;
      case 'runs': await runList(current, id === 'queued'); break;
      case 'run': await runDetails(id, current); break;
      case 'schedules': await scheduleList(current); break;
      case 'schedule': await scheduleDetails(id, current); break;
      case 'reviews': await reviewList(current); break;
      default: await taskList(current);
    }
  } catch (error) {
    if (current()) showError(error);
  }
}

window.addEventListener('hashchange', render);
render();
async function refreshService() {
  try { await serviceStatus(); }
  catch { serviceBox.textContent = 'Service unavailable'; lastService = undefined; }
  finally { setTimeout(refreshService, 1200); }
}
refreshService();
