import { chromium } from '@playwright/test';
import { login } from './helpers';
import * as fs from 'fs';
import * as path from 'path';

// Sign in once per suite (G3 login rate-limit 2req/s burst5 — per-test logins get
// 429) → reuse storageState. Prereq: source server (9800) + /tmp/ms-e2e/accounts.yaml
// dev accounts (allow_dev_credentials).
export default async function globalSetup() {
  const authDir = path.join(process.cwd(), '.auth');
  fs.mkdirSync(authDir, { recursive: true });

  const browser = await chromium.launch();
  for (const creds of [
    { username: 'admin', password: 'admin123', role: 'admin', file: 'admin.json' },
    { username: 'dispatcher', password: 'dispatch123', role: 'dispatcher', file: 'dispatcher.json' },
  ]) {
    const context = await browser.newContext();
    const page = await context.newPage();
    await login(page, creds);
    await context.storageState({ path: path.join(authDir, creds.file) });
    await context.close();
  }
  await browser.close();
}