const { defineConfig } = require('@playwright/test');

module.exports = defineConfig({
  testDir: '.',
  testMatch: 'console.spec.cjs',
  outputDir: process.env.RUSTWARD_WEB_ARTIFACTS,
  timeout: 60_000,
  use: { browserName: 'chromium', screenshot: 'only-on-failure', trace: 'retain-on-failure' },
});
