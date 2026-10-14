import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './e2e',
  timeout: 30000,
  globalSetup: './e2e/global-setup.ts',
  projects: [
    // Admin suite (admin token): existing dashboard/settings/sfu video + H3 admin view
    {
      name: 'admin',
      testIgnore: /roles-(dispatcher|operator)\.spec\.ts|login\.spec\.ts/,
      use: { baseURL: 'http://localhost:5173', storageState: '.auth/admin.json' },
    },
    // dispatcher suite (H3 role-aware read-only view)
    {
      name: 'dispatcher',
      testMatch: /roles-dispatcher\.spec\.ts/,
      use: { baseURL: 'http://localhost:5173', storageState: '.auth/dispatcher.json' },
    },
    // operator suite (I1 review: no monitoring nav + admin REST rejected)
    {
      name: 'operator',
      testMatch: /roles-operator\.spec\.ts/,
      use: { baseURL: 'http://localhost:5173', storageState: '.auth/operator.json' },
    },
    // anonymous suite (no storageState): login page + route guards + 401 self-heal
    {
      name: 'anon',
      testMatch: /login\.spec\.ts/,
      use: { baseURL: 'http://localhost:5173' },
    },
  ],
});
