import { defineConfig } from '@playwright/test';

// P3 monitor — independent e2e project + independent baseline dir (G6), separate
// from the admin suite. Prereq: source server (9800, /tmp/ms-e2e/server.yaml) +
// vite dev (5176). Login is rate-limited (G3 2req/s burst5) → globalSetup produces
// storageState once; per-test logins only in the anon guard suite (with retry).
export default defineConfig({
  testDir: './e2e',
  timeout: 30000,
  globalSetup: './e2e/global-setup.ts',
  projects: [
    // anonymous: login page + route guards + token self-heal (no storageState)
    {
      name: 'monitor-anon',
      testMatch: /guard\.spec\.ts/,
      use: { baseURL: 'http://localhost:5176' },
    },
    // authenticated shell smoke + single-stream wall (admin storageState from globalSetup)
    {
      name: 'monitor-admin',
      testMatch: /(shell|wall|timeline)\.spec\.ts/,
      use: { baseURL: 'http://localhost:5176', storageState: '.auth/admin.json' },
    },
  ],
});