/* Canonical Inhouse Google OAuth. Derived from Notes app-v5.js §4.5.
 * Edit in inhouse-services-updater, then sync; consumers only supply host configuration.
 */
(function (root) {
  'use strict';
  const ENDPOINT = 'https://oauth2.googleapis.com/token';
  const AUTH = 'https://accounts.google.com/o/oauth2/v2/auth';
  const TTL = 10 * 60 * 1000;
  const encode = bytes => btoa(String.fromCharCode(...bytes)).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
  const random = () => encode(root.crypto.getRandomValues(new Uint8Array(32)));
  const scopes = value => new Set(String(value || '').split(/\s+/).filter(Boolean));
  function create(config) {
    const storage = config.storage || root.localStorage;
    const key = config.sessionKey || 'inhouse-google-session-v1';
    const transactionKey = config.transactionKey || key + ':transaction';
    let generation = 0, pending = null, flight = null, exchange = null, signedOut = false;
    const read = name => { try { return JSON.parse(storage.getItem(name) || 'null'); } catch (_) { return null; } };
    const remove = name => storage.removeItem(name);
    let nativeRestore = null, nativeRestored = false;
    const nativeCalls = new Map();
    function nativeCall(method, session) {
      const bridge = root.InhouseOAuthNative;
      if (!bridge) return Promise.resolve(null);
      bridge.onmessage = event => {
        let data; try { data = JSON.parse(event.data); } catch (_) { return; }
        const call = nativeCalls.get(data.id); if (!call) return;
        nativeCalls.delete(data.id); clearTimeout(call.timer);
        // An absent/old provider is normal on the first installed app.
        call.resolve(data.session || null);
      };
      return new Promise(resolve => {
        const id = random();
        const timer = setTimeout(() => { nativeCalls.delete(id); resolve(null); }, 3000);
        nativeCalls.set(id, { resolve, timer });
        try { bridge.postMessage(JSON.stringify({ id, method, session })); }
        catch (_) { clearTimeout(timer); nativeCalls.delete(id); resolve(null); }
      });
    }
    const cancelled = () => new Error('Google sign-in was cancelled');
    function check(version) { if (version !== generation || signedOut) throw cancelled(); }
    function accepts(scope) {
      const granted = scopes(scope);
      return [...scopes(config.scope)].every(item => granted.has(item) || (item === 'https://www.googleapis.com/auth/drive.file' && granted.has('https://www.googleapis.com/auth/drive')));
    }
    function save(data, version, previous = read(key)) {
      check(version);
      const scope = data.scope || previous?.scope;
      if (!data.access_token || !accepts(scope)) throw new Error('Required Google permissions were not granted');
      const seconds = Number(data.expires_in);
      if (!Number.isFinite(seconds) || seconds <= 0) throw new Error('Invalid Google token lifetime');
      const session = { ...data, scope, expiresAt: Date.now() + seconds * 1000,
        refresh_token: data.refresh_token || previous?.refresh_token || '',
        client_id: previous?.client_id || config.androidClientId || config.clientId,
        email: data.email || previous?.email || config.hint?.() || '', updatedAt: Date.now() };
      remove(key + ':signed-out');
      storage.setItem(key, JSON.stringify(session));
      if (config.refreshKey && session.refresh_token) storage.setItem(config.refreshKey, session.refresh_token);
      config.onToken?.(session);
      if (root.InhouseOAuthNative) {
        void nativeCall('write', session);
        // Store the account hint with the shared grant so extra scopes use the same account.
        if (!session.email) void (config.fetch || root.fetch)('https://www.googleapis.com/drive/v3/about?fields=user(emailAddress)', {
          headers: { Authorization: 'Bearer ' + session.access_token }
        }).then(response => response.ok ? response.json() : null).then(profile => {
          if (version !== generation || signedOut || read(key)?.access_token !== session.access_token) return;
          if (profile?.user?.emailAddress) {
            session.email = profile.user.emailAddress; storage.setItem(key, JSON.stringify(session));
            void nativeCall('write', session);
          }
        }).catch(() => {});
      }
      return session;
    }
    async function tokenRequest(fields, version, previous) {
      const controller = new AbortController();
      const timer = setTimeout(() => controller.abort(), 20000);
      try {
        const response = await (config.fetch || root.fetch)(ENDPOINT, { method: 'POST',
          headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
          body: new URLSearchParams(fields).toString(), signal: controller.signal });
        const data = await response.json();
        check(version);
        if (!response.ok) {
          // A temporary network/server failure must not destroy an offline grant.
          if (data.error === 'invalid_grant') {
            remove(key); storage.setItem(key + ':signed-out', '1'); void nativeCall('clear'); if (config.refreshKey) remove(config.refreshKey);
          }
          const error = new Error('Google authorization failed');
          error.code = data.error; throw error;
        }
        return save(data, version, previous);
      } finally { clearTimeout(timer); }
    }
    function waitForCallback(start, timeout, version) {
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => { remove(transactionKey); finish(new Error('Google sign-in timed out')); }, timeout);
        function finish(error, data) {
          if (pending?.finish !== finish) return;
          pending = null; clearTimeout(timer);
          try { check(version); if (error) throw error; resolve(data); } catch (err) { reject(err); }
        }
        pending = { finish };
        try { Promise.resolve(start(finish)).catch(error => finish(error)); } catch (error) { finish(error); }
      });
    }
    function web(interactive, version) {
      const oauth = root.google?.accounts?.oauth2;
      if (!oauth) return Promise.reject(new Error('Google sign-in is still loading'));
      // Called synchronously from the click so popup blockers retain user activation.
      return waitForCallback(finish => {
        const client = oauth.initTokenClient({ client_id: config.clientId, scope: config.scope,
          include_granted_scopes: true,
          callback: data => {
            if (pending?.finish !== finish) return;
            try {
              if (data.error) throw new Error(data.error);
              finish(null, save(data, version));
            } catch (error) { finish(error); }
          }, error_callback: error => finish(new Error(error.type || 'Google popup failed')) });
        client.requestAccessToken({ prompt: interactive ? '' : 'none',
          ...(config.hint?.() ? { login_hint: config.hint() } : {}) });
      }, interactive ? 60000 : 9000, version);
    }
    function native(interactive, version) {
      const previous = read(key);
      const refresh = previous ? previous.refresh_token : (config.refreshKey && storage.getItem(config.refreshKey));
      if (refresh && accepts(previous?.scope || config.scope)) {
        return tokenRequest({ client_id: previous?.client_id || config.androidClientId, grant_type: 'refresh_token', refresh_token: refresh },
          version, previous || { refresh_token: refresh, scope: config.scope }).catch(error => {
          if (interactive && error.code === 'invalid_grant') return native(true, version);
          throw error;
        });
      }
      if (!interactive) return Promise.reject(new Error('Google sign-in is required'));
      return waitForCallback(async () => {
        const verifier = random(), state = (config.statePrefix || '') + random();
        const challenge = encode(new Uint8Array(await root.crypto.subtle.digest('SHA-256', new TextEncoder().encode(verifier))));
        check(version);
        storage.setItem(transactionKey, JSON.stringify({ verifier, state, createdAt: Date.now() }));
        const params = new URLSearchParams({ client_id: config.androidClientId || config.clientId,
          redirect_uri: config.redirectUri, response_type: config.androidClientId ? 'code' : 'token',
          scope: config.scope, include_granted_scopes: 'true', state });
        if (config.androidClientId) {
          params.set('access_type', 'offline'); params.set('code_challenge', challenge);
          params.set('code_challenge_method', 'S256'); params.set('prompt', 'consent');
        }
        const hint = previous?.email || config.hint?.(); if (hint) params.set('login_hint', hint);
        config.openAuthUrl(AUTH + '?' + params);
      }, TTL, version);
    }
    function request(interactive = false) {
      if (root.InhouseOAuthNative && !nativeRestored && storage.getItem(key + ':signed-out') !== '1') {
        if (!nativeRestore) {
          const version = generation;
          nativeRestore = nativeCall('read').then(session => {
            check(version);
            // Never switch an existing app's account during a background refresh.
            const ownsGrant = read(key) || (config.refreshKey && storage.getItem(config.refreshKey));
            if (session && !ownsGrant) storage.setItem(key, JSON.stringify(session));
          }).finally(() => { nativeRestored = true; });
        }
        return nativeRestore.then(() => request(interactive));
      }
      if (storage.getItem(key + ':signed-out') === '1') signedOut = true;
      if (signedOut && !interactive) return Promise.reject(cancelled());
      if (interactive) { signedOut = false; remove(key + ':signed-out'); }
      const session = read(key);
      if (session && session.expiresAt > Date.now() + 60000 && accepts(session.scope)) {
        config.onToken?.(session); return Promise.resolve(session);
      }
      if (flight) return flight;
      const version = generation;
      const promise = config.openAuthUrl ? native(interactive, version) : web(interactive, version);
      flight = promise;
      const clear = () => { if (flight === promise) flight = null; };
      promise.then(clear, clear);
      return promise;
    }
    async function handleCallback(payload) {
      if (exchange) return exchange;
      const transaction = read(transactionKey);
      if (transaction && !transaction.verifier && config.verifierKey) transaction.verifier = storage.getItem(config.verifierKey);
      const params = new URLSearchParams(String(payload || '').replace(/^[?#]/, ''));
      if (!transaction || params.get('state') !== transaction.state) return false;
      remove(transactionKey); if (config.verifierKey) remove(config.verifierKey); // Consume once, including denial and expired callbacks.
      const version = generation;
      const task = (async () => {
        if (Date.now() - transaction.createdAt > TTL || transaction.createdAt > Date.now()) throw new Error('Google sign-in expired');
        if (params.has('error')) throw new Error('Google sign-in was denied');
        if (config.androidClientId) {
          if (!params.get('code')) throw new Error('Missing Google authorization code');
          return tokenRequest({ client_id: config.androidClientId, code: params.get('code'),
            code_verifier: transaction.verifier, grant_type: 'authorization_code', redirect_uri: config.redirectUri }, version, null);
        }
        return save(Object.fromEntries(params), version, null);
      })();
      exchange = task;
      try { const data = await task; pending?.finish(null, data); return data; }
      catch (error) { pending?.finish(error); throw error; }
      finally { if (exchange === task) exchange = null; }
    }
    function cancel() {
      pending?.finish(cancelled()); generation++; pending = null; flight = null; exchange = null; remove(transactionKey);
    }
    function signOut() {
      cancel(); signedOut = true; storage.setItem(key + ':signed-out', '1'); void nativeCall('clear'); remove(key); if (config.refreshKey) remove(config.refreshKey);
      // Local sign-out must not revoke the Google grant used by the other Inhouse apps.
    }
    function invalidate() {
      const session = read(key);
      if (session) { session.expiresAt = 0; storage.setItem(key, JSON.stringify(session)); }
    }
    return { request, handleCallback, cancel, signOut, invalidate, hasPendingRequest: () => !!pending, session: () => read(key) };
  }
  root.InhouseGoogleOAuth = Object.freeze({ create });
  if (typeof module !== 'undefined') module.exports = root.InhouseGoogleOAuth;
})(globalThis);
