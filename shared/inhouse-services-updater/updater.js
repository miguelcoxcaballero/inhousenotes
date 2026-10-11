/* Inhouse services updater — canonical source: github.com/miguelcoxcaballero/inhouse-services-updater.
 * Edit here, sync the SAME revision into YouTube, Instagram, Notes, Photos and Read.
 * Every new release goes to A with a tiny changelog. Publish B ONLY on the user's explicit request.
 * B is the user default; A needs warning acknowledgement and a slide confirmation.
 * Updates are optional. Later defers automatic prompts for 14 days; settings always allow a manual check.
 */
(function (root) {
  'use strict';
  const VERSION = '1.2.3', TWO_WEEKS = 14 * 86400000;
  const compareVersions = (a, b) => {
    const x = String(a).split('.').map(Number), y = String(b).split('.').map(Number);
    for (let i = 0; i < Math.max(x.length, y.length); i++) { const d = (x[i] || 0) - (y[i] || 0); if (d) return Math.sign(d); }
    return 0;
  };
  function validatedApkUrl(value, config) {
    const url = new URL(value);
    if (url.protocol !== 'https:' || url.hostname !== 'github.com' || !url.pathname.startsWith('/' + config.repository + '/releases/download/') || !url.pathname.endsWith('.apk') || url.username || url.password) throw Error('Invalid download.');
    return url.href;
  }
  function releasePageUrl(value, config) {
    const url = new URL(validatedApkUrl(value, config));
    const path = url.pathname.slice(('/' + config.repository + '/releases/download/').length).split('/');
    if (path.length !== 2 || !path[0] || !path[1]) throw Error('Invalid release download.');
    // Keep the tag's URL encoding intact (including encoded slashes). Never use
    // /latest, the binary URL, or an unvalidated link from a native result.
    return 'https://github.com/' + config.repository + '/releases/tag/' + path[0];
  }
  function validateCatalog(value, channel, config) {
    if (!value || value.schemaVersion !== 1 || value.channel !== channel || !Array.isArray(value.releases)) throw Error('Invalid update catalog.');
    function release(item) {
      if (!item || !/^\d+\.\d+\.\d+$/.test(item.version) || !Number.isSafeInteger(item.versionCode) || item.versionCode < 1) throw Error('Invalid version.');
      const apkUrl = validatedApkUrl(item.apkUrl, config);
      if (!/^[a-f0-9]{64}$/i.test(item.apkSha256) || !Number.isSafeInteger(item.apkSizeBytes) || item.apkSizeBytes < 1) throw Error('Invalid download integrity.');
      if (typeof item.changelog !== 'string' || !item.changelog.trim() || item.changelog.length > 120) throw Error('Keep the changelog short.');
      return { ...item, apkUrl, channel };
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
    let statusNode = null, transition = null, closeTimer = null;
    const installKey = key + ':installation', seenInstallKey = key + ':installation-seen';
    function savedInstall() {
      try { return JSON.parse(win.localStorage.getItem(installKey) || 'null'); } catch (_) { return null; }
    }
    function saveInstall(value) {
      try { if (value) win.localStorage.setItem(installKey, JSON.stringify(value)); else win.localStorage.removeItem(installKey); } catch (_) {}
    }
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
    // Presentation follows Material Design 3: a modal bottom sheet on phones and a
    // basic dialog on wide screens, flat surfaces, text/filled buttons aligned to
    // the end, list items for settings. Secondary facts live behind "Details".
    const ICONS = {
      expand: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7 10l5 5 5-5"/></svg>',
      arrow: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5 12h14M13 6l6 6-6 6"/></svg>',
      warning: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 4.5 2.8 20h18.4z"/><path d="M12 10v4.4M12 17.3v.1"/></svg>',
      done: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5 12.5l4.5 4.5L19 7.5"/></svg>',
      error: '<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="9"/><path d="M12 7.5v5.5M12 16.4v.1"/></svg>',
    };
    const LEAVE_MS = 220;
    const motion = () => typeof win.matchMedia === 'function' && !win.matchMedia('(prefers-reduced-motion: reduce)').matches;
    function close(defer = false) {
      generation++; checking = null;
      if (defer) { readState(); state.snooze[state.channel] = now() + TWO_WEEKS; saveState(); lastPrompt = ""; }
      const node = dialog, inline = !!settingsContainer, animated = !!node && !inline && motion();
      dialog = null; statusNode = null; settingsContainer = null;
      if (node) { leave(node, animated); previousFocus?.focus?.(); }
      if (!config.close) return;
      // Native hosts finish their Activity on close; let the sheet leave first.
      if (animated) closeTimer = win.setTimeout(() => { closeTimer = null; config.close(); }, LEAVE_MS);
      else config.close();
    }
    function leave(node, animated) {
      ['id', 'role', 'aria-modal', 'aria-labelledby'].forEach(name => node.removeAttribute(name));
      node.querySelectorAll('[id]').forEach(child => child.removeAttribute('id'));
      node.setAttribute('aria-hidden', 'true'); node.inert = true;
      if (!animated) { node.remove(); return; }
      const card = node.querySelector('.isu-card');
      card.classList.remove('isu-dragging'); card.style.transform = '';
      node.classList.add('isu-leaving');
      win.setTimeout(() => node.remove(), LEAVE_MS);
    }
    function dismiss() { if (dialog && !installing) close(!!available); }
    function shell(screen) {
      if (closeTimer) { win.clearTimeout(closeTimer); closeTimer = null; }
      const fresh = !dialog;
      if (fresh) {
        previousFocus = doc.activeElement;
        dialog = doc.createElement('div'); dialog.id = 'inhouse-services-update'; dialog.className = settingsContainer ? 'isu-inline' : 'isu-overlay';
        dialog.setAttribute('role', settingsContainer ? 'region' : 'dialog');
        if (!settingsContainer) dialog.setAttribute('aria-modal', 'true');
        dialog.setAttribute('aria-labelledby', 'isu-title');
        const card = el('section', null, 'isu-card'); card.tabIndex = -1;
        if (!settingsContainer) card.append(el('div', null, 'isu-handle'));
        const body = el('div', null, 'isu-body'); body.dataset.isuBody = ''; card.append(body); dialog.append(card);
        dialog.addEventListener('keydown', event => {
          if (event.key === 'Escape' && !installing) { event.preventDefault(); event.stopPropagation(); close(!!available); }
          if (event.key === 'Tab' && !settingsContainer) {
            const buttons = [...dialog.querySelectorAll('button:not(:disabled),a[href],input:not(:disabled),[tabindex="0"]')]
              .filter(item => !item.closest('[hidden]') && win.getComputedStyle(item).visibility !== 'hidden');
            const first = buttons[0], last = buttons[buttons.length - 1], inside = buttons.includes(doc.activeElement);
            if (event.shiftKey && (doc.activeElement === first || !inside)) { event.preventDefault(); last?.focus(); }
            else if (!event.shiftKey && (doc.activeElement === last || !inside)) { event.preventDefault(); first?.focus(); }
          }
        });
        if (!settingsContainer) {
          dialog.addEventListener('click', event => { if (event.target === dialog) dismiss(); });
          draggable(card);
        }
        (settingsContainer || doc.body).append(dialog);
      }
      const card = dialog.querySelector('.isu-card'), body = dialog.querySelector('[data-isu-body]');
      transition = { card, body, fresh, from: fresh ? 0 : card.getBoundingClientRect().height };
      statusNode = el('span', null, 'isu-status'); statusNode.dataset.isuStatus = ''; statusNode.setAttribute('role', 'status');
      card.dataset.screen = screen; body.replaceChildren();
      return body;
    }
    // Called once a screen is filled: places the status line and runs a fade-through.
    function settle(focus) {
      const { card, body, fresh, from } = transition; transition = null;
      if (!statusNode.isConnected) body.append(statusNode);
      if (focus) focus.focus({ preventScroll: true });
      else if (!fresh || !settingsContainer) card.focus({ preventScroll: true });
      if (!motion() || fresh) return;
      body.classList.remove('isu-swap'); void body.offsetWidth; body.classList.add('isu-swap');
      resize(card, from);
    }
    function resize(card, from) {
      const to = card.getBoundingClientRect().height;
      if (!motion() || !card.animate || !from || Math.abs(to - from) < 2) return;
      card.style.overflow = 'hidden';
      card.animate([{ height: from + 'px' }, { height: to + 'px' }], { duration: 300, easing: 'cubic-bezier(.2,0,0,1)' }).onfinish = () => { card.style.overflow = ''; };
    }
    function morph(change) {
      const card = dialog?.querySelector('.isu-card'), from = card ? card.getBoundingClientRect().height : 0;
      change(); if (card) resize(card, from);
    }
    // Drag the sheet down by its handle or headline to dismiss it, as on Android.
    function draggable(card) {
      let drag = null;
      card.addEventListener('pointerdown', event => {
        if (installing || !event.isPrimary || event.pointerType === 'mouse') return;
        if (event.target.closest('button,a,input') || !event.target.closest('.isu-handle,.isu-head')) return;
        drag = { id: event.pointerId, y: event.clientY, dy: 0, last: event.clientY, at: event.timeStamp, speed: 0 };
        card.setPointerCapture?.(event.pointerId); card.classList.add('isu-dragging');
      });
      card.addEventListener('pointermove', event => {
        if (!drag || event.pointerId !== drag.id) return;
        const raw = event.clientY - drag.y, elapsed = Math.max(1, event.timeStamp - drag.at);
        drag.speed = (event.clientY - drag.last) / elapsed; drag.last = event.clientY; drag.at = event.timeStamp;
        drag.dy = raw > 0 ? raw : raw / 6;
        card.style.transform = 'translateY(' + drag.dy + 'px)';
        dialog?.style.setProperty('--isu-drag', String(Math.max(0, Math.min(1, drag.dy / card.offsetHeight))));
      });
      const end = event => {
        if (!drag || event.pointerId !== drag.id) return;
        const { dy, speed } = drag; drag = null;
        if (!installing && (dy > Math.min(140, card.offsetHeight * 0.3) || (speed > 0.6 && dy > 32))) { dismiss(); return; }
        card.classList.remove('isu-dragging'); card.style.transform = ''; dialog?.style.removeProperty('--isu-drag');
      };
      card.addEventListener('pointerup', end); card.addEventListener('pointercancel', end);
    }
    function el(tag, text, className) { const value = doc.createElement(tag); if (text != null) value.textContent = text; if (className) value.className = className; return value; }
    function icon(name, className) { const value = el('span', null, className || 'isu-icon'); value.innerHTML = ICONS[name]; value.setAttribute('aria-hidden', 'true'); return value; }
    function button(parent, text, action, className = 'isu-btn isu-text') {
      const value = el('button', text, className); value.type = 'button'; value.onclick = action; parent.append(value); return value;
    }
    function status(text, tone = '') {
      if (!dialog || !statusNode) return;
      const apply = () => {
        statusNode.dataset.tone = text ? tone : '';
        statusNode.replaceChildren();
        if (!text) return;
        if (tone === 'busy') statusNode.append(el('span', null, 'isu-spinner'));
        else if (tone === 'ok' || tone === 'error' || tone === 'warn') statusNode.append(icon(tone === 'ok' ? 'done' : 'error', 'isu-status-icon'));
        statusNode.append(el('span', text));
      };
      if (!!text !== !!statusNode.textContent && statusNode.isConnected && !transition) morph(apply); else apply();
    }
    function header(title, subtitle, className = '') {
      const value = el('header', null, 'isu-head' + (className ? ' ' + className : '')), heading = el('h2', title); heading.id = 'isu-title';
      value.append(heading); if (subtitle) value.append(subtitle);
      return value;
    }
    function actions(parent, className = '') { const value = el('div', null, 'isu-actions' + (className ? ' ' + className : '')); parent.append(value); return value; }
    function size(bytes) { return bytes >= 1048576 ? (bytes / 1048576).toFixed(1) + ' MB' : Math.max(1, Math.round(bytes / 1024)) + ' KB'; }
    // B is the default. Legacy implicit A preferences require renewed, explicit consent.
    function selectChannel(channel) {
      if (installing) return;
      readState(); state.channel = channel; state.developmentConsent = channel === 'A' ? 1 : 0;
      saveState(); generation++; checking = null; available = null; lastPrompt = '';
      settings(); check({ manual: true });
    }
    function credits(parent) {
      const value = el('span', null, 'isu-credits');
      const brand = el('a', 'Made by Inhouse', 'isu-brand'); brand.href = 'https://inhouse.miguelcoxcaballero.com/';
      const github = el('a', 'GitHub', 'isu-github'); github.href = 'https://github.com/' + config.repository;
      // Native hosts open these allowlisted links in the browser, outside the installer bridge.
      value.append(brand, github); parent.append(value); return value;
    }
    // A completed drag (or End on the keyboard) is required; a tap on the track never confirms.
    function slideToConfirm(armed, confirm) {
      const track = el('div', null, 'isu-slide'); track.hidden = true;
      const slider = el('input'); slider.id = 'isu-confirm-a'; slider.type = 'range'; slider.min = '0'; slider.max = '100'; slider.step = '1'; slider.value = '0';
      slider.setAttribute('aria-label', 'Slide to confirm version A');
      const label = el('span', 'Slide to switch to A', 'isu-slide-label'); label.setAttribute('aria-hidden', 'true');
      const thumb = icon('arrow', 'isu-slide-thumb');
      track.append(slider, el('span', null, 'isu-slide-fill'), label, thumb);
      const paint = () => track.style.setProperty('--v', String(Number(slider.value) / 100));
      slider.addEventListener('input', paint);
      slider.onchange = () => { if (armed() && Number(slider.value) === 100) confirm(); else { slider.value = '0'; paint(); } };
      let drag = null;
      track.addEventListener('pointerdown', event => {
        const bounds = thumb.getBoundingClientRect();
        if (!event.isPrimary || event.clientX < bounds.left - 12 || event.clientX > bounds.right + 12) return;
        event.preventDefault();
        drag = { id: event.pointerId, x: event.clientX, from: Number(slider.value), span: Math.max(1, track.clientWidth - thumb.offsetWidth - 8) };
        track.setPointerCapture?.(event.pointerId); track.classList.add('is-dragging');
      });
      track.addEventListener('pointermove', event => {
        if (!drag || event.pointerId !== drag.id) return;
        slider.value = String(Math.round(Math.max(0, Math.min(100, drag.from + (event.clientX - drag.x) / drag.span * 100)))); paint();
      });
      const release = event => {
        if (!drag || event.pointerId !== drag.id) return;
        drag = null; track.classList.remove('is-dragging');
        if (Number(slider.value) >= 92) { slider.value = '100'; paint(); }
        slider.dispatchEvent(new win.Event('change'));
      };
      track.addEventListener('pointerup', release); track.addEventListener('pointercancel', release);
      return { track, slider };
    }
    function confirmDevelopment() {
      // Opening or cancelling this screen never opts in or requests an A catalog.
      generation++; checking = null; available = null;
      const body = shell('warning');
      const head = header('Switch to version A?', null, 'isu-centered'); head.prepend(icon('warning', 'isu-icon isu-alert'));
      body.append(head, el('p', 'Version A gets untested development updates. Expect bugs, crashes or data loss.', 'isu-text-body'));
      let agreed = false;
      const { track, slider } = slideToConfirm(() => agreed, () => selectChannel('A'));
      body.append(track);
      const row = actions(body);
      button(row, 'Cancel', () => settings());
      const yes = button(row, 'I understand', () => {
        agreed = true;
        morph(() => { yes.hidden = true; track.hidden = false; });
        slider.focus({ preventScroll: true });
      }, 'isu-btn isu-text isu-primary');
      settle();
    }
    function settings(message = '', presentation = {}) {
      if (presentation.container && presentation.container !== settingsContainer) {
        if (dialog) close();
        settingsContainer = presentation.container;
      }
      readState(); const body = shell('settings');
      body.append(header('Updates', el('p', config.appName + ' ' + installed(), 'isu-sub')));
      body.append(el('h3', 'Channel', 'isu-subheader'));
      const options = el('div', null, 'isu-channels'); options.setAttribute('role', 'group'); options.setAttribute('aria-label', 'Update channel'); body.append(options);
      [['B', 'Reviewed releases'], ['A', 'Development builds, may be unstable']].forEach(([channel, note]) => {
        const value = button(options, null, () => {
          if (installing || state.channel === channel) return;
          if (channel === 'A') confirmDevelopment(); else selectChannel('B');
        }, 'isu-item');
        const text = el('span', null, 'isu-item-text'), description = el('span', note, 'isu-item-support'); description.id = 'isu-channel-' + channel;
        text.append(el('span', 'Version ' + channel, 'isu-item-title'), description);
        value.append(el('span', null, 'isu-radio'), text);
        value.setAttribute('aria-label', 'Version ' + channel); value.setAttribute('aria-describedby', description.id);
        value.setAttribute('aria-pressed', String(state.channel === channel));
      });
      body.append(el('div', null, 'isu-divider'));
      const checkItem = el('button', null, 'isu-item isu-check'); checkItem.type = 'button'; checkItem.onclick = () => check({ manual: true });
      const label = el('span', 'Check for updates', 'isu-item-title'); label.id = 'isu-check-label';
      const text = el('span', null, 'isu-item-text'); text.append(label, statusNode);
      checkItem.append(text); checkItem.setAttribute('aria-labelledby', label.id); body.append(checkItem);
      const row = actions(body, 'isu-actions-split'); credits(row);
      if (!settingsContainer) button(row, 'Done', () => close(false));
      settle(); status(message);
    }
    function progress(payload, label) {
      if (!dialog) return;
      const bar = dialog.querySelector('[data-isu-progress]'); if (!bar) return;
      bar.parentElement.dataset.state = 'busy';
      const meter = bar.querySelector('progress'), track = bar.querySelector('.isu-track');
      const total = Number(payload.totalBytes || available?.latest.apkSizeBytes || 0), downloaded = Math.max(0, Number(payload.downloadedBytes) || 0);
      const known = payload.percent >= 0 || payload.downloadedBytes != null;
      track.classList.toggle('is-indeterminate', !known);
      if (!known) {
        meter.removeAttribute('value'); bar.querySelector('.isu-progress-label').textContent = 'Preparing download…'; bar.querySelector('.isu-progress-value').textContent = '';
        return;
      }
      const percent = Math.max(0, Math.min(100, payload.percent >= 0 ? Math.round(payload.percent) : total > 0 ? Math.round(downloaded * 100 / total) : 0));
      meter.value = percent; track.style.setProperty('--p', String(percent / 100));
      bar.querySelector('.isu-progress-label').textContent = label || (percent >= 100 ? 'Verifying…' : 'Downloading… ' + percent + '%');
      bar.querySelector('.isu-progress-value').textContent = downloaded && total ? size(downloaded) + ' of ' + size(total) : '';
    }
    function downloadRecovery(parent, release, hidden = false) {
      let url;
      // Persisted attempts are untrusted too; use the same repository-bound URL
      // validation as the catalog. Never take a download URL from native errors.
      try { url = releasePageUrl(release?.apkUrl, config); } catch (_) { return; }
      const recovery = el('div', null, 'isu-recovery'); recovery.dataset.isuRecovery = ''; recovery.hidden = hidden;
      const link = el('a', 'Download APK', 'isu-download'); link.href = url;
      // A regular link lets old Notes/Read hosts open GitHub in the browser.
      // Their installer bridge, auth-only browser API and new windows may fail.
      recovery.append(link, el('p', 'Choose the APK under Assets on GitHub.', 'isu-recovery-help'));
      parent.append(recovery);
    }
    function showRecovery(visible) {
      const recovery = dialog?.querySelector('[data-isu-recovery]');
      if (recovery && recovery.hidden === visible) morph(() => { recovery.hidden = !visible; });
    }
    function result(payload) {
      if (typeof payload === 'string') { try { payload = JSON.parse(payload); } catch (_) { return; } }
      if (!payload) return;
      if (payload.status === 'installed') {
        // Only the native package manager (or a fresh installed-version read)
        // confirms installation. Opening Android's dialog is not success.
        saveInstall(null); installing = false;
        installationOutcome('Update installed', config.appName + ' ' + installed());
        return;
      }
      if (!dialog) return;
      const install = dialog.querySelector('[data-isu-install]'); if (!install) return;
      const idle = () => { install.closest('[data-state]').dataset.state = 'idle'; };
      if (payload.status === 'downloading') { installing = true; install.disabled = true; showRecovery(false); progress(payload); status(''); }
      else if (payload.status === 'ready' || payload.status === 'installing') {
        const pending = savedInstall(); if (pending) saveInstall({ ...pending, awaitingResult: true });
        installing = false; progress({ ...payload, percent: 100 }); idle();
        install.disabled = false; install.textContent = 'Open installer'; status('Confirm the installation in Android. If the app closes, open it again.');
        showRecovery(typeof native()?.getInstallState !== 'function');
      }
      else if (payload.status === 'permission_required') { installing = false; install.disabled = false; install.textContent = 'Continue'; idle(); status('Allow installs from this app, then tap Continue.', 'warn'); }
      else if (payload.status === 'error' || payload.status === 'cancelled' || payload.status === 'unconfirmed') {
        installing = false; install.disabled = false; install.textContent = 'Try again'; idle();
        status(payload.message || (payload.status === 'cancelled' ? 'Installation cancelled.' : 'The update could not be installed.'), payload.status === 'error' ? 'error' : 'warn');
        showRecovery(true);
      }
      dialog.querySelectorAll('[data-isu-idle]').forEach(b => { b.disabled = installing; });
    }
    function installationOutcome(title, message, recovery) {
      // The invisible checker must leave results for the foreground host.
      if (config.onOffer) return;
      installing = false; available = null;
      const body = shell('result'); body.append(header(title), el('p', message, 'isu-text-body'));
      if (recovery) downloadRecovery(body, recovery);
      const row = actions(body); button(row, 'Updates', () => settings()); button(row, 'Done', () => close(false), 'isu-btn isu-filled isu-primary');
      settle();
    }
    function refreshInstallation() {
      if (config.onOffer || installing || doc.visibilityState === 'hidden') return false;
      let nativeState = null, seen = '';
      try {
        nativeState = JSON.parse(native()?.getInstallState?.() || 'null');
        seen = win.localStorage.getItem(seenInstallKey) || '';
      } catch (_) {}
      const pending = savedInstall();
      const confirmed = pending && (installedCode() > 0
        ? installedCode() >= pending.versionCode : compareVersions(installed(), pending.version) >= 0);
      const activeCode = (dialog?.querySelector('[data-isu-install]') ? available?.latest.versionCode : 0) || 0;
      const requestedCode = Math.max(pending?.versionCode || 0, activeCode);
      const relevantPending = pending && pending.versionCode >= activeCode;
      const resultId = nativeState?.id + ':' + nativeState?.status;
      if (nativeState?.id && ['installed', 'cancelled', 'error'].includes(nativeState.status) && resultId !== seen
          && Number(nativeState.expectedVersionCode) >= requestedCode) {
        // Persist only after the result is rendered, so process death does not
        // swallow it. Reads from the native bridge remain idempotent.
        if (nativeState.status === 'installed') result(nativeState);
        else if (dialog?.querySelector('[data-isu-install]')) result(nativeState);
        else installationOutcome('Update not installed', nativeState.message || (nativeState.status === 'cancelled' ? 'Installation cancelled.' : 'Android could not install the update.'),
          pending?.versionCode === Number(nativeState.expectedVersionCode) ? pending : null);
        try { win.localStorage.setItem(seenInstallKey, resultId); } catch (_) {}
        saveInstall(null); return true;
      }
      if (confirmed && relevantPending) { result({ status: 'installed' }); return true; }
      if (relevantPending && pending.awaitingResult && !pending.reported) {
        const message = 'Installation has not been confirmed. You can try again.';
        if (dialog?.querySelector('[data-isu-install]')) result({ status: 'unconfirmed', message });
        else installationOutcome('Update not confirmed', message, pending);
        saveInstall({ ...pending, reported: true }); return true;
      }
      return false;
    }
    function show(value) {
      available = value;
      const release = value.latest, body = shell('offer');
      const versions = el('span', null, 'isu-versions'); versions.append(doc.createTextNode(installed() + ' → '), el('b', release.version));
      const subtitle = el('p', null, 'isu-sub'); subtitle.append(versions, doc.createTextNode(' · ' + size(release.apkSizeBytes)));
      body.append(header('Update available', subtitle));
      const changes = value.releases.filter(r => newer(r, installed(), installedCode()));
      if (changes.length) body.append(el('h3', 'What’s new', 'isu-subheader'), el('p', release.changelog, 'isu-text-body isu-note'));
      // Everything else stays one tap away: intermediate versions, release date and channel.
      const more = el('div', null, 'isu-more'); more.id = 'isu-more'; more.hidden = true;
      const earlier = changes.filter(r => r.versionCode !== release.versionCode);
      if (earlier.length) {
        const list = el('ul', null, 'isu-changes');
        earlier.slice(0, 3).forEach(r => { const item = el('li'); item.append(el('small', r.version), el('span', r.changelog)); list.append(item); });
        more.append(el('h3', 'Also in this update', 'isu-subheader isu-subheader-first'), list);
        if (earlier.length > 3) more.append(el('p', '+' + (earlier.length - 3) + ' earlier ' + (earlier.length - 3 === 1 ? 'version' : 'versions'), 'isu-earlier'));
      }
      const facts = el('dl', null, 'isu-facts'), released = new Date(release.releasedAt || '');
      if (!Number.isNaN(released.getTime())) {
        const row = el('div'); row.append(el('dt', 'Released'), el('dd', released.toLocaleDateString('en', { day: 'numeric', month: 'short', year: 'numeric' }))); facts.append(row);
      }
      const channelRow = el('div'), channelValue = el('dd', 'Version ' + value.channel + ' ');
      const change = button(channelValue, 'Change', () => settings(), 'isu-link'); change.dataset.isuIdle = ''; change.setAttribute('aria-label', 'Change channel');
      channelRow.append(el('dt', 'Channel'), channelValue); facts.append(channelRow); more.append(facts);
      const toggle = el('button', null, 'isu-disclosure'); toggle.type = 'button';
      toggle.setAttribute('aria-expanded', 'false'); toggle.setAttribute('aria-controls', more.id);
      toggle.append(el('span', 'Details'), icon('expand'));
      toggle.onclick = () => {
        const open = more.hidden;
        morph(() => { more.hidden = !open; toggle.setAttribute('aria-expanded', String(open)); });
      };
      body.append(toggle, more, statusNode);
      // Older installed apps can take the recovery route immediately after a
      // web refresh, without repeating an installer flow that may be broken.
      downloadRecovery(body, release, typeof native()?.getInstallState === 'function');
      const dock = el('div', null, 'isu-dock'); dock.dataset.state = 'idle';
      const row = actions(dock);
      const later = button(row, 'Later', () => close(true)); later.dataset.isuIdle = '';
      const install = button(row, 'Update', () => {
        installing = true; install.disabled = true; dialog.querySelectorAll('[data-isu-idle]').forEach(b => { b.disabled = true; });
        saveInstall({ version: release.version, versionCode: release.versionCode, apkUrl: release.apkUrl, awaitingResult: false });
        showRecovery(false); status(''); progress({});
        try {
          if (config.install) config.install(release);
          else if (native()?.installInhouseUpdate) native().installInhouseUpdate(release.apkUrl, release.apkSha256);
          else if (native()?.installAppUpdate) native().installAppUpdate(release.apkUrl, release.apkSha256);
          else throw Error('Open the Android app to install.');
        } catch (error) { result({ status: 'error', message: error.message }); }
      }, 'isu-btn isu-filled isu-primary'); install.dataset.isuInstall = '';
      const bar = el('div', null, 'isu-progress'); bar.dataset.isuProgress = '';
      const labels = el('div', null, 'isu-progress-row'); labels.append(el('span', 'Preparing download…', 'isu-progress-label'), el('span', '', 'isu-progress-value'));
      const meter = doc.createElement('progress'); meter.max = 100; meter.value = 0; meter.setAttribute('aria-label', 'Download');
      const track = el('div', null, 'isu-track'); track.append(el('i'));
      bar.append(track, labels, meter); dock.append(bar); body.append(dock);
      settle();
    }
    async function check({ manual = false } = {}) {
      if (installing || (!manual && snoozed())) return false;
      if (checking) {
        const token = generation, pending = checking;
        if (manual) status('Checking…', 'busy');
        const found = await pending;
        if (manual && token === generation) {
          if (available && found) show(available);
          else report(available);
        }
        return found;
      }
      readState(); const channel = state.channel, token = generation;
      if (manual && !dialog) settings(); if (manual) status('Checking…', 'busy');
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
          if (manual) report(value);
          return false;
        } catch (_) { if (manual) report(null); return false; }
      })();
      checking = pending;
      try { return await pending; } finally { if (checking === pending) checking = null; }
    }
    function report(value) {
      if (!value) status('Could not check for updates.', 'error');
      else if (!value.latest) status('No releases on version ' + value.channel + '.');
      else if (compareVersions(installed(), value.latest.version) > 0) status('Your version is newer than version ' + value.channel + '.');
      else status('You’re up to date.', 'ok');
    }
    function start() {
      if (started) return; started = true; readState();
      win.handleInhouseUpdateResult = result;
      win.addEventListener('focus', automatic); doc.addEventListener('visibilitychange', visible);
      timer = win.setInterval(automatic, 15 * 60000); automatic();
    }
    function automatic() { if (!refreshInstallation()) check(); }
    function visible() { if (doc.visibilityState !== 'hidden') refreshInstallation(); }
    function stop() { win.removeEventListener('focus', automatic); doc.removeEventListener('visibilitychange', visible); win.clearInterval(timer); started = false; }
    function offerPending(value) {
      readState(); const valid = validateCatalog(value, state.channel, config);
      if (!snoozed() && newer(valid.latest, installed(), installedCode())) { win.handleInhouseUpdateResult = result; show(valid); return true; }
      return false;
    }
    win.handleInhouseUpdateResult = result;
    return { start, stop, check, settings, close, result, offerPending, refreshInstallation, getState: readState, version: VERSION };
  }
  const api = { create, compareVersions, validateCatalog, newer, TWO_WEEKS, VERSION };
  root.InhouseServicesUpdater = api;
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
})(typeof window !== 'undefined' ? window : globalThis);
