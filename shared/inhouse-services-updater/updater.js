/* Inhouse services updater — canonical source: github.com/miguelcoxcaballero/inhouse-services-updater.
 * Edit here, sync the SAME revision into YouTube, Instagram, Notes, Photos and Read.
 * Every new release goes to A with a tiny changelog. Publish B ONLY on the user's explicit request.
 * B is the user default; A needs warning acknowledgement and a slide confirmation.
 * Updates are optional. Later defers automatic prompts for 14 days; settings always allow a manual check.
 */
(function (root) {
  'use strict';
  const VERSION = '1.0.0', TWO_WEEKS = 14 * 86400000;
  const compareVersions = (a, b) => {
    const x = String(a).split('.').map(Number), y = String(b).split('.').map(Number);
    for (let i = 0; i < Math.max(x.length, y.length); i++) { const d = (x[i] || 0) - (y[i] || 0); if (d) return Math.sign(d); }
    return 0;
  };
  function validateCatalog(value, channel, config) {
    if (!value || value.schemaVersion !== 1 || value.channel !== channel || !Array.isArray(value.releases)) throw Error('Invalid update catalog.');
    function release(item) {
      if (!item || !/^\d+\.\d+\.\d+$/.test(item.version) || !Number.isSafeInteger(item.versionCode) || item.versionCode < 1) throw Error('Invalid version.');
      const url = new URL(item.apkUrl);
      if (url.protocol !== 'https:' || url.hostname !== 'github.com' || !url.pathname.startsWith('/' + config.repository + '/releases/download/') || !url.pathname.endsWith('.apk') || url.username || url.password) throw Error('Invalid download.');
      if (!/^[a-f0-9]{64}$/i.test(item.apkSha256) || !Number.isSafeInteger(item.apkSizeBytes) || item.apkSizeBytes < 1) throw Error('Invalid download integrity.');
      if (typeof item.changelog !== 'string' || !item.changelog.trim() || item.changelog.length > 120) throw Error('Keep the changelog short.');
      return { ...item, apkUrl: url.href, channel };
    }
    const releases = value.releases.map(release).sort((a, b) => b.versionCode - a.versionCode);
    if (new Set(releases.map(r => r.versionCode)).size !== releases.length) throw Error('Duplicate versions.');
    const latest = value.latest == null ? null : release(value.latest);
    if (latest && (!releases.length || latest.versionCode !== releases[0].versionCode || latest.apkSha256 !== releases[0].apkSha256)) throw Error('Inconsistent update catalog.');
    if (!latest && releases.length) throw Error('Missing latest version.');
    return { ...value, latest, releases };
  }
  function newer(release, installed, code) {
    if (!release) return false;
    const comparison = compareVersions(release.version, installed);
    return comparison >= 0 && (code > 0 ? release.versionCode > code : comparison > 0);
  }
  function create(config) {
    const win = config.window || root, doc = win.document, native = () => win.InhouseNative;
    const key = 'inhouse-services-updater:' + config.appId;
    let state = { channel: 'B', developmentConsent: 0, snooze: {} }, dialog = null, available = null, checking = null, generation = 0;
    let lastPrompt = '', timer = null, started = false, installing = false, previousFocus = null, settingsContainer = null;
    const now = () => config.now ? config.now() : Date.now();
    const installed = () => String(config.getVersion?.() || native()?.getAppVersion?.() || config.installedVersion || '0.0.0');
    const installedCode = () => Number(config.getVersionCode?.() || native()?.getAppVersionCode?.() || config.installedVersionCode || 0);
    function readState() {
      try {
        const raw = native()?.getUpdateState ? native().getUpdateState() : win.localStorage.getItem(key);
        const value = raw ? JSON.parse(raw) : {};
        state = { channel: value.channel === 'A' && value.developmentConsent === 1 ? 'A' : 'B', developmentConsent: value.channel === 'A' && value.developmentConsent === 1 ? 1 : 0, snooze: value.snooze && typeof value.snooze === 'object' ? value.snooze : {} };
      } catch (_) { /* Memory state still permits manual updating when storage is unavailable. */ }
      return state;
    }
    function saveState() {
      const json = JSON.stringify(state);
      if (native()?.saveUpdateState) native().saveUpdateState(json);
      try { win.localStorage.setItem(key, json); } catch (_) { /* Native persistence is authoritative in APK hosts. */ }
    }
    function snoozed() { readState(); return Number(state.snooze[state.channel] || 0) > now(); }
    async function freshJson(url) {
      const controller = new AbortController(), timeout = win.setTimeout(() => controller.abort(), 12000);
      try {
        const request = new URL(url); request.searchParams.set('check', String(now()));
        const response = await (config.fetch || win.fetch.bind(win))(request.href, { cache: 'no-store', signal: controller.signal });
        if (!response.ok) throw Error('HTTP ' + response.status);
        return await response.json();
      } finally { win.clearTimeout(timeout); }
    }
    async function catalog(channel) {
      const path = 'updates/' + channel + '.json';
      let first;
      try {
        first = validateCatalog(await freshJson('https://raw.githubusercontent.com/' + config.repository + '/main/' + path), channel, config);
        if (newer(first.latest, installed(), installedCode())) return first;
      } catch (_) { /* A release can precede raw content propagation. Recover within the same channel only. */ }
      try {
        const result = await freshJson('https://api.github.com/repos/' + config.repository + '/contents/' + path + '?ref=main');
        const recovered = validateCatalog(JSON.parse(win.atob(result.content.replace(/\s/g, ''))), channel, config);
        if (!first || (recovered.latest?.versionCode || 0) >= (first.latest?.versionCode || 0)) return recovered;
      } catch (error) { if (!first) throw error; }
      return first;
    }
    function close(defer = false) {
      generation++; checking = null;
      if (defer) { readState(); state.snooze[state.channel] = now() + TWO_WEEKS; saveState(); lastPrompt = ""; }
      if (dialog) { dialog.remove(); dialog = null; previousFocus?.focus?.(); }
      settingsContainer = null;
      if (config.close) config.close();
    }
    function shell(title) {
      if (!dialog) {
        previousFocus = doc.activeElement;
        dialog = doc.createElement('div'); dialog.id = 'inhouse-services-update'; dialog.className = settingsContainer ? 'isu-inline' : 'isu-overlay';
        dialog.setAttribute('role', settingsContainer ? 'region' : 'dialog');
        if (!settingsContainer) dialog.setAttribute('aria-modal', 'true');
        dialog.setAttribute('aria-labelledby', 'isu-title');
        dialog.innerHTML = '<section class="isu-card"><header><div><small></small><h2 id="isu-title"></h2></div></header><div data-isu-body></div><p class="isu-status" data-isu-status role="status"></p></section>';
        dialog.addEventListener('keydown', event => {
          if (event.key === 'Escape' && !installing) { event.preventDefault(); event.stopPropagation(); close(!!available); }
          if (event.key === 'Tab' && !settingsContainer) {
            const buttons = [...dialog.querySelectorAll('button:not(:disabled),a[href],input:not(:disabled),[tabindex="0"]')];
            const first = buttons[0], last = buttons[buttons.length - 1];
            if (event.shiftKey && doc.activeElement === first) { event.preventDefault(); last?.focus(); }
            else if (!event.shiftKey && doc.activeElement === last) { event.preventDefault(); first?.focus(); }
          }
        });
        (settingsContainer || doc.body).append(dialog);
      }
      dialog.querySelector('h2').textContent = title;
      dialog.querySelector('header small').textContent = config.appName;
      const body = dialog.querySelector('[data-isu-body]'); body.replaceChildren(); status('');
      return body;
    }
    function el(tag, text, className) { const value = doc.createElement(tag); if (text != null) value.textContent = text; if (className) value.className = className; return value; }
    function button(parent, text, action, className = 'isu-secondary') {
      const value = el('button', text, className); value.type = 'button'; value.onclick = action; parent.append(value); return value;
    }
    function status(text) { if (dialog) dialog.querySelector('[data-isu-status]').textContent = text; }
    // B is the default. Legacy implicit A preferences require renewed, explicit consent.
    function selectChannel(channel) {
      if (installing) return;
      readState(); state.channel = channel; state.developmentConsent = channel === 'A' ? 1 : 0;
      saveState(); generation++; checking = null; available = null; lastPrompt = '';
      settings('Version ' + channel + ' selected.'); check({ manual: true });
    }
    function footer(body) {
      const value = el('footer', null, 'isu-footer');
      const brand = el('a', null, 'isu-brand'); brand.href = 'https://inhouse.miguelcoxcaballero.com/';
      brand.innerHTML = '<span class="isu-mark" aria-hidden="true"><svg viewBox="0 0 40 24"><path d="M4 22L20 6L36 22"/></svg></span><span>Made by Inhouse</span>';
      const github = el('a', 'GitHub', 'isu-github'); github.href = 'https://github.com/' + config.repository;
      // Native hosts open these allowlisted links in the browser, outside the installer bridge.
      value.append(brand, github); body.append(value);
    }
    function confirmDevelopment() {
      // Opening or cancelling this screen never opts in or requests an A catalog.
      generation++; checking = null; available = null;
      const body = shell('Use version A?');
      body.append(el('p', 'Development builds', 'isu-warning-title'));
      body.append(el('p', 'Version A contains untested development updates. Bugs, instability or data loss are possible. Reviewed updates are released on version B.', 'isu-warning'));
      let agreed = false;
      const yes = button(body, 'I understand — continue', () => {
        agreed = true; yes.hidden = true; track.hidden = false; slider.focus();
      }, 'isu-primary');
      const track = el('div', null, 'isu-slide'); track.hidden = true;
      const label = el('label', 'Slide to confirm version A'); label.htmlFor = 'isu-confirm-a';
      const slider = el('input'); slider.id = 'isu-confirm-a'; slider.type = 'range'; slider.min = '0'; slider.max = '100'; slider.step = '1'; slider.value = '0';
      slider.setAttribute('aria-label', 'Slide to confirm version A');
      slider.onchange = () => { if (agreed && Number(slider.value) === 100) selectChannel('A'); else slider.value = '0'; };
      track.append(label, slider); body.append(track);
      button(body, 'Keep version B', () => settings(), 'isu-secondary'); footer(body); yes.focus();
    }
    function settings(message = '', presentation = {}) {
      if (presentation.container && presentation.container !== settingsContainer) {
        if (dialog) close();
        settingsContainer = presentation.container;
      }
      readState(); const body = shell('Updates');
      body.append(el('p', 'Installed version ' + installed(), 'isu-app'));
      const heading = el('h3', 'Update channel', 'isu-section-title'); body.append(heading);
      body.append(el('p', 'B: reviewed releases. A: development builds.', 'isu-caption'));
      const options = el('div', null, 'isu-channels'); options.setAttribute('role', 'group'); options.setAttribute('aria-label', 'Update channel'); body.append(options);
      ['B', 'A'].forEach(channel => {
        const value = button(options, 'Version ' + channel, () => {
          if (installing || state.channel === channel) return;
          if (channel === 'A') confirmDevelopment(); else selectChannel('B');
        });
        value.setAttribute('aria-pressed', String(state.channel === channel));
      });
      button(body, 'Check for updates', () => check({ manual: true }), 'isu-primary');
      button(body, 'Done', () => close(false), 'isu-text'); footer(body);
      status(message); options.querySelector('button[aria-pressed="true"]').focus();
    }
    function progress(payload) {
      if (!dialog) return;
      const progress = dialog.querySelector('[data-isu-progress]'); if (!progress) return;
      const total = Number(payload.totalBytes || available?.latest.apkSizeBytes || 0), downloaded = Math.max(0, Number(payload.downloadedBytes) || 0);
      const percent = Math.max(0, Math.min(100, payload.percent >= 0 ? Math.round(payload.percent) : total > 0 ? Math.round(downloaded * 100 / total) : 0));
      progress.hidden = false; progress.querySelector('progress').value = percent;
      progress.querySelector('span').textContent = percent + '% · ' + (downloaded / 1048576).toFixed(1) + (total ? ' / ' + (total / 1048576).toFixed(1) : '') + ' MB';
    }
    function result(payload) {
      if (typeof payload === 'string') { try { payload = JSON.parse(payload); } catch (_) { return; } }
      if (!dialog || !payload) return;
      const install = dialog.querySelector('[data-isu-install]'); if (!install) return;
      if (payload.status === 'downloading') { installing = true; install.disabled = true; progress(payload); status(payload.message || 'Downloading…'); }
      else if (payload.status === 'ready') { progress({ ...payload, percent: 100 }); status('Confirm installation in Android.'); installing = false; }
      else if (payload.status === 'permission_required') { installing = false; install.disabled = false; install.textContent = 'Continue'; status('Allow installs from this app, then tap Continue.'); }
      else if (payload.status === 'error') { installing = false; install.disabled = false; install.textContent = 'Retry'; status(payload.message || 'Download failed. Try again.'); }
      dialog.querySelectorAll('[data-isu-idle]').forEach(b => { b.disabled = installing; });
    }
    function show(value) {
      available = value;
      const release = value.latest, body = shell('Update available');
      body.append(el('p', config.appName, 'isu-app'));
      body.append(el('p', installed() + ' → ' + release.version + ' ' + state.channel, 'isu-versions'));
      const skipped = value.releases.filter(r => r.versionCode < release.versionCode && newer(r, installed(), installedCode()));
      if (skipped.length) body.append(el('p', 'Skipped versions: ' + skipped.slice(0, 3).reverse().map(r => r.version).join(' · ') + (skipped.length > 3 ? ' (+' + (skipped.length - 3) + ')' : ''), 'isu-caption'));
      const changes = value.releases.filter(r => newer(r, installed(), installedCode()));
      const list = el('ul', null, 'isu-changes'); changes.slice(0, 3).forEach(r => list.append(el('li', r.version + ' · ' + r.changelog))); body.append(list);
      if (changes.length > 3) body.append(el('p', '+' + (changes.length - 3) + ' earlier versions', 'isu-caption'));
      const bar = el('div', null, 'isu-progress'); bar.hidden = true; bar.dataset.isuProgress = '';
      const meter = doc.createElement('progress'); meter.max = 100; meter.value = 0; meter.setAttribute('aria-label', 'Download'); bar.append(el('span', '0%'), meter); body.append(bar);
      const install = button(body, 'Update', () => {
        installing = true; install.disabled = true; dialog.querySelectorAll('[data-isu-idle]').forEach(b => { b.disabled = true; });
        status('Preparing…'); progress({ percent: 0 });
        try {
          if (config.install) config.install(release);
          else if (native()?.installInhouseUpdate) native().installInhouseUpdate(release.apkUrl, release.apkSha256);
          else if (native()?.installAppUpdate) native().installAppUpdate(release.apkUrl, release.apkSha256);
          else throw Error('Open the Android app to install.');
        } catch (error) { result({ status: 'error', message: error.message }); }
      }, 'isu-primary'); install.dataset.isuInstall = '';
      const later = button(body, 'Remind me later', () => close(true)); later.dataset.isuIdle = '';
      body.append(el('p', 'We’ll remind you in 14 days.', 'isu-caption isu-centered'));
      const change = button(body, 'Version ' + state.channel + ' · Change', () => settings(), 'isu-text'); change.dataset.isuIdle = '';
      install.focus();
    }
    async function check({ manual = false } = {}) {
      if (installing || (!manual && snoozed())) return false;
      if (checking) {
        const token = generation, pending = checking;
        if (manual) status('Checking…');
        const found = await pending;
        if (manual && token === generation) {
          if (available && found) show(available);
          else status(available ? catalogStatus(available) : 'Could not check for updates. Try again.');
        }
        return found;
      }
      readState(); const channel = state.channel, token = generation;
      if (manual && !dialog) settings(); if (manual) status('Checking…');
      const pending = (async () => {
        try {
          const value = await catalog(channel);
          readState(); if (token !== generation || state.channel !== channel) return false;
          available = value;
          if (newer(value.latest, installed(), installedCode())) {
            const id = channel + ':' + value.latest.versionCode + ':' + (state.snooze[channel] || 0);
            if (manual || (!snoozed() && id !== lastPrompt)) {
              if (!manual) lastPrompt = id;
              if (!manual && config.onOffer) config.onOffer(value); else show(value);
            }
            return true;
          }
          if (manual) status(catalogStatus(value));
          return false;
        } catch (_) { if (manual) status('Could not check for updates. Try again.'); return false; }
      })();
      checking = pending;
      try { return await pending; } finally { if (checking === pending) checking = null; }
    }
    function catalogStatus(value) {
      return !value.latest ? 'No releases on version ' + value.channel + '.' : compareVersions(installed(), value.latest.version) > 0 ? 'Keep your installed build until version ' + value.channel + ' catches up.' : 'You’re up to date on version ' + value.channel + '.';
    }
    function start() {
      if (started) return; started = true; readState();
      win.handleInhouseUpdateResult = result;
      win.addEventListener('focus', automatic); timer = win.setInterval(automatic, 15 * 60000); automatic();
    }
    function automatic() { check(); }
    function stop() { win.removeEventListener('focus', automatic); win.clearInterval(timer); started = false; }
    function offerPending(value) {
      readState(); const valid = validateCatalog(value, state.channel, config);
      if (!snoozed() && newer(valid.latest, installed(), installedCode())) { win.handleInhouseUpdateResult = result; show(valid); return true; }
      return false;
    }
    win.handleInhouseUpdateResult = result;
    return { start, stop, check, settings, close, result, offerPending, getState: readState, version: VERSION };
  }
  const api = { create, compareVersions, validateCatalog, newer, TWO_WEEKS, VERSION };
  root.InhouseServicesUpdater = api;
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
})(typeof window !== 'undefined' ? window : globalThis);
