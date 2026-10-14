import { chromium } from '@playwright/test';
import { login } from './helpers';
import * as fs from 'fs';
import * as path from 'path';

// H3: sign in once per suite (G3 login rate-limit 2req/s burst5 — per-test logins get 429) → reuse storageState.
// Prereq: Docker server (9800) + config/accounts.docker.yaml dev account.

export default async function globalSetup() {
  const authDir = path.join(process.cwd(), '.auth');
  fs.mkdirSync(authDir, { recursive: true });

  const browser = await chromium.launch();
  for (const creds of [
    { username: 'admin', password: 'admin123', role: 'admin', file: 'admin.json' },
    { username: 'dispatcher', password: 'dispatch123', role: 'dispatcher', file: 'dispatcher.json' },
    { username: 'operator', password: 'operator123', role: 'operator', file: 'operator.json' },
  ]) {
    const context = await browser.newContext();
    const page = await context.newPage();
    await login(page, creds);
    await context.storageState({ path: path.join(authDir, creds.file) });
    await context.close();
  }
  await browser.close();
}
