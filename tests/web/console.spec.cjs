const { test: base, expect } = require('@playwright/test');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const readline = require('node:readline');

const test = base.extend({
  consoleUrl: async ({}, use) => {
    const daemon = spawn(process.env.RUSTWARD_WEB_BINARY, ['web-preview'], { stdio: ['ignore', 'pipe', 'inherit'] });
    const exited = once(daemon, 'exit');
    const lines = readline.createInterface({input: daemon.stdout});
    let timeout;
    try {
      const url = await Promise.race([
        once(lines, 'line').then(([line]) => {
          const match = /^Rustward preview: (http:\/\/127\.0\.0\.1:\d+)$/.exec(line);
          if (!match) throw new Error(line);
          return match[1];
        }),
        exited.then(([code]) => { throw new Error(`Preview exited with status ${code}`); }),
        new Promise((_, reject) => { timeout = setTimeout(() => reject(new Error('Preview startup timed out')), 5000); }),
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

async function task(page, url, label) {
  await page.goto(url);
  await page.getByRole('link', {name: label, exact: true}).click();
  await page.getByLabel('name', {exact: true}).fill('sample');
}

test('a lost response and page reload reuse the same admission', async ({ page, consoleUrl }) => {
  await task(page, consoleUrl, 'Example task');
  let firstRun;
  await page.route('**/api', async route => {
    const data = route.request().postDataJSON();
    if (data.operation !== 'submit' || firstRun) return route.continue();
    const response = await route.fetch();
    firstRun = (await response.json()).ok.run_id;
    await route.abort();
  });
  await page.getByRole('button', {name: 'Run task'}).click();
  await expect(page.getByRole('button', {name: 'Retry submission'})).toBeVisible();
  await page.reload();
  await expect(page.getByLabel('name', {exact: true})).toHaveValue('sample');
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
  expect(await page.getByLabel('name', {exact: true}).evaluate(node => node.validity.valueMissing)).toBe(true);
  await page.getByLabel('name', {exact: true}).fill('sample');
  await page.route('**/api', route => {
    const data = route.request().postDataJSON();
    if (data.operation === 'submit') data.inputs.mode = {type: 'string', value: 'unlisted'};
    return route.continue({postData: JSON.stringify(data)});
  });
  await page.getByRole('button', {name: 'Run task'}).click();
  await expect(page.getByRole('alert')).toContainText('do not match');
  const rejected = await page.evaluate(async () => (await fetch('/api', {
    method: 'POST', headers: {'Content-Type': 'application/json'},
    body: JSON.stringify({operation: 'control', request: {command: 'pause'}}),
  })).status);
  expect(rejected).toBe(403);
  await page.getByRole('link', {name: 'Runs', exact: true}).click();
  await expect(page.getByText('No items.')).toBeVisible();
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
  const submitted = page.waitForRequest(request => request.url().endsWith('/api') && request.postDataJSON().operation === 'submit');
  await page.getByRole('button', {name: 'Run task'}).click();
  expect((await submitted).postDataJSON().inputs.count).toEqual({type: 'integer', value: '9223372036854775807'});
  await expect(page.getByText('Completed · Success')).toBeVisible();
});

test('completed output shows when capture was truncated', async ({ page, consoleUrl }) => {
  await task(page, consoleUrl, 'Truncated task');
  await page.getByRole('button', {name: 'Run task'}).click();
  await expect(page.getByText('Completed · Success')).toBeVisible();
  await expect(page.getByText('Output truncated')).toBeVisible();
  await expect(page.getByLabel('stdout')).toContainText('Task started');
});
