// Browser smoke tests (D14). Each project gets its own fluxo server on a fresh, empty data folder,
// so every run starts at first-time setup. The no-JavaScript project proves the app works as plain
// HTML forms; the others exercise the enhancements.
//
//   cargo build -p fluxo && (cd tests/browser && npx playwright test)
//
// FLUXO_BIN runs another build instead; CI tests the production binary that it then deploys.

import { defineConfig, devices } from '@playwright/test';
import { resolve } from 'node:path';

const bin = resolve(process.env.FLUXO_BIN || '../../target/debug/fluxo');

const servers = [
  { name: 'chromium-js', port: 18101, use: { ...devices['Desktop Chrome'] } },
  { name: 'chromium-no-js', port: 18102, use: { ...devices['Desktop Chrome'], javaScriptEnabled: false } },
  { name: 'firefox-js', port: 18103, use: { ...devices['Desktop Firefox'] } },
];

export default defineConfig({
  testDir: '.',
  testMatch: '*.spec.mjs',
  timeout: 90_000,
  fullyParallel: false,
  workers: servers.length,
  reporter: [['list']],
  projects: servers.map((s) => ({
    name: s.name,
    use: { ...s.use, baseURL: `http://127.0.0.1:${s.port}` },
  })),
  webServer: servers.map((s) => ({
    command:
      `sh -c 'D=$(mktemp -d) && FLUXO_DATA_DIR=$D FLUXO_LISTEN=127.0.0.1:${s.port} FLUXO_SETUP_TOKEN=browser-test-setup ` +
      `FLUXO_SECURE_COOKIES=false RUST_LOG=warn exec ${bin}'`,
    url: `http://127.0.0.1:${s.port}/static/styles.css`,
    reuseExistingServer: false,
    timeout: 30_000,
  })),
});
