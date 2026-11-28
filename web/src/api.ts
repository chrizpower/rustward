type InputSchema<T, V> = {
  id: string;
  type: T;
  required: boolean;
  default: V | null;
  choices: V[] | null;
};

export type Input = InputSchema<'string', string> | InputSchema<'integer', string> |
  InputSchema<'boolean', boolean>;
export function inputChoices(input: Input): (string | boolean)[] | null {
  return input.choices ?? (input.type === 'boolean' ? [true, false] : null);
}

export type FormValue = { type: 'string' | 'integer'; value: string } |
  { type: 'boolean'; value: boolean };
export type Inputs = Record<string, FormValue>;

export type TaskCard = { id: string; label: string };
export type Task = TaskCard & { inputs: Input[] };
type Phase = 'Queued' | 'Launching' | 'Running' | 'CleaningUp' | 'Completed';
type Result = 'Cancellation' | 'Success' | 'Failure' | 'Interruption' | 'Timeout';
type RunState = { phase: Phase; result: Result | null };
export type RunRow = {
  sequence: number;
  run_id: string;
  task_id: string;
  phase: 'queued' | 'launching' | 'running' | 'cleaning_up' | 'completed';
  result: Lowercase<Result> | null;
  schedule_id: string | null;
  due_ms: number | null;
  admitted_ms: number | null;
  dispatched_ms: number | null;
  completed_ms: number | null;
  review_required: boolean;
  runtime_id: string;
};
export type Admission = {
  status: 'admitted' | 'already_admitted' | 'history_removed';
  run_id: string;
};

export type StreamName = 'stdout' | 'stderr';
type StreamEvidence = {
  retained_bytes: number;
  observed_bytes: number | null;
  truncated: boolean | null;
};
type Launch = {
  start_state: string;
  identity: { invocation: number[]; control_group: string } | null;
  exit: { Code: number } | { Signal: number } | null;
  cleanup_confirmed: boolean;
  review_required: boolean;
  log_bytes: number;
  stop_reason: string | null;
  output_failed: boolean;
  launch_failure?: string;
  capture_setup_failure?: string;
  output_evidence?: Record<StreamName, StreamEvidence> & {
    synchronized: boolean;
    recovered: boolean;
  };
};
type Chunk = { next: number; bytes: number[]; more: boolean };
type Output = {
  submitted_inputs: string | null;
  state: RunState;
  record: Launch | null;
  streams: Record<StreamName, Chunk | null>;
};

export type Schedule = {
  id: string;
  task_id: string;
  cron: string;
  paused: boolean;
  last_ms: number | null;
  next_ms: number | null;
};
export type Occurrence = { due_ms: number; revision: string; decision: string; run_id: string | null };
export type Gap = { id: number; after_ms: number; through_ms: number; reason: string; missed_count: number | null };
export type ScheduleHistory = {
  active: boolean;
  paused: boolean;
  global_paused: boolean;
  checkpoint_ms: number;
  occurrences: Occurrence[];
  gaps: Gap[];
};

export type Review = {
  run_id: string;
  evidence_version: number;
  task_id: string;
  runtime_id: string;
  task_revision: number[];
  config_revision: number[];
  state: RunState;
  launch: Launch;
  times: { dispatched_at_ms: number | null; completed_at_ms: number | null };
};
type Acknowledgement = {
  acknowledgement_id: string;
  review: Review;
  note: string;
  acknowledged_at_ms: number;
};
export type ServiceStatus = {
  version: string;
  paused: boolean;
  draining: boolean;
  failed: boolean;
  active_runs: number;
  dispatch_block: string | null;
};

type Operations = {
  tasks: [{ after?: string }, TaskCard[]];
  task: [{ task_id: string }, Task];
  submit: [{ key: string; task_id: string; inputs: Inputs }, Admission];
  runs: [{ queued: boolean; before?: number }, RunRow[]];
  schedules: [{ after?: string }, Schedule[]];
  output: [{ run_id: string; include_inputs?: boolean } & Record<StreamName, number>, Output];
};
type Commands = {
  issue_key: [Record<never, never>, { key: string }];
  service_status: [Record<never, never>, ServiceStatus];
  pause: [Record<never, never>, null];
  resume: [Record<never, never>, null];
  cancel: [{ run_id: string }, { status: string }];
  reviews: [{ after: string | null }, Review[]];
  acknowledge: [{ acknowledgement_id: string; review: Review; note: string }, Acknowledgement];
  schedule_pause: [{ schedule_id?: string }, null];
  schedule_resume: [{ schedule_id?: string }, null];
  schedule_history: [{ schedule_id: string; after_due_ms?: number; after_gap_id?: number }, ScheduleHistory];
};
export type ApiRequest = {
  [K in keyof Operations]: { operation: K } & Operations[K][0];
}[keyof Operations] | {
  operation: 'control';
  request: { [K in keyof Commands]: { command: K } & Commands[K][0] }[keyof Commands];
};
export type ApiResponse<T> = { ok: T } | { error: string; fields?: Record<string, string> };

export class ApiError extends Error {
  constructor(message: string, readonly serviceError = false, readonly fields?: Record<string, string>) {
    super(message);
  }
}

async function request<T>(body: object): Promise<T> {
  const response = await fetch('/api', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', 'X-Rustward-CSRF': '1' },
    body: JSON.stringify(body),
  });
  if (response.headers.get('content-type')?.startsWith('application/json')) {
    const value: ApiResponse<T> = await response.json();
    if ('error' in value) throw new ApiError(value.error, true, value.fields);
    return value.ok;
  }
  throw new ApiError(await response.text() || `Request failed (${response.status})`);
}

export function api<K extends keyof Operations>(operation: K, fields: Operations[K][0]) {
  return request<Operations[K][1]>({ operation, ...fields });
}

export function control<K extends keyof Commands>(command: K, fields: Commands[K][0]) {
  return request<Commands[K][1]>({ operation: 'control', request: { command, ...fields } });
}
