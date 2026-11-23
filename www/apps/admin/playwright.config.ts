import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './e2e',
  timeout: 30000,
  globalSetup: './e2e/global-setup.ts',
  projects: [
    // Admin suite (admin token): existing dashboard/settings/sfu video + H3 admin view
    {
      name: 'admin',
      // F4: deployment-dependent specs are excluded here (not just documented)
      testIgnore: /roles-(dispatcher|operator)\.spec\.ts|login\.spec\.ts|restart-heal\.spec\.ts|sfu\.spec\.ts|vehicle-.*\.spec\.ts/,
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
      // V0/N1/F3/F5: media scenarios live ONLY here -- fake mic + autoplay.
      // Default projects stay mic-less so the honest-degrade test keeps working.
      name: 'talk-media',
      testMatch: /talk-media\.spec\.ts/,
      use: {
        baseURL: 'http://localhost:5173',
        storageState: '.auth/admin.json',
        launchOptions: {
          args: [
            '--use-fake-device-for-media-stream',
            '--use-fake-ui-for-media-stream',
            '--autoplay-policy=no-user-gesture-required',
          ],
        },
      },
    },
    {
      name: 'anon',
      testMatch: /login\.spec\.ts/,
      use: { baseURL: 'http://localhost:5173' },
    },
  ],
});
