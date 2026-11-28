import { test as base, expect, type Page, type Request } from '@playwright/test';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { createInterface } from 'node:readline';
import type { ApiRequest, ApiResponse } from '../src/api';

const test = base.extend<{ consoleUrl: string }>({
  consoleUrl: async ({}, use) => {
    const binary = process.env.RUSTWARD_WEB_BINARY;
    if (!binary) throw new Error('Run browser tests with just web-test.');
    const daemon = spawn(binary, ['web-preview'], { stdio: ['ignore', 'pipe', 'inherit'] });
    const exited = once(daemon, 'exit');
    const lines = createInterface({input: daemon.stdout});
    let timeout: ReturnType<typeof setTimeout> | undefined;
    try {
      const url = await Promise.race([
        once(lines, 'line').then(([line]) => {
          const match = /^Rustward preview: (http:\/\/127\.0\.0\.1:\d+)$/.exec(line);
          if (!match) throw new Error(line);
          return match[1];
        }),
        exited.then(([code]) => { throw new Error(`Preview exited with status ${code}`); }),
        new Promise<never>((_, reject) => { timeout = setTimeout(() => reject(new Error('Preview startup timed out')), 5000); }),
      ]);
      clearTimeout(timeout);
      await use(url);
    } finally {
      clearTimeout(timeout);
      lines.close();
      daemon.kill('SIGTERM');
      const force = setTimeout(() => daemon.kill('SIGKILL'), 3500);
      await exited;
      clearTimeout(force);
    }
  },
});

async function task(page: Page, url: string, label: string) {
  await page.goto(url);
  await page.getByRole('link', {name: label, exact: true}).click();
  await page.getByLabel('name', {exact: true}).fill('sample');
}

function apiRequest(request: Request): ApiRequest {
  return request.postDataJSON();
}

test('a lost response and page reload reuse the same admission', async ({ page, consoleUrl }) => {
  await task(page, consoleUrl, 'Example task');
  await page.getByLabel('count', { exact: true }).clear();
  await page.getByLabel('mode', { exact: true }).selectOption('');
  let firstRun: string | undefined;
  await page.route('**/api', async route => {
    const data = apiRequest(route.request());
    if (data.operation !== 'submit' || firstRun) return route.continue();
    const response = await route.fetch();
    const reply: ApiResponse<{ run_id: string }> = await response.json();
    if ('error' in reply) throw new Error(reply.error);
    firstRun = reply.ok.run_id;
    await route.abort();
  });
  await page.getByRole('button', {name: 'Run task'}).click();
  await expect(page.getByRole('button', {name: 'Retry submission'})).toBeVisible();
  await page.reload();
  await expect(page.getByLabel('name', {exact: true})).toHaveValue('sample');
  await expect(page.getByLabel('count', { exact: true })).toHaveValue('');
  await expect(page.getByLabel('mode', { exact: true })).toHaveValue('');
  await page.getByRole('button', {name: 'Retry submission'}).click();
  await expect(page).toHaveURL(new RegExp(`#run/${firstRun}$`));
  await page.getByRole('link', {name: 'Runs', exact: true}).click();
  await expect(page.getByRole('link', {name: 'ordinary', exact: true})).toHaveCount(1);
});

test('the console reconnects to the same output and cancels the run', async ({ page, consoleUrl }) => {
  await task(page, consoleUrl, 'Long task');
  await page.getByRole('button', {name: 'Run task'}).click();
  await expect(page.getByLabel('stdout')).toContainText('Task started');
  await expect(page.getByLabel('stderr')).toContainText('Synthetic diagnostic');
  const run = page.url();
  await page.reload();
  await expect(page).toHaveURL(run);
  await expect(page.getByLabel('stdout')).toContainText('Task started');
  await page.getByRole('button', {name: 'Cancel run'}).click();
  await expect(page.getByText('Completed · Cancellation')).toBeVisible();
});

test('form validation and request protection reject invalid submissions', async ({ page, consoleUrl }) => {
  await task(page, consoleUrl, 'Example task');
  await page.getByLabel('name', {exact: true}).clear();
  await page.getByRole('button', {name: 'Run task'}).click();
  await expect(page).toHaveURL(/#task\/ordinary$/);
  const summary = page.getByRole('group', { name: 'Correct the task inputs' });
  await expect(summary).toBeFocused();
  await summary.getByRole('link', { name: 'name. Enter a value.' }).press('Enter');
  await expect(page.getByLabel('name', { exact: true })).toBeFocused();
  expect(await page.getByLabel('name', {exact: true}).evaluate((node: HTMLInputElement) => node.validity.valueMissing)).toBe(true);
  await page.getByLabel('name', {exact: true}).fill('sample');
  await page.route('**/api', route => {
    const data = apiRequest(route.request());
    if (data.operation === 'submit') data.inputs.mode = {type: 'string', value: 'unlisted'};
    return route.continue({postData: JSON.stringify(data)});
  });
  await page.getByRole('button', {name: 'Run task'}).click();
  await expect(summary.getByRole('link', { name: 'mode. Select one of the available values.' })).toBeVisible();
  await expect(page.getByLabel('mode', { exact: true })).toHaveAttribute('aria-invalid', 'true');
  await expect(page.getByLabel('name', { exact: true })).toHaveValue('sample');
  const rejected = await page.evaluate(async () => (await fetch('/api', {
    method: 'POST', headers: {'Content-Type': 'application/json'},
    body: JSON.stringify({operation: 'control', request: {command: 'pause'}}),
  })).status);
  expect(rejected).toBe(403);
  await page.getByRole('link', {name: 'Runs', exact: true}).click();
  await expect(page.getByText('No runs yet.')).toBeVisible();
});

test('output is text and acknowledging a review releases the successor', async ({ page, consoleUrl }) => {
  await task(page, consoleUrl, 'Interrupted task');
  await page.getByRole('button', {name: 'Run task'}).click();
  await expect(page.getByLabel('stdout')).toContainText('<img src=x onerror=alert(1)>');
  await expect(page.getByLabel('stdout').locator('img')).toHaveCount(0);
  await expect(page.getByText('Completed · Interruption · Review required')).toBeVisible();
  await task(page, consoleUrl, 'Example task');
  await page.getByRole('button', {name: 'Run task'}).click();
  await expect(page.getByText('Queued', {exact: true})).toBeVisible();
  await expect(page.locator('#service')).toContainText('Review required');
  const successor = page.url();
  await page.getByRole('link', {name: 'Review', exact: true}).click();
  await page.getByLabel('Checks, remaining uncertainty, and reason to resume').fill('Synthetic cleanup checked. Resume the preview.');
  await page.getByRole('button', {name: 'Acknowledge review'}).click();
  await expect(page.getByText('No pending reviews.')).toBeVisible();
  await page.goto(successor);
  await expect(page.getByText('Completed · Success')).toBeVisible();
});

test('schedule and dispatch pauses are independent', async ({ page, consoleUrl }) => {
  await page.goto(consoleUrl);
  await page.getByRole('link', {name: 'Schedules', exact: true}).click();
  await page.getByRole('button', {name: 'Resume all schedules'}).click();
  await expect(page.getByText(/Next: .* UTC/)).toBeVisible();
  await page.getByRole('link', {name: 'example', exact: true}).click();
  await expect(page.getByText(/First installation/)).toBeVisible();
  await page.getByRole('button', {name: 'Pause schedule', exact: true}).click();
  await expect(page.getByRole('button', {name: 'Resume schedule', exact: true})).toBeVisible();
  await expect(page.getByRole('button', {name: 'Pause dispatch'})).toBeVisible();
});

test('the form preserves signed 64-bit integer input', async ({ page, consoleUrl }) => {
  await task(page, consoleUrl, 'Example task');
  await page.getByLabel('count', {exact: true}).fill('9223372036854775807');
  const submitted = page.waitForRequest(request => request.url().endsWith('/api') && apiRequest(request).operation === 'submit');
  await page.getByRole('button', {name: 'Run task'}).click();
  const data = apiRequest(await submitted);
  if (data.operation !== 'submit') throw new Error('Expected a submission request.');
  expect(data.inputs.count).toEqual({type: 'integer', value: '9223372036854775807'});
  await expect(page.getByText('Completed · Success')).toBeVisible();
  await page.getByText('Submitted inputs', { exact: true }).click();
  await expect(page.locator('details pre')).toContainText('9223372036854775807');
});

test('completed output shows when capture was truncated', async ({ page, consoleUrl }) => {
  await task(page, consoleUrl, 'Truncated task');
  await page.getByRole('button', {name: 'Run task'}).click();
  await expect(page.getByText('Completed · Success')).toBeVisible();
  await expect(page.getByLabel('stdout').locator('..').getByText('Output truncated')).toBeVisible();
  await expect(page.getByLabel('stdout')).toContainText('Task started');
});

test('keyboard submission respects dispatch pause and the queued runs filter', async ({ page, consoleUrl }) => {
  await task(page, consoleUrl, 'Example task');
  await page.getByRole('button', { name: 'Pause dispatch' }).press('Enter');
  await expect(page.getByRole('button', { name: 'Resume dispatch' })).toBeVisible();
  await page.getByLabel('name', { exact: true }).press('Enter');
  await expect(page.getByText('Queued', { exact: true })).toBeVisible();
  const run = page.url();

  const runs = page.getByRole('link', { name: 'Runs', exact: true });
  await runs.press('Enter');
  await expect(runs).toHaveAttribute('aria-current', 'page');
  await page.getByRole('link', { name: 'Queued runs' }).press('Enter');
  await expect(page.getByRole('heading', { name: 'Queue', exact: true })).toBeVisible();
  await expect(page.getByRole('link', { name: 'ordinary', exact: true })).toHaveCount(1);
  await expect(page.getByText('Runtime: synthetic', { exact: true })).toBeVisible();

  for (const theme of ['light', 'dark']) {
    await page.getByLabel('Theme').selectOption(theme);
    await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
    await expect(page.getByText('Queued runs will wait.', { exact: false })).toBeVisible();
    const table = page.getByRole('table', { name: 'Queued runs' });
    await expect(table.getByText('Queued', { exact: true })).toBeVisible();
    await expect(table.getByText('Admitted', { exact: true })).toBeVisible();
    await expect(table.locator('time')).toContainText('UTC');
    await table.getByRole('link', { name: 'ordinary', exact: true }).press('Enter');
    await expect(page).toHaveURL(run);
    await page.getByText('Submitted inputs', { exact: true }).click();
    await expect(page.locator('details pre').getByText(/"name":\s*"sample"/)).toBeVisible();
    await page.goBack();
  }
  await page.reload();
  await expect(page.getByLabel('Theme')).toHaveValue('dark');

  await page.getByRole('button', { name: 'Resume dispatch' }).press('Enter');
  await page.goto(run);
  await expect(page.getByText('Completed · Success')).toBeVisible();
});

test('a failed output poll reports the error and retries', async ({ page, consoleUrl }) => {
  await task(page, consoleUrl, 'Long task');
  let failed = false;
  await page.route('**/api', async route => {
    if (apiRequest(route.request()).operation === 'output' && !failed) {
      failed = true;
      return route.fulfill({ json: { error: 'OutputUnavailable' } });
    }
    await route.continue();
  });
  await page.getByRole('button', { name: 'Run task' }).click();
  await expect(page.getByRole('alert')).toHaveText('OutputUnavailable');
  await expect(page.getByLabel('stdout')).toContainText('Task started');
  await page.getByRole('button', { name: 'Cancel run' }).click();
  await expect(page.getByRole('alert')).toBeEmpty();
  await expect(page.getByText('Completed · Cancellation')).toBeVisible();
});

test('the footer shows the version reported by the service', async ({ page, consoleUrl }) => {
  await page.goto(consoleUrl);
  const version = await page.evaluate(async () => {
    const response = await fetch('/api', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'X-Rustward-CSRF': '1' },
      body: JSON.stringify({ operation: 'control', request: { command: 'service_status' } }),
    });
    const result = await response.json() as ApiResponse<{ version: string }>;
    if ('error' in result) throw new Error(result.error);
    return result.ok.version;
  });
  expect(version).toMatch(/^\d+\.\d+\.\d+-/);
  const footer = page.getByRole('contentinfo');
  await expect(footer).toHaveText(`Version ${version}`);
  await expect(page.getByRole('banner')).not.toContainText(`Version ${version}`);
  for (const theme of ['light', 'dark']) {
    await page.getByLabel('Theme').selectOption(theme);
    await expect(footer).toBeVisible();
  }
});

test('the theme selector aligns with the right edge of the header content', async ({ page, consoleUrl }) => {
  await page.goto(consoleUrl);
  const theme = page.getByLabel('Theme');
  for (const width of [1280, 320]) {
    await page.setViewportSize({ width, height: 800 });
    const rightGap = await theme.evaluate(select => {
      const header = select.closest('header')!;
      return header.getBoundingClientRect().right - select.getBoundingClientRect().right -
        parseFloat(getComputedStyle(header).paddingRight);
    });
    expect(Math.abs(rightGap)).toBeLessThan(1);
  }
});

for (const theme of ['light', 'dark'] as const) {
  test(`all console views fit a narrow screen in ${theme} mode`, async ({ page, consoleUrl }) => {
    await page.setViewportSize({ width: 320, height: 800 });
    await page.emulateMedia({ colorScheme: theme });
    await page.goto(consoleUrl);
    await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
    await page.keyboard.press('Tab');
    await expect(page.getByRole('link', { name: 'Skip to content' })).toBeFocused();
    await page.keyboard.press('Enter');
    await expect(page.getByRole('main')).toBeFocused();

    async function checkView(heading: string) {
      await expect(page.getByRole('heading', { level: 1, name: heading, exact: true })).toBeVisible();
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(320);
    }

    await checkView('Tasks');
    await page.getByRole('link', { name: 'Interrupted task', exact: true }).press('Enter');
    await checkView('Interrupted task');
    await page.getByLabel('name', { exact: true }).fill('screen review');
    await page.getByRole('button', { name: 'Run task', exact: true }).press('Enter');
    await expect(page.getByText('Completed · Interruption · Review required')).toBeVisible();
    await checkView('Run details');
    await expect(page.getByRole('region', { name: 'stdout' })).toBeVisible();

    await page.getByRole('link', { name: 'Runs', exact: true }).press('Enter');
    await expect(page.getByRole('table', { name: 'Run history' }).getByText('Review required')).toBeVisible();
    await checkView('Runs');
    await page.getByRole('link', { name: 'Review', exact: true }).press('Enter');
    await checkView('Review');
    await page.getByRole('button', { name: 'Acknowledge review' }).press('Enter');
    await expect(page.getByText('Enter a review note.')).toBeVisible();
    await expect(page.getByRole('textbox')).toBeFocused();
    await page.getByRole('textbox').fill('Reviewed the synthetic effect.');
    await page.getByRole('button', { name: 'Acknowledge review' }).press('Enter');
    await expect(page.getByText('Review acknowledged.', { exact: true })).toBeVisible();

    await page.getByRole('link', { name: 'Schedules', exact: true }).press('Enter');
    await expect(page.getByRole('table', { name: 'Schedules' })).toBeVisible();
    await checkView('Schedules');
    await page.getByRole('link', { name: 'example', exact: true }).press('Enter');
    await checkView('example');
    await expect(page.getByRole('button', { name: 'Pause schedule', exact: true }))
      .toHaveAccessibleDescription(/Queued runs can still start/);
    await expect(page.getByRole('button', { name: 'Pause dispatch', exact: true }))
      .toHaveAccessibleDescription(/Active runs continue/);
  });
}
