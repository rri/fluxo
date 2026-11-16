// End-to-end account flows in a real browser, with and without JavaScript.
// Tests run in order against one fresh server per project; later tests build on earlier ones.

import { test, expect } from '@playwright/test';
import { createHmac } from 'node:crypto';

test.describe.configure({ mode: 'serial' });

const PASSWORD = 'a long enough password';
const state = { secret: null, codes: [], invitee: 'kofi.m' };

// ---------------------------------------------------------------- helpers

function base32(text) {
  const abc = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';
  let bits = '';
  for (const ch of text.replace(/[\s=]/g, '').toUpperCase()) bits += abc.indexOf(ch).toString(2).padStart(5, '0');
  const out = [];
  for (let i = 0; i + 8 <= bits.length; i += 8) out.push(parseInt(bits.slice(i, i + 8), 2));
  return Buffer.from(out);
}

function totp(key, t = Date.now()) {
  const counter = Buffer.alloc(8);
  counter.writeBigUInt64BE(BigInt(Math.floor(t / 1000 / 30)));
  const h = createHmac('sha1', key).update(counter).digest();
  const o = h[h.length - 1] & 15;
  return String(((h.readUInt32BE(o) & 0x7fffffff) % 1_000_000)).padStart(6, '0');
}

/** A code from a 30-second step that hasn't been used yet (codes can't be replayed). */
async function freshCode(key) {
  const wait = 30_000 - (Date.now() % 30_000) + 500;
  await new Promise((r) => setTimeout(r, wait));
  return totp(key);
}

/** Type a code into the code field: six boxes with JavaScript, one field without. */
async function enterCode(page, code, js, submitName) {
  if (js) {
    const boxes = page.locator('.otp input');
    await expect(boxes).toHaveCount(6);
    await boxes.first().click();
    await page.keyboard.type(code); // the last digit submits the form by itself
  } else {
    await page.locator('#code').fill(code);
    await page.getByRole('button', { name: submitName }).click();
  }
}

async function signOut(page) {
  await page.locator('summary[aria-label="Account"]').click();
  await page.getByRole('button', { name: 'Sign out' }).click();
  await expect(page).toHaveURL(/\/login$/);
}

const js = (testInfo) => testInfo.project.use.javaScriptEnabled !== false;

// ---------------------------------------------------------------- tests

test('first person sets up fluxo and turns on 2FA', async ({ page }, info) => {
  await page.goto('/');
  await expect(page).toHaveURL(/\/setup$/);
  await expect(page.getByRole('heading', { name: 'Set up fluxo' })).toBeVisible();
  await page.getByLabel('Username').fill('Ada');
  await page.getByLabel('Password', { exact: true }).fill(PASSWORD);
  await page.getByLabel('Confirm password').fill(PASSWORD);
  await page.getByRole('button', { name: 'Create Administrator account' }).click();

  await expect(page).toHaveURL(/\/settings\/2fa\/setup\?first=1$/);
  await expect(page.getByAltText(/QR code/)).toBeVisible();
  await page.getByText('Can’t scan? Enter this key').click();
  state.secret = base32(await page.locator('.secret').textContent());
  await enterCode(page, totp(state.secret), js(info), 'Turn on 2FA');

  await expect(page.getByRole('heading', { name: 'Save your recovery codes' })).toBeVisible();
  state.codes = await page.locator('#codes span').allTextContents();
  expect(state.codes).toHaveLength(8);
  await page.getByLabel('I’ve saved these somewhere safe').check();
  await page.getByRole('button', { name: 'Continue' }).click();
  await expect(page.getByRole('heading', { name: 'Focus' })).toBeVisible();
  await expect(page.getByText('Signed in as ada')).toBeVisible();
});

test('sign out, then back in with the authenticator', async ({ page }, info) => {
  await page.goto('/login');
  await page.getByLabel('Username').fill('ada');
  await page.getByLabel('Password').fill('wrong password!');
  await page.getByRole('button', { name: 'Sign in' }).click();
  await expect(page.getByRole('alert')).toContainText('don’t match');
  await page.getByLabel('Password').fill(PASSWORD);
  await page.getByRole('button', { name: 'Sign in' }).click();
  await expect(page).toHaveURL(/\/login\/2fa$/);
  await enterCode(page, await freshCode(state.secret), js(info), 'Verify');
  await expect(page.getByRole('heading', { name: 'Focus' })).toBeVisible();
  await signOut(page);
});

test('a recovery code signs in once', async ({ page }) => {
  await page.goto('/login');
  await page.getByLabel('Username').fill('ada');
  await page.getByLabel('Password').fill(PASSWORD);
  await page.getByRole('button', { name: 'Sign in' }).click();
  await page.getByRole('link', { name: 'Use a recovery code instead' }).click();
  await page.getByLabel('Recovery code').fill(state.codes[0].toUpperCase());
  await page.getByRole('button', { name: 'Verify' }).click();
  await expect(page).toHaveURL(/\/settings/);
  await expect(page.getByText('7 of 8 unused')).toBeVisible();
});

test('theme is saved to the account', async ({ page }, info) => {
  await signInAsAda(page, info);
  await page.goto('/settings');
  if (js(info)) {
    const saved = page.waitForResponse((r) => r.url().endsWith('/settings/theme'));
    await page.getByRole('button', { name: 'Toggle theme' }).click(); // system → light
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
    await saved;
  } else {
    await page.locator('label[for="t-l"]').click();
    await page.locator('#app-h').locator('xpath=../..').getByRole('button', { name: 'Save' }).click();
  }
  await page.goto('/');
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
});

test('an invitation link brings in one new person', async ({ page, browser }, info) => {
  await signInAsAda(page, info);
  await page.goto('/settings');
  await page.getByPlaceholder(/Who is it for/).fill('For Kofi');
  await page.getByRole('button', { name: 'Create invitation link' }).click();
  const link = await page.locator('#new-link').inputValue();
  expect(link).toContain('/join/');

  const other = await browser.newContext({ javaScriptEnabled: js(info) });
  const guest = await other.newPage();
  await guest.goto(link);
  await guest.getByLabel('Username').fill(state.invitee);
  await guest.getByLabel('Password', { exact: true }).fill(PASSWORD);
  await guest.getByLabel('Confirm password').fill(PASSWORD);
  await guest.getByRole('button', { name: 'Create account' }).click();
  await expect(guest).toHaveURL(/\/settings\/2fa\/setup\?first=1$/);
  await guest.getByRole('link', { name: 'Skip for now' }).click();
  await expect(guest.getByText(`Signed in as ${state.invitee}`)).toBeVisible();
  await expect(guest.locator('#adm-h')).toHaveCount(0);
  await guest.goto(link);
  await expect(guest.getByRole('heading', { name: 'This link can’t be used' })).toBeVisible();
  await other.close();
});

test('a locked-out person gets back in with an Administrator’s code', async ({ page, browser }, info) => {
  await signInAsAda(page, info);
  await page.goto('/settings');
  await page.locator('#rec-user').fill(state.invitee);
  await page.getByRole('button', { name: 'Make a recovery code' }).click();
  const code = await page.locator('#issued-code').inputValue();
  expect(code).toMatch(/^[A-Z2-9]{4}-[A-Z2-9]{4}-[A-Z2-9]{4}$/);

  const other = await browser.newContext({ javaScriptEnabled: js(info) });
  const them = await other.newPage();
  await them.goto('/login');
  await them.getByRole('link', { name: 'Locked out?' }).click();
  await them.getByLabel('Username').fill(state.invitee);
  await them.getByLabel(/Recovery code/).fill(code.toLowerCase());
  await them.getByRole('button', { name: 'Continue' }).click();
  await expect(them).toHaveURL(/\/reset-password$/);
  await them.goto('/settings');
  await expect(them).toHaveURL(/\/reset-password$/); // nothing else until a new password is set
  await them.getByLabel('New password', { exact: true }).fill('a completely new password');
  await them.getByLabel('Confirm new password').fill('a completely new password');
  await them.getByRole('button', { name: 'Save and continue' }).click();
  await expect(them).toHaveURL(/\/settings\/2fa\/setup\?first=1$/);
  await other.close();
});

test('pages send a strict content security policy and break nothing under it', async ({ page }) => {
  const problems = [];
  page.on('console', (m) => { if (m.type() === 'error') problems.push(m.text()); });
  const res = await page.goto('/login');
  expect(res.headers()['content-security-policy']).toContain("script-src 'self'");
  await page.goto('/setup');
  expect(problems).toEqual([]);
});

// ada has 2FA on, so signing in uses a fresh authenticator code.
async function signInAsAda(page, info) {
  await page.goto('/login');
  if (!page.url().endsWith('/login')) return; // already signed in
  await page.getByLabel('Username').fill('ada');
  await page.getByLabel('Password').fill(PASSWORD);
  await page.getByRole('button', { name: 'Sign in' }).click();
  await enterCode(page, await freshCode(state.secret), js(info), 'Verify');
  await expect(page.getByRole('heading', { name: 'Focus' })).toBeVisible();
}

test('break a goal down and track a task', async ({ page }, info) => {
  await signInAsAda(page, info);
  await page.goto('/goals');
  await page.getByRole('link', { name: '+ New goal' }).first().click();
  await page.getByLabel('The goal').fill('Launch the beta');
  await page.getByLabel('Why it matters').fill('Real teams, real goals.');
  await page.getByRole('button', { name: 'Create goal' }).click();
  await expect(page.getByRole('heading', { level: 1, name: 'Launch the beta' })).toBeVisible();

  const addStep = async (title, days, needs) => {
    await page.getByLabel('Title', { exact: true }).fill(title);
    await page.getByLabel('Likely duration').fill(days);
    if (needs) await page.getByLabel('It needs (optional)').selectOption({ label: needs });
    await page.getByRole('button', { name: 'Add step' }).click();
  };
  await addStep('Schema', '3');
  await addStep('API', '5', 'Schema');
  // A sub-goal: with JavaScript the task-only fields hide themselves.
  await page.getByLabel('Title', { exact: true }).fill('Accounts');
  await page.locator('label[for="k-goal"]').click();
  if (js(info)) await expect(page.getByLabel('Likely duration')).toBeHidden();
  await page.getByRole('button', { name: 'Add step' }).click();

  const graph = page.locator('.graph');
  await expect(graph.getByText('API')).toBeVisible();
  if (js(info)) {
    await expect(graph).toHaveClass(/enhanced/); // laid out as a graph
  } else {
    await expect(page.getByText('Needs: Schema')).toBeVisible(); // plain list
  }

  await graph.getByRole('link', { name: /API/ }).click();
  await page.locator('label[for="s-p"]').click(); // In progress
  await page.getByLabel('Remaining').fill('2');
  await page.getByRole('button', { name: 'Save task' }).click();
  await expect(page.getByText('In progress', { exact: true }).first()).toBeVisible();

  await page.getByPlaceholder('New risk…').fill('Vendor API changes');
  await page.getByLabel('Likelihood %').fill('30');
  await page.getByLabel('Impact at least, days').fill('2');
  await page.getByLabel('Impact at most, days').fill('4');
  await page.getByRole('button', { name: 'Add risk' }).click();
  await expect(page.locator('.risk .lk')).toHaveText('30%');
  const slider = page.getByLabel('Likelihood of Vendor API changes');
  if (js(info)) {
    const saved = page.waitForResponse((r) => /\/risks\/\d+$/.test(r.url()) && r.request().method() === 'POST');
    await slider.fill('10'); // saves on change
    await saved;
    await page.waitForLoadState();
    await expect(page.locator('.risk .lk')).toHaveText('10%');
  } else {
    await slider.fill('10');
    await page.locator('.risk-form').getByRole('button', { name: 'Save' }).click();
    await expect(page.locator('.risk .lk')).toHaveText('10%');
  }
  await page.reload();
  await expect(page.getByRole('img', { name: /30% → 10%/ })).toBeAttached();
});
