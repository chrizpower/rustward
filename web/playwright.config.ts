import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './tests',
  outputDir: process.env.RUSTWARD_WEB_ARTIFACTS,
  timeout: 60_000,
  use: { browserName: 'chromium', screenshot: 'only-on-failure', trace: 'retain-on-failure' },
});
