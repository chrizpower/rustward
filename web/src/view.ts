const errorMessages: ReadonlyMap<string, string> = new Map([
  ['InvalidInputs', 'The submitted values do not match the task input schema.'],
  ['PayloadConflict', 'This request key belongs to a different submission.'],
  ['QueueFull', 'The queue is full.'],
  ['Closed', 'Admission is closed.'],
  ['ReviewChanged', 'The review changed. Open it again before acknowledging.'],
  ['NotCompleted', 'Local cleanup must finish before acknowledgement.'],
]);

export interface Actions {
  clearError(): void;
  showError(error: unknown): void;
}

export function errorMessage(error: unknown) {
  const message = error instanceof Error ? error.message : String(error);
  return errorMessages.get(message) ?? message;
}

export function utc(ms: number | null) {
  return ms === null ? 'Unknown' : new Date(ms).toISOString().replace('.000Z', ' UTC');
}

export function label(value: string) {
  return value.replaceAll('_', ' ').replace(/^[a-z]/, char => char.toUpperCase());
}

export function href(section: string, id?: string) {
  return '#' + section + (id === undefined ? '' : '/' + encodeURIComponent(id));
}

export function readSession<T>(key: string): T | null {
  return JSON.parse(sessionStorage.getItem(key) ?? 'null');
}
