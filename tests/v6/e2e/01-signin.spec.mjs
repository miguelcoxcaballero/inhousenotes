// 01-signin (TESTING §2): Welcome → Continue with Google (GIS stub) → Home with the Drive profile; a reload keeps
// the session; Sign out → Welcome and the keys are removed; app mode signs in with PKCE through
// `InhouseNative.openAuthUrl` + `handleInhouseNativeOAuth('?code=…')`. Home's lists themselves are W4.1 (02-home).
// The second half is the executor self-test: every op of host/fetch.js and host/platform.js against the real
// browser, driven through `shell.fx` / `shell.fx_results`.
import { expect, test } from '@playwright/test';

test.use({ serviceWorkers: 'block' });

const DRIVE_SCOPE = 'https://www.googleapis.com/auth/drive';
const CORS = { 'access-control-allow-origin': '*', 'access-control-allow-headers': '*', 'access-control-allow-methods': '*', 'access-control-expose-headers': '*' };
const KEYS = ['drive-token-v1', 'drive-remember-v1', 'drive-hint-email', 'drive-profile-v1', 'drive-refresh-token-v1', 'drive-pkce-verifier-v1', 'drive-consent-granted'];

// Google Identity Services, Drive `about` and the token endpoint. `gis` is how the consent answers:
// 'grant' | 'deny' | 'silent-fail' (only interactive requests succeed) | 'none' (the script never loads).
async function stubGoogle(page, { gis = 'grant', about = true } = {}) {
  const calls = { drive: [], token: [] };
  await page.route('https://accounts.google.com/gsi/client', route => route.fulfill({
    contentType: 'text/javascript',
    body: gis === 'none' ? '' : `(() => {
      const log = window.__gis = { mode: ${JSON.stringify(gis)}, requests: [], revoked: [], count: 0 };
      window.google = { accounts: { oauth2: {
        initTokenClient: config => {
          const client = { callback: config.callback, requestAccessToken: options => {
            log.requests.push(options);
            const ok = log.mode === 'grant' || (log.mode === 'silent-fail' && options.prompt === 'consent');
            setTimeout(() => client.callback(ok
              ? { access_token: 'gis-token-' + (++log.count), expires_in: 3600, scope: config.scope, token_type: 'Bearer' }
              : { error: options.prompt === 'none' ? 'interaction_required' : 'access_denied' }), 20);
          } };
          log.client = config.client_id;
          return client;
        },
        revoke: (token, done) => { log.revoked.push(token); done?.(); }
      } } };
    })();`
  }));
  await page.route('https://www.googleapis.com/**', async route => {
    const request = route.request(), url = new URL(request.url());
    if (request.method() === 'OPTIONS') return route.fulfill({ status: 204, headers: CORS });
    calls.drive.push({ path: url.pathname + url.search, auth: request.headers().authorization || '' });
    if (url.pathname === '/drive/v3/about' && url.searchParams.get('fields')?.startsWith('user') && about) {
      if (!request.headers().authorization) return route.fulfill({ status: 401, headers: CORS, json: { error: { code: 401 } } });
      return route.fulfill({ headers: CORS, json: { user: { displayName: 'Ana Test', emailAddress: 'ana@example.com', photoLink: '' } } });
    }
    return route.fulfill({ status: url.pathname === '/drive/v3/about' ? 401 : 404, headers: CORS, json: { error: { code: 404, message: 'not mocked' } } });
  });
  await page.route('https://oauth2.googleapis.com/token', async route => {
    if (route.request().method() === 'OPTIONS') return route.fulfill({ status: 204, headers: CORS });
    const body = Object.fromEntries(new URLSearchParams(route.request().postData() || ''));
    calls.token.push(body);
    await route.fulfill({ headers: CORS, json: { access_token: `pkce-token-${calls.token.length}`, expires_in: 3599, scope: DRIVE_SCOPE, token_type: 'Bearer', ...(body.grant_type === 'authorization_code' ? { refresh_token: 'refresh-1' } : {}) } });
  });
  return calls;
}

async function open(page, url = '/?e2e=1') {
  const problems = [];
  page.on('console', message => { if (message.type() === 'error' && !/Failed to load resource/.test(message.text())) problems.push(`console: ${message.text()}`); });
  page.on('pageerror', error => problems.push(`pageerror: ${error.message}`));
  await page.goto(url);
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  expect(await page.evaluate(() => window.__IHN_TEST_API__.ready())).toBe(true);
  return problems;
}
const cmd = (page, name, args) => page.evaluate(([n, a]) => window.__IHN_TEST_API__.cmd(n, a), [name, args]);
const stored = page => page.evaluate(keys => Object.fromEntries(keys.map(key => [key, localStorage.getItem(key)])), KEYS);

test('web: Welcome → Google → Home, reload keeps the session, sign out removes it', async ({ page }) => {
  const calls = await stubGoogle(page);
  const problems = await open(page);
  await expect(page.locator('#welcome-view')).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.dataset.appReady)).toBe('true');
  expect(await page.evaluate(() => [document.documentElement.dataset.inhouseApp, window.INHOUSE_NOTES_APP])).toEqual(['false', false]);

  // The first button only explains; the sheet's button signs in.
  await page.click('#btn-welcome-start');
  await expect(page.locator('#welcome-sheet')).toBeVisible();
  await expect(page.locator('#btn-welcome-login')).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(page.locator('#welcome-sheet')).toBeHidden();
  await expect(page.locator('#btn-welcome-start')).toBeFocused();
  await page.click('#btn-welcome-start');
  await page.click('#btn-welcome-login');

  await expect(page.locator('#drive-home')).toBeVisible();
  await expect(page.locator('#welcome-view')).toBeHidden();
  await expect(page.locator('#btn-drive-login')).toBeHidden();
  await expect(page.locator('#drive-profile')).toBeVisible();
  await expect(page.locator('#drive-search')).toBeEnabled();
  expect(await page.evaluate(() => window.__gis.requests)).toEqual([{ scope: DRIVE_SCOPE, include_granted_scopes: true, prompt: 'consent' }]);
  expect(await page.evaluate(() => window.__gis.client)).toBe('435784295430-cmug30o42f1vu4ijgor9sjb0ro4oo37o.apps.googleusercontent.com');

  // The profile comes from Drive with the new token and fills the menu.
  await page.click('#btn-drive-profile');
  await expect(page.locator('#drive-profile-menu')).toBeVisible();
  await expect(page.locator('#drive-profile-name')).toHaveText('Ana Test');
  await expect(page.locator('#drive-profile-email')).toHaveText('ana@example.com');
  expect(calls.drive.find(call => call.path.startsWith('/drive/v3/about?fields=user'))).toMatchObject({ auth: 'Bearer gis-token-1' });
  expect(await page.evaluate(() => document.getElementById('drive-profile-avatar').src)).toMatch(/^data:image\/svg\+xml/);
  const session = await stored(page);
  expect(JSON.parse(session['drive-token-v1'])).toMatchObject({ token: 'gis-token-1', grantedScopes: DRIVE_SCOPE });
  expect(JSON.parse(session['drive-token-v1']).expiry).toBeGreaterThan(Date.now() + 3_500_000);
  expect(session).toMatchObject({ 'drive-remember-v1': '1', 'drive-hint-email': 'ana@example.com' });
  expect(JSON.parse(session['drive-profile-v1'])).toMatchObject({ name: 'Ana Test', email: 'ana@example.com' });
  // Home was asked to list (the lists are rendered by W4.1).
  const state = await cmd(page, 'shell.state');
  expect(state.view).toBe('home');
  expect(state.signals.filter(signal => signal[0] === 0x21)).toHaveLength(1);
  // Menu Refresh closes the menu and lists again.
  await page.click('#btn-drive-menu-refresh');
  await expect(page.locator('#drive-profile-menu')).toBeHidden();
  expect((await cmd(page, 'shell.state')).signals.filter(signal => signal[0] === 0x21)).toHaveLength(2);

  // Reload: the boot script paints Home at once, the stored token is used and no consent is asked.
  await page.reload();
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(() => window.__IHN_TEST_API__.ready());
  expect(await page.evaluate(() => document.documentElement.dataset.bootDestination)).toBe('drive');
  await expect(page.locator('#drive-home')).toBeVisible();
  await expect(page.locator('#drive-profile-name')).toHaveText('Ana Test', { useInnerText: false });
  expect(await cmd(page, 'auth.state')).toMatchObject({ signedIn: true, reconnecting: false });
  expect(await page.evaluate(() => window.__gis.requests)).toEqual([]);

  // Sign out: token revoked, keys removed, Welcome; a reload stays on Welcome.
  await page.click('#btn-drive-profile');
  await page.click('#btn-drive-menu-logout');
  await expect(page.locator('#welcome-view')).toBeVisible();
  await expect(page.locator('#drive-home')).toBeHidden();
  expect(await page.evaluate(() => window.__gis.revoked)).toEqual(['gis-token-1']);
  expect(Object.values(await stored(page))).toEqual(KEYS.map(() => null));
  expect(await page.evaluate(() => window.__INHOUSE_BOOT_SIGNED_IN__)).toBe(false);
  await expect(page.locator('#drive-status')).toHaveText('Sign in to browse your Drive.');
  await page.reload();
  await expect(page.locator('#welcome-view')).toBeVisible();
  expect(problems).toEqual([]);
});

test('web: a refused consent says so; an expired session asks to reconnect', async ({ page }) => {
  await stubGoogle(page, { gis: 'deny' });
  await open(page);
  await page.click('#btn-welcome-start');
  await page.click('#btn-welcome-login');
  await expect(page.locator('#drive-status')).toHaveText('Sign-in failed. Try again.', { useInnerText: false });
  await expect(page.locator('#welcome-view')).toBeVisible();
  expect(await stored(page)).toMatchObject({ 'drive-token-v1': null });

  // A returning user whose token has expired and whose silent refresh Google refuses.
  await page.evaluate(scope => {
    localStorage.setItem('drive-token-v1', JSON.stringify({ token: 'expired', expiry: Date.now() - 1000, grantedScopes: scope }));
    localStorage.setItem('drive-remember-v1', '1');
    localStorage.setItem('drive-hint-email', 'ana@example.com');
  }, DRIVE_SCOPE);
  await page.reload();
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await expect(page.locator('#drive-home')).toBeVisible();
  await expect(page.locator('#btn-drive-login')).toHaveText('Reconnect');
  await expect(page.locator('#btn-drive-login')).toBeVisible();
  await expect(page.locator('#drive-status')).toHaveText('Drive connection paused. Sign in again to continue syncing.');
  expect(await page.evaluate(() => window.__gis.requests)).toEqual([{ scope: DRIVE_SCOPE, include_granted_scopes: true, prompt: 'none', login_hint: 'ana@example.com' }]);
  expect(await cmd(page, 'auth.state')).toMatchObject({ signedIn: false, reconnecting: true });
  // Reconnect signs in interactively.
  await page.evaluate(() => { window.__gis.mode = 'grant'; });
  await page.click('#btn-drive-login');
  await expect(page.locator('#btn-drive-login')).toBeHidden();
  await expect(page.locator('#drive-profile-name')).toHaveText('Ana Test', { useInnerText: false });
  expect(await cmd(page, 'auth.state')).toMatchObject({ signedIn: true, reconnecting: false, email: 'ana@example.com' });
});

test('web: Google Identity Services loading late is picked up', async ({ page }) => {
  await stubGoogle(page, { gis: 'none' });
  await open(page);
  expect(await cmd(page, 'auth.state')).toMatchObject({ gis: false });
  await page.evaluate(() => {
    window.google = { accounts: { oauth2: { initTokenClient: config => ({ callback: config.callback, requestAccessToken() {} }) } } };
  });
  await expect.poll(async () => (await cmd(page, 'auth.state')).gis, { timeout: 3000 }).toBe(true);
});

test('app mode: PKCE through InhouseNative.openAuthUrl and handleInhouseNativeOAuth', async ({ page }) => {
  const calls = await stubGoogle(page);
  await page.addInitScript(() => {
    window.InhouseNative = { openAuthUrl: url => { (window.__authUrls ||= []).push(url); }, getAppVersion: () => '1.0.28', getAppVersionCode: () => 29, getUpdateState: () => '{}', saveUpdateState() {} };
  });
  await page.route('https://raw.githubusercontent.com/**', route => route.fulfill({ status: 404, body: '' }));
  await page.route('https://api.github.com/**', route => route.fulfill({ status: 404, body: '' }));
  await open(page, '/?e2e=1&inhouse_app=1');
  expect(await page.evaluate(() => [document.documentElement.dataset.inhouseApp, window.INHOUSE_NOTES_APP, localStorage.getItem('inhouseNotesAppMode')])).toEqual(['true', true, '1']);
  expect(await page.evaluate(() => [!!document.getElementById('btn-drive-fullscreen'), !!document.getElementById('btn-fullscreen')])).toEqual([false, false]);
  expect((await cmd(page, 'app.state')).flags & 1).toBe(1);

  await page.click('#btn-welcome-start');
  await page.click('#btn-welcome-login');
  await expect.poll(() => page.evaluate(() => window.__authUrls?.length || 0)).toBe(1);
  await expect(page.locator('#drive-status')).toHaveText('Complete sign-in in the Google window.', { useInnerText: false });
  const auth = new URL(await page.evaluate(() => window.__authUrls[0]));
  const verifier = await page.evaluate(() => localStorage.getItem('drive-pkce-verifier-v1'));
  expect(auth.origin + auth.pathname).toBe('https://accounts.google.com/o/oauth2/v2/auth');
  expect(Object.fromEntries(auth.searchParams)).toEqual({
    client_id: '435784295430-9ta7t92s33r25bamvrnbmgt65iihqpfo.apps.googleusercontent.com',
    redirect_uri: 'com.local.inhousenotes:/oauth2redirect', response_type: 'code', scope: DRIVE_SCOPE, access_type: 'offline',
    include_granted_scopes: 'true', code_challenge: auth.searchParams.get('code_challenge'), code_challenge_method: 'S256',
    state: 'inhouse_notes_pkce', prompt: 'consent'
  });
  // The challenge is base64url(SHA-256(verifier)).
  expect(verifier).toMatch(/^[A-Za-z0-9_-]{43}$/);
  expect(await page.evaluate(async v => {
    const hash = new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(v)));
    return btoa(String.fromCharCode(...hash)).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
  }, verifier)).toBe(auth.searchParams.get('code_challenge'));

  // Android evaluates the callback with the redirect's query.
  expect(await page.evaluate(() => window.handleInhouseNativeOAuth('?code=4%2Fcode-1&state=inhouse_notes_pkce'))).toBe(true);
  await expect(page.locator('#drive-home')).toBeVisible();
  await expect(page.locator('#drive-profile-name')).toHaveText('Ana Test', { useInnerText: false });
  expect(calls.token).toEqual([{ client_id: '435784295430-9ta7t92s33r25bamvrnbmgt65iihqpfo.apps.googleusercontent.com', code: '4/code-1', code_verifier: verifier, grant_type: 'authorization_code', redirect_uri: 'com.local.inhousenotes:/oauth2redirect' }]);
  expect(await stored(page)).toMatchObject({ 'drive-refresh-token-v1': 'refresh-1', 'drive-pkce-verifier-v1': null, 'drive-consent-granted': '1', 'drive-remember-v1': '1' });
  expect(calls.drive.find(call => call.path.startsWith('/drive/v3/about?fields=user'))).toMatchObject({ auth: 'Bearer pkce-token-1' });

  // A later launch with only the refresh token renews silently; an OAuth error is shown.
  await page.evaluate(() => localStorage.removeItem('drive-token-v1'));
  await page.reload();
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await expect.poll(() => calls.token.length).toBe(2);
  expect(calls.token[1]).toEqual({ client_id: '435784295430-9ta7t92s33r25bamvrnbmgt65iihqpfo.apps.googleusercontent.com', grant_type: 'refresh_token', refresh_token: 'refresh-1' });
  await expect.poll(async () => (await cmd(page, 'auth.state')).signedIn).toBe(true);
  expect(await page.evaluate(() => window.handleInhouseNativeOAuth('#error=access_denied'))).toBe(false);
  await expect(page.locator('#drive-status')).toHaveText('Sign-in failed: access_denied', { useInnerText: false });
});

test('theme, profile menu, fullscreen and the loading overlay follow 5.18.0', async ({ page }) => {
  await stubGoogle(page);
  await page.addInitScript(() => { localStorage.setItem('drive-remember-v1', '1'); });
  await open(page);
  await expect(page.locator('#drive-home')).toBeVisible();
  // Night mode: stored choice, both toggles in step, <html data-theme>.
  await page.click('#btn-drive-profile');
  await page.locator('label[for="theme-toggle"]').click();
  expect(await page.evaluate(() => [document.documentElement.dataset.theme, localStorage.getItem('inhouse-theme'), document.getElementById('theme-toggle-editor').checked])).toEqual(['dark', 'dark', true]);
  // A click inside the menu keeps it open; outside closes it; Escape too.
  await expect(page.locator('#drive-profile-menu')).toBeVisible();
  await page.mouse.click(5, 300);
  await expect(page.locator('#drive-profile-menu')).toBeHidden();
  await page.click('#btn-drive-profile');
  expect(await page.getAttribute('#btn-drive-profile', 'aria-expanded')).toBe('true');
  await page.keyboard.press('Escape');
  await expect(page.locator('#drive-profile-menu')).toBeHidden();
  expect(await page.getAttribute('#btn-drive-profile', 'aria-expanded')).toBe('false');
  await page.locator('label[for="theme-toggle"]').evaluate(() => document.getElementById('theme-toggle').click());
  expect(await page.evaluate(() => [document.documentElement.dataset.theme, localStorage.getItem('inhouse-theme')])).toEqual(['light', 'light']);

  // Fullscreen button: label, active state and icon follow the fullscreen state (headless Chromium grants it).
  await expect(page.locator('#btn-drive-fullscreen')).toHaveAttribute('aria-label', 'Fullscreen');
  await page.click('#btn-drive-fullscreen');
  await expect(page.locator('#btn-drive-fullscreen')).toHaveAttribute('aria-label', 'Exit fullscreen');
  await expect(page.locator('#btn-drive-fullscreen')).toHaveClass(/active/);
  expect(await page.evaluate(() => !!document.fullscreenElement)).toBe(true);
  await page.click('#btn-drive-fullscreen');
  await expect(page.locator('#btn-drive-fullscreen')).toHaveAttribute('title', 'Fullscreen');
  await expect(page.locator('#btn-drive-fullscreen')).not.toHaveClass(/active/);

  // Loading overlay classes and ARIA.
  await cmd(page, 'shell.loading', { text: 'Opening document...' });
  await expect(page.locator('#loading-overlay')).toHaveClass(/visible/);
  await expect(page.locator('#loading-text')).toHaveText('Opening document...');
  expect(await page.getAttribute('#loading-overlay', 'aria-busy')).toBe('true');
  await cmd(page, 'shell.hide_loading');
  expect(await page.evaluate(() => document.getElementById('loading-overlay').className)).toBe('fading-out');
  await expect(page.locator('#loading-overlay')).toHaveClass('', { timeout: 2000 });
  await cmd(page, 'shell.loading', { text: 'Saving on this device…', home: true });
  await expect(page.locator('#loading-overlay')).toHaveClass('home-save-transition visible');
  await cmd(page, 'shell.hide_loading');
  await expect(page.locator('#loading-overlay')).toHaveClass('', { timeout: 2000 });
});

// Going offline and coming back, in the browser: the `offline` event, the NETWORK signal, the session flag, the
// status line (`#drive-status` is `display: none` in the stylesheet, so its text is the whole of it) and the probes
// of Drive — none while connected, then after 30 s, 60 s, 120 s … (v5: offline-library-v5.js scheduleOfflineProbe),
// at once on `online`, and none again once Drive answered. Timers run on Playwright's clock.
test('offline: NETWORK signal, status line, probe backoff, `online` and recovery', async ({ page, context }) => {
  await stubGoogle(page);
  let down = false;
  const probes = [];
  // Registered after stubGoogle, so it is asked first.
  await page.route('https://www.googleapis.com/drive/v3/about?fields=kind', route => {
    probes.push({ method: route.request().method(), auth: route.request().headers().authorization || '', cache: route.request().headers()['cache-control'] || '' });
    return down ? route.abort('internetdisconnected') : route.fallback();
  });
  await page.addInitScript(scope => {
    if (localStorage.getItem('drive-remember-v1')) return;
    localStorage.setItem('drive-token-v1', JSON.stringify({ token: 'stored-token', expiry: Date.now() + 3_000_000, grantedScopes: scope }));
    localStorage.setItem('drive-remember-v1', '1');
    localStorage.setItem('drive-hint-email', 'ana@example.com');
  }, DRIVE_SCOPE);
  await page.clock.install();
  await open(page);
  const NETWORK = 0x26;
  const network = async () => (await cmd(page, 'shell.state')).signals.filter(signal => signal[0] === NETWORK).map(signal => signal[1]);
  const flag = () => page.evaluate(() => sessionStorage.getItem('ihn-network-offline'));

  // Connected: one unauthenticated probe at boot, Drive answered (401 counts), nothing is probed again.
  await expect.poll(() => probes.length).toBe(1);
  expect(probes[0]).toMatchObject({ method: 'GET', auth: '' });
  await expect(page.locator('#drive-home')).toBeVisible();
  await expect.poll(flag).toBe('0');
  expect((await cmd(page, 'shell.state')).offline).toBe(false);
  await expect(page.locator('#drive-status')).toHaveText('Loading your PDFs...', { useInnerText: false });
  await page.clock.runFor(10 * 60_000);
  expect(probes).toHaveLength(1);
  expect(await network()).toEqual([]);

  // The network goes away.
  down = true;
  await context.setOffline(true);
  await expect.poll(async () => (await cmd(page, 'shell.state')).offline).toBe(true);
  expect(await network()).toEqual(['0']);
  expect(await flag()).toBe('1');
  expect(await page.evaluate(() => getComputedStyle(document.getElementById('drive-status')).display)).toBe('none');
  // Without a local library the line keeps its text (5.18.0 only rewrites it from the library's reload) …
  await expect(page.locator('#drive-status')).toHaveText('Loading your PDFs...', { useInnerText: false });
  // … and with one (the offline library unit reports it) it is 5.18.0's offline line.
  await cmd(page, 'auth.local_library', { ready: true });
  await expect(page.locator('#drive-status')).toHaveText('Offline · changes stay on this device', { useInnerText: false });
  await expect(page.locator('#btn-drive-login')).toBeHidden();
  await expect(page.locator('#drive-search')).toBeEnabled();

  // Probes: 30 s after going offline, then 60 s, 120 s after each failure.
  const after = async (quietMs, count) => {
    await page.clock.runFor(quietMs - 1000);
    expect(probes, `no probe before ${quietMs} ms`).toHaveLength(count - 1);
    await page.clock.runFor(1500);
    await expect.poll(() => probes.length, `a probe after ${quietMs} ms`).toBe(count);
    // The failed answer has to reach the app before the next interval starts counting.
    await page.waitForTimeout(150);
    await page.clock.runFor(100);
  };
  await after(30_000, 2);
  await after(60_000 - 600, 3);
  await after(120_000 - 600, 4);
  expect((await cmd(page, 'shell.state')).offline).toBe(true);
  expect(await network()).toEqual(['0']);

  // `online` probes at once; Drive still does not answer, so the app stays offline.
  await context.setOffline(false);
  await expect.poll(() => probes.length).toBe(5);
  await page.waitForTimeout(150);
  expect((await cmd(page, 'shell.state')).offline).toBe(true);
  expect(await network()).toEqual(['0']);
  // A reload in the same tab starts offline (the session flag) and probes at boot.
  await page.reload();
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(() => window.__IHN_TEST_API__.ready());
  expect((await cmd(page, 'shell.state')).offline).toBe(true);
  await expect.poll(() => probes.length).toBe(6);
  await page.waitForTimeout(150);
  await page.clock.runFor(100);

  // Drive is back: the next probe (30 s again) finds it, the app is connected and stops probing.
  down = false;
  await page.clock.runFor(31_000);
  await expect.poll(async () => (await cmd(page, 'shell.state')).offline).toBe(false);
  expect(probes).toHaveLength(7);
  expect(await network()).toEqual(['1']);
  expect(await flag()).toBe('0');
  await page.clock.runFor(10 * 60_000);
  expect(probes).toHaveLength(7);
});

// ---- executor self-test ---------------------------------------------------------------------------------------

// One effect record: `fx(page, 0x30, ['r'], ['s', 'GET'], …)` → {req, obj}; field types as in shell/fx.rs `probe`.
const fx = (page, op, ...fields) => cmd(page, 'shell.fx', { op, fields });
const results = async page => (await cmd(page, 'shell.fx_results')).results;
async function result(page, req) {
  let found;
  await expect.poll(async () => {
    found = (await results(page)).find(entry => entry.req === req) || found;
    return !!found;
  }, { timeout: 5000 }).toBe(true);
  return found;
}
const NONE = [['b', 0]], bytes = text => [['b', 1], ['x', text]], blob = (obj, start = 0, end = -1) => [['b', 2], ['u', obj], ['f', start], ['f', end]];
const fetchOp = (page, method, url, headers, flags, deadline, idle, body = NONE) =>
  fx(page, 0x30, ['r'], ['s', method], ['s', url], ['s', headers], ['u', flags], ['f', deadline], ['f', idle], ...body);
// A fetch result: `str headers, body…`.
function parse(entry) {
  const raw = Buffer.from(entry.hex, 'hex');
  return { status: entry.status, obj: entry.obj, text: entry.text, headerLength: raw.readUInt32LE(0) };
}

test('executors: fetch, blobs, digest and deflate (host/fetch.js)', async ({ page }) => {
  await stubGoogle(page);
  const seen = [];
  let release;
  await page.route('**/__fx/**', async route => {
    const request = route.request(), name = new URL(request.url()).pathname.split('/').pop();
    seen.push({ name, method: request.method(), headers: request.headers(), body: request.postData() });
    if (name === 'hold') return new Promise(resolve => { release = () => { route.abort().catch(() => {}); resolve(); }; });
    if (name === 'fail') return route.abort('connectionrefused');
    if (name === 'resume') return route.fulfill({ status: 308, headers: { range: 'bytes=0-4', 'x-probe': 'yes' }, body: '' });
    return route.fulfill({ status: name === 'missing' ? 404 : 200, contentType: 'text/plain', headers: { etag: '"v7"' }, body: `echo:${request.postData() || ''}` });
  });
  await open(page);
  const base = new URL(page.url()).origin + '/__fx/';

  // POST with headers and an inline body → status, headers, body.
  let call = await fetchOp(page, 'POST', base + 'echo', 'Content-Type: text/plain\nX-One: 1\n', 4 | 8, 5000, 0, bytes('héllo'));
  let got = await result(page, call.req);
  expect(got.status).toBe(200);
  expect(got.text).toContain('etag: "v7"\n');
  expect(got.text.endsWith('echo:héllo')).toBe(true);
  expect(seen.at(-1)).toMatchObject({ method: 'POST', body: 'héllo' });
  expect(seen.at(-1).headers).toMatchObject({ 'x-one': '1', 'content-type': 'text/plain' });
  // A 404 is a result too (Rust decides what it means).
  got = await result(page, (await fetchOp(page, 'GET', base + 'missing', '', 0, 5000, 0)).req);
  expect(got.status).toBe(404);
  // Status only: no body is read.
  got = await result(page, (await fetchOp(page, 'GET', base + 'echo', '', 16, 5000, 0)).req);
  expect([got.status, got.text.includes('echo:')]).toEqual([200, false]);
  // Network failure, deadline and abort.
  got = await result(page, (await fetchOp(page, 'GET', base + 'fail', '', 0, 5000, 0)).req);
  expect(got.status).toBe(-1);
  expect(got.text).toMatch(/fetch/i);
  got = await result(page, (await fetchOp(page, 'GET', base + 'hold', '', 0, 1000, 0)).req);
  expect(got.status).toBe(-2);
  release();
  call = await fetchOp(page, 'GET', base + 'hold', '', 0, 0, 0);
  await expect.poll(() => seen.filter(entry => entry.name === 'hold').length).toBe(2);
  await fx(page, 0x31, ['u', call.req]);
  expect((await result(page, call.req)).status).toBe(-3);
  release();

  // Blobs: create, slice, concat, read, info, object URL.
  const hello = (await fx(page, 0x38, ['o'], ['s', 'text/plain'], ['x', 'hello world'])).obj;
  const world = (await fx(page, 0x39, ['o'], ['u', hello], ['f', 6], ['f', -1], ['s', ''])).obj;
  const both = (await fx(page, 0x3a, ['o'], ['s', 'text/x-test'], ['u', 2], ['u', world], ['u', hello])).obj;
  expect((await result(page, (await fx(page, 0x3b, ['r'], ['u', both], ['f', 0], ['f', -1])).req)).text).toBe('worldhello world');
  expect((await result(page, (await fx(page, 0x3b, ['r'], ['u', both], ['f', 5], ['f', 10])).req)).text).toBe('hello');
  got = await result(page, (await fx(page, 0x3e, ['r'], ['u', both])).req);
  expect(Buffer.from(got.hex, 'hex').readDoubleLE(0)).toBe(16);
  expect(got.text).toContain('text/x-test');
  got = await result(page, (await fx(page, 0x3c, ['r'], ['u', both])).req);
  expect(got.text).toMatch(/^blob:http/);
  expect(await page.evaluate(url => fetch(url).then(response => response.text()), got.text)).toBe('worldhello world');
  await fx(page, 0x3d, ['s', got.text]);
  expect(await page.evaluate(url => fetch(url).then(() => 'alive', () => 'revoked'), got.text)).toBe('revoked');
  expect((await result(page, (await fx(page, 0x3b, ['r'], ['u', 12345], ['f', 0], ['f', -1])).req)).status).toBe(-1);

  // A resumable-upload chunk: PUT of a blob range with Content-Range, 308 + Range back; keepalive flag.
  got = await result(page, (await fetchOp(page, 'PUT', base + 'resume', 'Content-Range: bytes 0-4/16\n', 1, 5000, 0, blob(both, 0, 5))).req);
  expect(got.status).toBe(308);
  expect(got.text).toContain('range: bytes=0-4\n');
  expect(seen.at(-1)).toMatchObject({ method: 'PUT', body: 'world' });
  expect(seen.at(-1).headers['content-range']).toBe('bytes 0-4/16');
  // A download delivered as a Blob handle (flag 2), then read back and released.
  got = await result(page, (await fetchOp(page, 'POST', base + 'echo', '', 2, 5000, 3000, bytes('pdf bytes'))).req);
  expect(got.status).toBe(200);
  expect(got.obj).toBeGreaterThan(0);
  expect((await result(page, (await fx(page, 0x3b, ['r'], ['u', got.obj], ['f', 0], ['f', -1])).req)).text).toBe('echo:pdf bytes');
  await fx(page, 0x01, ['u', got.obj]);
  expect((await result(page, (await fx(page, 0x3b, ['r'], ['u', got.obj], ['f', 0], ['f', -1])).req)).status).toBe(-1);

  // SHA-256 of bytes and of a blob.
  got = await result(page, (await fx(page, 0x78, ['r'], ...bytes('abc'))).req);
  expect(got.hex).toBe('ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad');
  got = await result(page, (await fx(page, 0x78, ['r'], ...blob(hello, 0, 5))).req);
  expect(got.hex).toBe('2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824');
  // Deflate raw → blob → inflate; zlib wrapper; garbage fails.
  const text = 'inhouse notes '.repeat(40);
  got = await result(page, (await fx(page, 0x90, ['r'], ['b', 4], ...bytes(text))).req);
  expect(Buffer.from(got.hex, 'hex').readDoubleLE(0)).toBeLessThan(60);
  expect((await result(page, (await fx(page, 0x90, ['r'], ['b', 1], ...blob(got.obj))).req)).text).toBe(text);
  got = await result(page, (await fx(page, 0x90, ['r'], ['b', 2], ...bytes(text))).req);
  expect(got.hex.startsWith('789c')).toBe(true);
  expect((await result(page, (await fx(page, 0x90, ['r'], ['b', 1], ...bytes('not deflate'))).req)).status).toBe(-1);

  // The Drive client on top: token header, error mapping, accepted statuses.
  await page.route('https://www.googleapis.com/drive/v3/files/**', route => {
    const request = route.request();
    if (request.method() === 'OPTIONS') return route.fulfill({ status: 204, headers: CORS });
    seen.push({ name: 'drive', headers: request.headers() });
    return route.fulfill({ status: request.url().includes('gone') ? 404 : 200, headers: CORS, json: { id: 'f1' } });
  });
  await page.click('#btn-welcome-start');
  await page.click('#btn-welcome-login');
  await expect(page.locator('#drive-home')).toBeVisible();
  let id = await cmd(page, 'drive.client.request', { url: 'https://www.googleapis.com/drive/v3/files/f1?fields=id' });
  got = await result(page, Number(id));
  expect([got.status, got.text.endsWith('{"id":"f1"}')]).toEqual([200, true]);
  expect(seen.at(-1).headers.authorization).toBe('Bearer gis-token-1');
  id = await cmd(page, 'drive.client.request', { url: 'https://www.googleapis.com/drive/v3/files/gone' });
  expect(await result(page, Number(id))).toMatchObject({ status: -404, text: 'Drive API error 404: {"id":"f1"}' });
  id = await cmd(page, 'drive.client.request', { url: 'https://www.googleapis.com/drive/v3/files/gone', accept404: true });
  expect((await result(page, Number(id))).status).toBe(404);
});

test('executors: GIS, native bridges, files, clipboard, share, download, history (host/platform.js)', async ({ page, context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await stubGoogle(page);
  await page.addInitScript(() => {
    const log = window.__native = [];
    window.InhouseNative = {
      getPdfExportApiVersion: () => 2,
      beginPdfExport: name => { log.push(['begin', name]); },
      appendPdfExportChunk: chunk => { log.push(['chunk', chunk]); },
      finishPdfExport: () => { log.push(['finish']); setTimeout(() => window.handleInhousePdfExportResult(JSON.stringify({ ok: true, name: 'a.pdf' })), 10); },
      explode: () => { throw new Error('native boom'); }
    };
    window.InhouseBackground = { schedule: () => log.push(['schedule']), cancel: () => log.push(['cancel']), complete: json => log.push(['complete', json]) };
    navigator.canShare = data => !!data.files?.length;
    navigator.share = async data => { log.push(['share', data.title, data.files[0].name, data.files[0].type, await data.files[0].text()]); };
  });
  await open(page);

  // GIS: init, request (answer as JSON), revoke.
  expect((await result(page, (await fx(page, 0xa0, ['r'], ['s', 'client-x'], ['s', 'scope-x'])).req)).status).toBe(0);
  let got = await result(page, (await fx(page, 0xa1, ['r'], ['b', 0], ['s', 'scope-x'], ['s', 'ana@example.com'])).req);
  expect(JSON.parse(got.text)).toMatchObject({ access_token: 'gis-token-1', expires_in: 3600 });
  expect(await page.evaluate(() => window.__gis.requests.at(-1))).toEqual({ scope: 'scope-x', include_granted_scopes: true, prompt: 'none', login_hint: 'ana@example.com' });
  await fx(page, 0xa2, ['s', 'gis-token-1']);
  expect(await page.evaluate(() => window.__gis.revoked)).toEqual(['gis-token-1']);

  // Native bridge: a call with its result, a missing method, a throwing one; a blob as base64 chunks.
  expect(await result(page, (await fx(page, 0xa4, ['r'], ['s', 'getPdfExportApiVersion'], ['u', 0])).req)).toMatchObject({ status: 0, text: '2' });
  expect((await result(page, (await fx(page, 0xa4, ['r'], ['s', 'savePdf'], ['u', 2], ['s', 'a.pdf'], ['s', 'AAAA'])).req)).status).toBe(-1);
  expect(await result(page, (await fx(page, 0xa4, ['r'], ['s', 'explode'], ['u', 0])).req)).toMatchObject({ status: -2, text: 'native boom' });
  await fx(page, 0xa4, ['r'], ['s', 'beginPdfExport'], ['u', 1], ['s', 'a.pdf']);
  const pdf = (await fx(page, 0x38, ['o'], ['s', 'application/pdf'], ['x', '%PDF-1.7 0123456789'])).obj;
  expect((await result(page, (await fx(page, 0xa5, ['r'], ['u', pdf], ['s', 'appendPdfExportChunk'], ['u', 8])).req)).status).toBe(0);
  await fx(page, 0xa4, ['r'], ['s', 'finishPdfExport'], ['u', 0]);
  const native = await page.evaluate(() => window.__native);
  expect(native[0]).toEqual(['begin', 'a.pdf']);
  const chunks = native.filter(entry => entry[0] === 'chunk').map(entry => entry[1]);
  expect(chunks.every(chunk => chunk.length <= 8)).toBe(true);
  expect(Buffer.from(chunks.join(''), 'base64').toString()).toBe('%PDF-1.7 0123456789');
  expect(chunks.join('')).toBe(Buffer.from('%PDF-1.7 0123456789').toString('base64'));
  // The export result and the OAuth redirect arrive as messages (channels 2 and 1).
  await page.evaluate(() => window.handleInhouseNativeOAuth('#error=access_denied'));
  await expect.poll(async () => (await cmd(page, 'shell.fx_results')).messages.length, { timeout: 3000 }).toBeGreaterThan(0);
  await page.evaluate(() => { window.handleInhousePdfExportResult({ ok: false, message: 'disk full' }); window.handleInhouseNativeOAuth('?error=x'); });
  expect((await cmd(page, 'shell.fx_results')).messages).toEqual([{ chan: 2, data: '{"ok":false,"message":"disk full"}' }, { chan: 1, data: 'error=x' }]);

  // Background bridge and the boot flag.
  for (const what of [0, 1, 2]) expect((await result(page, (await fx(page, 0xab, ['r'], ['b', what], ['s', '{"busy":false,"pending":0,"errors":[]}'])).req)).status).toBe(0);
  expect((await page.evaluate(() => window.__native)).slice(-3)).toEqual([['schedule'], ['cancel'], ['complete', '{"busy":false,"pending":0,"errors":[]}']]);
  await fx(page, 0xaa, ['b', 1]);
  expect(await page.evaluate(() => window.__INHOUSE_BOOT_SIGNED_IN__)).toBe(true);

  // File chooser: the files of a change event become blob handles.
  await fx(page, -1);
  await page.evaluate(() => { document.getElementById('pdf-input').multiple = true; });
  await page.setInputFiles('#pdf-input', [{ name: 'a.pdf', mimeType: 'application/pdf', buffer: Buffer.from('%PDF-a') }, { name: 'b.png', mimeType: 'image/png', buffer: Buffer.from('png!') }]);
  let files;
  await expect.poll(async () => { files = (await results(page)).find(entry => entry.status === 2); return !!files; }).toBe(true);
  expect(JSON.parse(files.text)).toMatchObject([{ name: 'a.pdf', type: 'application/pdf', size: 6 }, { name: 'b.png', type: 'image/png', size: 4 }]);
  expect((await result(page, (await fx(page, 0x3b, ['r'], ['u', files.base + 1], ['f', 0], ['f', -1])).req)).text).toBe('png!');
  got = await result(page, (await fx(page, 0x3e, ['r'], ['u', files.base])).req);
  expect(got.text).toContain('a.pdf');

  // Clipboard, share, download.
  expect((await result(page, (await fx(page, 0xb4, ['r'], ['s', 'https://inhousenotes.com/?drive=x'])).req)).status).toBe(0);
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe('https://inhousenotes.com/?drive=x');
  expect((await result(page, (await fx(page, 0xb8, ['r'], ['u', pdf], ['s', 'notes.pdf'], ['s', 'application/pdf'], ['s', 'notes.pdf'])).req)).status).toBe(0);
  expect((await page.evaluate(() => window.__native)).at(-1)).toEqual(['share', 'notes.pdf', 'notes.pdf', 'application/pdf', '%PDF-1.7 0123456789']);
  expect((await result(page, (await fx(page, 0xb8, ['r'], ['u', 4242], ['s', 'x'], ['s', ''], ['s', ''])).req)).status).toBe(-1);
  const [download] = await Promise.all([page.waitForEvent('download'), fx(page, 0xb9, ['u', pdf], ['s', 'notes.pdf'], ['f', 0], ['f', 1200])]);
  expect(download.suggestedFilename()).toBe('notes.pdf');

  // History and resize.
  await fx(page, 0xc0, ['b', 0], ['s', '?e2e=1&step=1']);
  expect(new URL(page.url()).searchParams.get('step')).toBe('1');
  await fx(page, 0xc0, ['b', 1], ['s', '?e2e=1&step=2']);
  expect(new URL(page.url()).searchParams.get('step')).toBe('2');
  await cmd(page, 'shell.fx_results');
  await fx(page, 0xc0, ['b', 2], ['s', '']);
  await expect.poll(async () => (await cmd(page, 'shell.fx_results')).messages, { timeout: 3000 }).toEqual([{ chan: 0x10, data: expect.stringMatching(/\?e2e=1$/) }]);
  await page.evaluate(() => { window.__resizes = 0; window.addEventListener('resize', () => { window.__resizes += 1; }); });
  await fx(page, 0xbd);
  expect(await page.evaluate(() => window.__resizes)).toBe(1);
  // The updater is only created in app mode (14-updater); without the app the op still answers.
  expect((await result(page, (await fx(page, 0xa8, ['r'])).req)).status).toBe(0);
  expect(await page.evaluate(() => typeof window.InhouseUpdater?.settings)).toBe('function');
  // location.replace navigates.
  await Promise.all([page.waitForNavigation(), fx(page, 0xc0, ['b', 4], ['s', '/?e2e=1&replaced=1'])]);
  expect(new URL(page.url()).searchParams.get('replaced')).toBe('1');
});
