import { test, expect, type Page } from '@playwright/test';
import { spawn, type ChildProcess } from 'child_process';
import * as path from 'path';
import { fileURLToPath } from 'url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// SFU video rendering e2e — requires: Docker server (9800) + your own clean SFU producer (host-legacy).
// After the PIT-106 fix the browser consume path works end to end (candidates alias + vite ws proxy);
// the producer must be pure-SFU (host-legacy sfu_produce) — a multi-process gateway host would also relay P2P
// SDP/ICE, clashing with the browser SFU negotiation on one RTCPeerConnection (known limitation).

const REPO_ROOT = path.resolve(__dirname, '..', '..', '..', '..');
const HOST_BIN = path.join(REPO_ROOT, 'target', 'debug', 'host-legacy');

async function startCleanProducer(): Promise<ChildProcess> {
  const child = spawn(HOST_BIN, [], { cwd: REPO_ROOT, stdio: 'ignore' });
  return child;
}

async function waitForProducer(page: Page, room: string, timeoutMs = 30000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const rooms = await page.evaluate(async () => {
      const token = localStorage.getItem('mediaservo_admin_token');
      const res = await fetch('/api/admin/sfu/rooms', { headers: { Authorization: `Bearer ${token}` } });
      return res.ok ? res.json() : null;
    });
    const matched = rooms?.rooms?.find((r: { room_id: string }) => r.room_id === room);
    if (matched && matched.producers > 0) return;
    await page.waitForTimeout(2000);
  }
  throw new Error(`producer for room ${room} not ready within ${timeoutMs}ms`);
}

test.describe('SFU Video Rendering', () => {
  // own producer takes ~12s to start → widened per-test budget (default 30s is not enough)
  test.setTimeout(120_000);
  let producer: ChildProcess | null = null;

  test.beforeAll(async () => {
    // own clean SFU producer (test-room) — the room browser Play targets
    producer = await startCleanProducer();
  });

  test.afterAll(() => {
    producer?.kill();
    producer = null;
  });

  test.beforeEach(async ({ page }) => {
    // storageState (global-setup login) provides the admin token — PIT-103: a signed-in state is still required after auth.
    await page.goto('/');
  });

  test('navigates to dashboard and clicks Play to render video', async ({ page }) => {
    // Navigate to dashboard
    await expect(page.locator('.dashboard')).toBeVisible();
    // wait for the test-room SFU producer to be ready
    await waitForProducer(page, 'test-room');

    // Find and click the Play button for test-room
    const playButton = page.locator('.device-group', { hasText: 'test-room' }).locator('button.btn-play');
    await expect(playButton).toBeVisible();
    await playButton.click();

    // Wait for video element to appear
    const video = page.locator('video');
    await expect(video).toBeVisible({ timeout: 10000 });

    // Verify video readyState >= 2 (HAVE_CURRENT_DATA) — wait for real frame data to arrive
    await page.waitForFunction(() => {
      const v = document.querySelector('video');
      return v && v.readyState >= 2;
    }, { timeout: 15000 });
    const readyState = await video.evaluate((el: HTMLVideoElement) => el.readyState);
    expect(readyState).toBeGreaterThanOrEqual(2);
  });

  test('shows error state when connection fails', async ({ page }) => {
    // Navigate to dashboard
    await expect(page.locator('.dashboard')).toBeVisible();

    // Mock a failed connection by intercepting WebSocket
    await page.route('**/ws', (route) => {
      route.abort();
    });

    // Find and click the Play button
    const playButton = page.locator('.device-group', { hasText: 'test-room' }).locator('button.btn-play');
    await expect(playButton).toBeVisible();
    await playButton.click();

    // Wait for VideoPlayer error state (Signal Lost) — WS was aborted → connect fails
    const signalLost = page.locator('.vp-status-msg.error');
    await expect(signalLost).toBeVisible({ timeout: 15000 });
    await expect(signalLost).toContainText(/signal lost|error|failed|disconnected/i);
  });

  test('video stream has correct dimensions', async ({ page }) => {
    // Navigate to dashboard
    await expect(page.locator('.dashboard')).toBeVisible();
    await waitForProducer(page, 'test-room');

    // Find and click the Play button for test-room
    const playButton = page.locator('.device-group', { hasText: 'test-room' }).locator('button.btn-play');
    await expect(playButton).toBeVisible();
    await playButton.click();

    // Wait for video element to appear and have dimensions
    const video = page.locator('video');
    await expect(video).toBeVisible({ timeout: 10000 });

    // Wait for video to have non-zero dimensions
    await page.waitForFunction(() => {
      const v = document.querySelector('video');
      return v && v.videoWidth > 0 && v.videoHeight > 0;
    }, { timeout: 15000 });

    // Verify dimensions
    const dimensions = await video.evaluate((el: HTMLVideoElement) => ({
      width: el.videoWidth,
      height: el.videoHeight,
    }));
    expect(dimensions.width).toBeGreaterThan(0);
    expect(dimensions.height).toBeGreaterThan(0);
  });
});
