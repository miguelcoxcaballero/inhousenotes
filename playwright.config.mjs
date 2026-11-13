import { existsSync } from 'node:fs';
import { defineConfig, devices } from '@playwright/test';

// v6 smoke tests (docs/v6/TESTING.md §2). Run `npm run build` first: the server overlays build/ on the tree.
// IHN_E2E_PORT picks another port when several checkouts run tests on one machine; IHN_CHROMIUM points at an
// already installed Chromium (sandboxes where `playwright install` is not available).
const port = Number(process.env.IHN_E2E_PORT) || 4300;
const server = existsSync('tests/v6/server.mjs') ? 'tests/v6/server.mjs' : 'scripts/serve.mjs';

export default defineConfig({
  testDir: './tests/v6/e2e',
  testMatch: /.*\.spec\.mjs/,
  timeout: 30_000,
  expect: { timeout: 8_000 },
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  workers: 1,
  reporter: process.env.CI ? [['line'], ['html', { open: 'never' }]] : 'line',
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    launchOptions: process.env.IHN_CHROMIUM ? { executablePath: process.env.IHN_CHROMIUM } : {}
  },
  projects: [
    { name: 'chromium', use: { ...devices['Desktop Chrome'] } }
  ],
  webServer: {
    command: `node ${server} --root . --overlay build --port ${port}`,
    url: `http://127.0.0.1:${port}/`,
    reuseExistingServer: false,
    timeout: 20_000
  }
});
