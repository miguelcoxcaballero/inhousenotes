// The mock of everything outside the app's own origin: Google Identity Services, the OAuth token endpoint, Drive
// (drive.mjs), public Drive downloads, avatars/thumbnails/fonts, and GitHub (github.mjs).
//
//   const mock = createMock();                 // one per browser context (one "internet")
//   mock.drive.addAccount({email}); mock.drive.addFile({...});
//   await mock.install(context);               // routes every non-local request; unknown hosts are aborted
//   mock.inject({ path: /\/files\/abc$/, method: 'PATCH', status: 403, reason: 'userRateLimitExceeded', times: 2 });
//
// Nothing here knows about the app under test: 5.18.0 and v6 get the same mock (TESTING §3.1).
import zlib from 'node:zlib';
import { Drive, driveErrors } from './drive.mjs';
import { GitHub } from './github.mjs';

export const DRIVE_SCOPE = 'https://www.googleapis.com/auth/drive';
const CORS = { 'Access-Control-Allow-Origin': '*', 'Access-Control-Expose-Headers': 'ETag, Location, Range, Content-Range, X-GUploader-UploadID, X-Goog-Hash, Content-Length' };

/** A tiny valid PNG of one colour (avatars, thumbnails, icons). Deterministic bytes. */
export function solidPng(width, height, [r, g, b, a = 255]) {
  const crcTable = solidPng.table ||= Array.from({ length: 256 }, (_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0; });
  const crc = buf => { let c = 0xffffffff; for (const byte of buf) c = crcTable[(c ^ byte) & 255] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; };
  const chunk = (type, data) => { const out = Buffer.alloc(12 + data.length); out.writeUInt32BE(data.length, 0); out.write(type, 4, 'latin1'); data.copy(out, 8); out.writeUInt32BE(crc(out.subarray(4, 8 + data.length)), 8 + data.length); return out; };
  const head = Buffer.alloc(13); head.writeUInt32BE(width, 0); head.writeUInt32BE(height, 4); head[8] = 8; head[9] = 6;
  const row = Buffer.alloc(1 + width * 4); for (let x = 0; x < width; x++) row.set([r, g, b, a], 1 + x * 4);
  const raw = Buffer.concat(Array.from({ length: height }, () => row));
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', head), chunk('IDAT', zlib.deflateSync(raw, { level: 9 })), chunk('IEND', Buffer.alloc(0))]);
}

// Served as https://accounts.google.com/gsi/client. The token comes from the mock at click time, so a test can switch
// `mock.gis.mode` between 'grant', 'deny' (access_denied), 'closed' (popup closed) and 'silent-fail' (prompt=none fails).
const GIS_STUB = `(() => {
  const ask = (client, config, overrides) => {
    const params = new URLSearchParams({ client_id: config.client_id || '', scope: overrides.scope || config.scope || '', prompt: overrides.prompt ?? config.prompt ?? '', login_hint: overrides.login_hint || overrides.hint || config.login_hint || config.hint || '' });
    fetch('https://accounts.google.com/__mock/token?' + params, { cache: 'no-store' }).then(r => r.json()).then(reply => {
      if (reply.popup) { if (typeof client.error_callback === 'function') client.error_callback({ type: reply.popup, message: reply.popup }); return; }
      if (typeof client.callback === 'function') client.callback(reply);
    }, () => { if (typeof client.error_callback === 'function') client.error_callback({ type: 'unknown', message: 'network' }); });
  };
  const oauth2 = {
    initTokenClient(config) {
      const client = { callback: config.callback, error_callback: config.error_callback,
        requestAccessToken(overrides) { ask(client, config, overrides || {}); } };
      return client;
    },
    initCodeClient(config) { return { requestCode() {} }; },
    hasGrantedAllScopes: (response, ...scopes) => scopes.every(s => String(response && response.scope || '').split(/\\s+/).includes(s)),
    hasGrantedAnyScope: (response, ...scopes) => scopes.some(s => String(response && response.scope || '').split(/\\s+/).includes(s)),
    revoke(token, done) { fetch('https://oauth2.googleapis.com/revoke?token=' + encodeURIComponent(token), { method: 'POST' }).then(() => done && done({ successful: true }), () => done && done({ successful: false })); }
  };
  const id = { initialize() {}, prompt() {}, renderButton() {}, disableAutoSelect() {}, cancel() {}, revoke(hint, done) { done && done({ successful: true }); }, storeCredential() {} };
  window.google = Object.assign(window.google || {}, { accounts: { oauth2, id } });
  if (typeof window.onGoogleLibraryLoad === 'function') window.onGoogleLibraryLoad();
})();`;

export function createMock(options = {}) {
  const drive = new Drive(options.drive || {});
  const github = new GitHub(options.github || {});
  const mock = {
    drive, github,
    /** GIS behaviour: account that signs in and how the consent ends. */
    gis: { mode: 'grant', account: null, scope: DRIVE_SCOPE, expiresIn: 3599 },
    /** Authorization codes accepted by the token endpoint (Android PKCE): code → email. */
    codes: new Map(),
    latency: 0,            // ms added to every mocked response (real time)
    offline: false,        // abort every mocked request
    rules: [],
    pending: 0,            // requests received and not yet answered
    activity: 0,           // total requests seen (settle detection)
    calls: [],             // {method, url, host, path, status, bytes, at} of every request that left the app's origin
    unexpected: [],        // requests to hosts the mock does not know (aborted)
    held: [],              // requests parked by a `hold` rule until release()
    gate: false,           // parity harness: answer nothing until the harness calls next() (deterministic I/O order)
    gated: [],             // requests waiting at the gate: {key, fetch, image, resume}

    reset() {
      drive.reset(); github.reset();
      Object.assign(mock.gis, { mode: 'grant', account: null, scope: DRIVE_SCOPE, expiresIn: 3599 });
      mock.codes.clear(); mock.latency = 0; mock.offline = false;
      mock.rules.length = 0; mock.calls.length = 0; mock.unexpected.length = 0; mock.held.length = 0; mock.gated.length = 0; mock.activity = 0;
      return mock;
    },

    /**
     * Failure/latency injection. A rule matches by `method`, `host`, `path` (RegExp or substring of the pathname),
     * `query` (RegExp on the search string) or `when(request)`; `times` limits it (default 1, Infinity allowed).
     * Effect: `status` (+ `reason`, `message`, `body`, `headers`), `abort: true` (network error),
     * `lost: true` (the request is applied, the response is dropped), `delay` ms (real time), `respond(request)`, or
     * `hold: true`: the request stays unanswered (and does not count as pending) until `release()`, then proceeds.
     */
    inject(rule) { const entry = { times: 1, hits: 0, ...rule }; mock.rules.push(entry); return entry; },
    clearInjections() { mock.rules.length = 0; },
    /**
     * With `gate` on, answers exactly one waiting request: the smallest by method, URL and body size, so the order
     * does not depend on which request reached the mock first. Returns false when none waits.
     */
    next() {
      if (!mock.gated.length) return false;
      mock.gated.sort((a, b) => a.key < b.key ? -1 : a.key > b.key ? 1 : a.order - b.order);
      mock.gated.shift().resume();
      return true;
    },
    /**
     * Puts a request of the app's own origin (a font, an image) behind the same gate and then lets it through to the
     * server untouched, so asynchronous same-origin loads are ordered like mocked ones.
     */
    async gateLocal(route, kind) {
      const request = route.request();
      mock.activity++;
      if (mock.gate) await new Promise(resume => mock.gated.push({ key: `${request.method()} ${new URL(request.url()).pathname}`, order: mock.activity, fetch: false, image: kind === 'image', font: kind === 'font', resume }));
      mock.pending++; mock.activity++;
      try { await route.continue(); } catch { /* context closed */ } finally { mock.pending--; mock.activity++; }
    },
    /** Lets every held request continue. */
    release() { for (const resume of mock.held.splice(0)) resume(); },

    /** localStorage entries of a signed-in 5.x session (same keys v6 reads, spec/persistence §1.4). */
    session(email, { expiresInMs = 3600_000, now = Date.now() } = {}) {
      const account = drive.accounts.get(email);
      if (!account) throw new Error(`Unknown mock account ${email}`);
      return {
        'drive-token-v1': JSON.stringify({ token: account.token, expiry: now + expiresInMs, grantedScopes: DRIVE_SCOPE }),
        'drive-remember-v1': '1',
        'drive-hint-email': email,
        'drive-profile-v1': JSON.stringify({ name: account.displayName, email, photo: account.photoLink || undefined })
      };
    },

    /** Resolves when no mocked request is in flight and none arrived for `quietMs` of real time. */
    async idle(quietMs = 30, timeoutMs = 15_000) {
      const start = Date.now();
      for (;;) {
        const seen = mock.activity;
        await new Promise(r => setTimeout(r, quietMs));
        if (!mock.pending && mock.activity === seen) return true;
        if (Date.now() - start > timeoutMs) return false;
      }
    },

    async install(context) {
      await context.route(url => !/^(127\.0\.0\.1|localhost|\[::1\])$/.test(url.hostname) && /^https?:$/.test(url.protocol), route => serve(mock, route));
      return mock;
    }
  };
  return mock;
}

function matches(rule, req) {
  if (rule.times !== Infinity && rule.hits >= rule.times) return false;
  if (rule.method && rule.method.toUpperCase() !== req.method) return false;
  if (rule.host && rule.host !== req.url.hostname) return false;
  if (rule.path && !(rule.path instanceof RegExp ? rule.path.test(req.url.pathname) : req.url.pathname.includes(rule.path))) return false;
  if (rule.query && !rule.query.test(req.url.search)) return false;
  if (rule.when && !rule.when(req)) return false;
  return true;
}

async function serve(mock, route) {
  const request = route.request();
  const req = { method: request.method(), url: new URL(request.url()), headers: request.headers(), body: request.postDataBuffer() || Buffer.alloc(0) };
  const call = { method: req.method, url: request.url(), host: req.url.hostname, path: req.url.pathname, status: 0, bytes: 0, at: Date.now() };
  mock.calls.push(call); mock.pending++; mock.activity++;
  const finish = async action => { try { await action(); } catch { /* page or context already closed */ } finally { mock.pending--; mock.activity++; } };
  const send = out => finish(() => { call.status = out.status || 200; call.bytes = req.body.length + (out.body ? Buffer.byteLength(out.body) : 0); return route.fulfill({ status: out.status || 200, headers: { ...CORS, 'Cache-Control': 'no-store', ...(out.headers || {}) }, body: out.body ?? '' }); });
  const abort = () => finish(() => { call.status = -1; return route.abort('failed'); });
  try {
    if (req.method === 'OPTIONS') return send({ status: 204, headers: { 'Access-Control-Allow-Methods': 'GET, POST, PUT, PATCH, DELETE, OPTIONS', 'Access-Control-Allow-Headers': '*', 'Access-Control-Max-Age': '600' } });
    if (mock.gate) {
      const type = request.resourceType();
      mock.pending--; mock.activity++;
      await new Promise(resume => mock.gated.push({ key: `${req.method} ${req.url.href} ${req.body.length}`, order: mock.activity, fetch: type === 'fetch' || type === 'xhr', image: type === 'image', resume }));
      mock.pending++; mock.activity++;
    }
    if (mock.offline) return abort();
    const rule = mock.rules.find(r => matches(r, req));
    if (rule) rule.hits++;
    const delay = (rule?.delay || 0) + mock.latency;
    if (delay) await new Promise(r => setTimeout(r, delay));
    if (rule?.hold) { mock.pending--; mock.activity++; await new Promise(resume => mock.held.push(resume)); mock.pending++; mock.activity++; }
    if (rule?.abort) return abort();
    if (rule?.respond) return send(await rule.respond(req));
    if (rule?.status) {
      const template = rule.status === 412 ? driveErrors.precondition() : rule.status === 401 ? driveErrors.auth() : rule.status === 403 && /rate/i.test(rule.reason || 'userRateLimitExceeded') ? driveErrors.rate(rule.reason) : null;
      const json = rule.body ?? (template && !rule.message ? template.json : { error: { code: rule.status, message: rule.message || `Injected ${rule.status}`, errors: [{ domain: 'global', reason: rule.reason || 'backendError', message: rule.message || `Injected ${rule.status}` }] } });
      return send({ status: rule.status, headers: { 'Content-Type': 'application/json; charset=UTF-8', ...(rule.headers || {}) }, body: typeof json === 'string' || Buffer.isBuffer(json) ? json : JSON.stringify(json) });
    }
    const out = await answer(mock, req);
    if (rule?.lost) return abort();
    return send(out);
  } catch (error) {
    return send({ status: 500, headers: { 'Content-Type': 'text/plain' }, body: `mock failure: ${error?.stack || error}` });
  }
}

const json = (value, status = 200) => ({ status, headers: { 'Content-Type': 'application/json; charset=UTF-8' }, body: JSON.stringify(value) });

function answer(mock, req) {
  const { drive } = mock, host = req.url.hostname, path = req.url.pathname, q = req.url.searchParams;
  if (host === 'www.googleapis.com') {
    if (path.startsWith('/oauth2/') && path.endsWith('/userinfo')) {
      const account = drive.accounts.get(drive.tokens.get(/^Bearer\s+(.+)$/i.exec(req.headers.authorization || '')?.[1]));
      return account ? json({ sub: account.permissionId, email: account.email, email_verified: true, name: account.displayName, picture: account.photoLink }) : json({ error: 'invalid_token' }, 401);
    }
    return drive.handle(req);
  }
  if (host === 'oauth2.googleapis.com') {
    if (path === '/revoke') { drive.revokeToken(q.get('token') || new URLSearchParams(req.body.toString()).get('token')); return json({}); }
    if (path === '/token') {
      const form = new URLSearchParams(req.body.toString('utf8'));
      let email = null;
      if (form.get('grant_type') === 'authorization_code') { email = mock.codes.get(form.get('code')); mock.codes.delete(form.get('code')); if (!form.get('code_verifier')) email = null; }
      else if (form.get('grant_type') === 'refresh_token') email = drive.refreshTokens.get(form.get('refresh_token'));
      const account = drive.accounts.get(email);
      if (!account) return json({ error: 'invalid_grant', error_description: 'Bad Request' }, 400);
      const reply = { access_token: drive.issueToken(account.email), expires_in: mock.gis.expiresIn, scope: mock.gis.scope, token_type: 'Bearer' };
      if (form.get('grant_type') === 'authorization_code') reply.refresh_token = account.refreshToken;
      return json(reply);
    }
    return json({ error: 'not_found' }, 404);
  }
  if (host === 'accounts.google.com') {
    if (path === '/gsi/client') return mock.gis.mode === 'absent' ? { status: 503, body: '' } : { status: 200, headers: { 'Content-Type': 'text/javascript; charset=utf-8' }, body: GIS_STUB };
    if (path === '/__mock/token') {
      const silent = q.get('prompt') === 'none';
      const account = drive.accounts.get(mock.gis.account || q.get('login_hint')) || (silent ? null : [...drive.accounts.values()][0]);
      if (mock.gis.mode === 'closed' && !silent) return json({ popup: 'popup_closed' });
      if (mock.gis.mode === 'blocked' && !silent) return json({ popup: 'popup_failed_to_open' });
      if (mock.gis.mode === 'deny' || !account) return json({ error: silent ? 'interaction_required' : 'access_denied', error_description: 'mock', scope: '' });
      if (mock.gis.mode === 'silent-fail' && silent) return json({ error: 'interaction_required' });
      return json({ access_token: drive.issueToken(account.email), expires_in: mock.gis.expiresIn, scope: mock.gis.scope, token_type: 'Bearer', prompt: q.get('prompt') || '' });
    }
    return { status: 200, headers: { 'Content-Type': 'text/html; charset=utf-8' }, body: '<!doctype html><title>Mock Google sign-in</title>' };
  }
  if (host === 'drive.usercontent.google.com' || host === 'drive.google.com') {
    const preview = /^\/file\/d\/([^/]+)\//.exec(path);
    const id = q.get('id') || preview?.[1];
    const file = drive.file(id || '');
    const open = file && drive.rank(file, null) >= 1 && !file.trashed && (!file.resourceKey || q.get('resourcekey') === file.resourceKey);
    if (preview || path === '/open') return { status: open ? 200 : 403, headers: { 'Content-Type': 'text/html; charset=utf-8' }, body: `<!doctype html><title>Mock Drive preview</title><body style="margin:0;background:#fff">${open ? 'Mock Drive preview' : 'You need access'}</body>` };
    if (path === '/download' || path === '/uc') {
      // Signed-out downloads work only for "anyone with the link" files; otherwise Google answers with an HTML sign-in page.
      if (!open) return { status: 403, headers: { 'Content-Type': 'text/html; charset=utf-8' }, body: '<!doctype html><title>Sign in</title>' };
      const head = file.revisions.at(-1);
      return { status: 200, headers: { 'Content-Type': head.mimeType || 'application/octet-stream', 'Content-Disposition': `attachment; filename="${file.name.replace(/"/g, '')}"` }, body: head.data };
    }
    return { status: 404, headers: { 'Content-Type': 'text/html' }, body: '' };
  }
  if (host.endsWith('.googleusercontent.com')) {
    // Avatars and Drive thumbnails: one flat colour derived from the URL, so both apps get the same bytes.
    let hash = 0; for (const c of path) hash = (hash * 31 + c.charCodeAt(0)) >>> 0;
    return { status: 200, headers: { 'Content-Type': 'image/png' }, body: solidPng(8, 8, [80 + (hash & 127), 80 + ((hash >> 8) & 127), 80 + ((hash >> 16) & 127)]) };
  }
  if (host === 'fonts.googleapis.com') return { status: 200, headers: { 'Content-Type': 'text/css; charset=utf-8' }, body: '/* mock: fonts are vendored */' };
  if (host === 'fonts.gstatic.com' || host === 'unpkg.com' || host === 'cdnjs.cloudflare.com') return { status: 404, headers: { 'Content-Type': 'text/plain' }, body: '' };
  if (host === 'raw.githubusercontent.com' || host === 'api.github.com' || host === 'github.com' || host.endsWith('.githubusercontent.com')) return mock.github.handle(req);
  mock.unexpected.push(`${req.method} ${req.url}`);
  return { status: 599, headers: { 'Content-Type': 'text/plain' }, body: 'unmocked host' };
}
