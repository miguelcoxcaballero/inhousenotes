/* Inhouse services updater — canonical source: github.com/miguelcoxcaballero/inhouse-services-updater.
 * Edit here, sync the SAME revision into YouTube, Instagram, Notes, Photos and Read.
 * Every new release goes to A with a tiny changelog. Publish B ONLY on the user's explicit request.
 * B is the user default; A needs warning acknowledgement and a slide confirmation.
 * Updates are optional. Later defers automatic prompts for 14 days; settings always allow a manual check.
 */
(function (root) {
  'use strict';
  const VERSION = '1.1.0', TWO_WEEKS = 14 * 86400000;
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
    let statusNode = null, transition = null, closeTimer = null;
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
    // Presentation. Screens: offer (update available), settings, and the version A warning.
    // Overlays are a bottom sheet on phones and a centered dialog on wide screens; hosts can
    // embed settings inline. Motion is skipped when reduced motion is requested.
    const ICONS = {
      mark: '<svg viewBox="0 0 40 24" aria-hidden="true"><path d="M4 22L20 6L36 22"/></svg>',
      chevron: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M9 6l6 6-6 6"/></svg>',
      arrow: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5 12h14M13 6l6 6-6 6"/></svg>',
      warning: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3.8L2.6 20h18.8z"/><path d="M12 10v4.5M12 17.2v.1"/></svg>',
      ok: '<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="10"/><path d="M7.5 12.3l3 3 6-6.3"/></svg>',
      error: '<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="10"/><path d="M12 7v6M12 16.6v.1"/></svg>',
    };
    const LEAVE_MS = 260;
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
        if (!settingsContainer) card.append(el('div', null, 'isu-grabber'));
        const body = el('div', null, 'isu-body'); body.dataset.isuBody = ''; card.append(body); dialog.append(card);
        dialog.addEventListener('keydown', event => {
          if (event.key === 'Escape' && !installing) { event.preventDefault(); event.stopPropagation(); close(!!available); }
          if (event.key === 'Tab' && !settingsContainer) {
            const buttons = [...dialog.querySelectorAll('button:not(:disabled),a[href],input:not(:disabled),[tabindex="0"]')].filter(item => !item.closest('[hidden]') && win.getComputedStyle(item).visibility !== 'hidden');
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
      statusNode = el('p', null, 'isu-status'); statusNode.dataset.isuStatus = ''; statusNode.setAttribute('role', 'status');
      card.dataset.screen = screen; body.replaceChildren();
      return body;
    }
    // Called once a screen is filled: places the status line and animates the change of screen.
    function settle(focus) {
      const { card, body, fresh, from } = transition; transition = null;
      if (!statusNode.isConnected) body.append(statusNode);
      if (focus) focus.focus({ preventScroll: true });
      else if (!fresh || !settingsContainer) card.focus({ preventScroll: true });
      if (!motion()) return;
      body.classList.remove('isu-enter'); void body.offsetWidth; body.classList.add('isu-enter');
      if (!fresh) resize(card, from);
    }
    function resize(card, from) {
      const to = card.getBoundingClientRect().height;
      if (!motion() || !card.animate || !from || Math.abs(to - from) < 2) return;
      card.style.overflow = 'hidden';
      card.animate([{ height: from + 'px' }, { height: to + 'px' }], { duration: 320, easing: 'cubic-bezier(.2,.8,.2,1)' }).onfinish = () => { card.style.overflow = ''; };
    }
    function morph(change) {
      const card = dialog?.querySelector('.isu-card'), from = card ? card.getBoundingClientRect().height : 0;
      change(); if (card) resize(card, from);
    }
    // Swipe the sheet down by its grabber or header to dismiss it, as on Android and iOS.
    function draggable(card) {
      let drag = null;
      card.addEventListener('pointerdown', event => {
        if (installing || !event.isPrimary || event.pointerType === 'mouse') return;
        if (event.target.closest('button,a,input') || !event.target.closest('.isu-grabber,.isu-head')) return;
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
    function svg(name, className) { const value = el('span', null, className); value.innerHTML = ICONS[name]; value.setAttribute('aria-hidden', 'true'); return value; }
    function button(parent, text, action, className = 'isu-btn isu-tonal') {
      const value = el('button', text, className); value.type = 'button'; value.onclick = action; parent.append(value); return value;
    }
    function status(text, tone = '') {
      if (!dialog || !statusNode) return;
      const apply = () => {
        statusNode.dataset.tone = text ? tone : '';
        statusNode.replaceChildren();
        if (!text) return;
        if (tone === 'busy') statusNode.append(el('span', null, 'isu-spinner'));
        else if (tone) statusNode.append(svg(tone === 'ok' ? 'ok' : 'error', 'isu-status-icon'));
        statusNode.append(el('span', text));
      };
      if (!!text !== !!statusNode.textContent && statusNode.isConnected && !transition) morph(apply); else apply();
    }
    function header(title, options = {}) {
      const value = el('header', null, 'isu-head' + (options.bar ? ' isu-bar' : ''));
      if (options.icon) value.append(options.icon);
      const text = el('div', null, 'isu-head-text'), heading = el('h2', title); heading.id = 'isu-title';
      text.append(heading); if (options.subtitle) text.append(options.subtitle);
      value.append(text); return value;
    }
    function appIcon() { return svg('mark', 'isu-icon'); }
    function size(bytes) { return bytes >= 1048576 ? (bytes / 1048576).toFixed(1) + ' MB' : Math.max(1, Math.round(bytes / 1024)) + ' KB'; }
    // B is the default. Legacy implicit A preferences require renewed, explicit consent.
    function selectChannel(channel) {
      if (installing) return;
      readState(); state.channel = channel; state.developmentConsent = channel === 'A' ? 1 : 0;
      saveState(); generation++; checking = null; available = null; lastPrompt = '';
      settings(); check({ manual: true });
    }
    function footer(body) {
      const value = el('footer', null, 'isu-footer');
      const brand = el('a', null, 'isu-brand'); brand.href = 'https://inhouse.miguelcoxcaballero.com/';
      brand.append(svg('mark', 'isu-mark'), el('span', 'Made by Inhouse'));
      const github = el('a', 'GitHub', 'isu-github'); github.href = 'https://github.com/' + config.repository;
      const dot = el('span', '·'); dot.setAttribute('aria-hidden', 'true');
      // Native hosts open these allowlisted links in the browser, outside the installer bridge.
      value.append(brand, dot, github); body.append(value);
    }
    // A completed drag (or End on the keyboard) is required; a tap on the track never confirms.
    function slideToConfirm(armed, confirm) {
      const track = el('div', null, 'isu-slide'); track.hidden = true;
      const slider = el('input'); slider.id = 'isu-confirm-a'; slider.type = 'range'; slider.min = '0'; slider.max = '100'; slider.step = '1'; slider.value = '0';
      slider.setAttribute('aria-label', 'Slide to confirm version A');
      const label = el('span', 'Slide to switch', 'isu-slide-label'); label.setAttribute('aria-hidden', 'true');
      const thumb = svg('arrow', 'isu-slide-thumb');
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
      body.append(header('Switch to version A?', { icon: svg('warning', 'isu-icon isu-icon-warn') }));
      body.append(el('p', 'Version A gets untested development updates. Expect bugs, crashes or data loss.', 'isu-warning'));
      let agreed = false;
      const { track, slider } = slideToConfirm(() => agreed, () => selectChannel('A'));
      const actions = el('div', null, 'isu-actions');
      button(actions, 'Cancel', () => settings());
      const yes = button(actions, 'I understand', () => {
        agreed = true;
        morph(() => { yes.hidden = true; track.hidden = false; actions.classList.add('isu-armed'); });
        slider.focus({ preventScroll: true });
      }, 'isu-btn isu-primary');
      body.append(track, actions); settle();
    }
    function settings(message = '', presentation = {}) {
      if (presentation.container && presentation.container !== settingsContainer) {
        if (dialog) close();
        settingsContainer = presentation.container;
      }
      readState(); const body = shell('settings');
      const head = header('Updates', { bar: true });
      if (!settingsContainer) button(head, 'Done', () => close(false), 'isu-link');
      const app = el('div', null, 'isu-app-row'), name = el('div');
      name.append(el('strong', config.appName), el('span', 'Version ' + installed(), 'isu-app'));
      app.append(appIcon(), name);
      body.append(head, app, el('h3', 'Channel', 'isu-label'));
      const options = el('div', null, 'isu-channels'); options.setAttribute('role', 'group'); options.setAttribute('aria-label', 'Update channel'); body.append(options);
      [['B', 'Reviewed releases'], ['A', 'Development builds']].forEach(([channel, note]) => {
        const value = button(options, null, () => {
          if (installing || state.channel === channel) return;
          if (channel === 'A') confirmDevelopment(); else selectChannel('B');
        }, 'isu-option');
        const description = el('span', note, 'isu-option-note'); description.id = 'isu-channel-' + channel;
        value.append(el('span', 'Version ' + channel, 'isu-option-title'), description, el('span', null, 'isu-radio'));
        value.setAttribute('aria-label', 'Version ' + channel); value.setAttribute('aria-describedby', description.id);
        value.setAttribute('aria-pressed', String(state.channel === channel));
      });
      button(body, 'Check for updates', () => check({ manual: true }), 'isu-btn isu-tonal isu-check');
      body.append(statusNode); footer(body);
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
        meter.removeAttribute('value'); bar.querySelector('.isu-progress-label').textContent = 'Preparing…'; bar.querySelector('.isu-progress-value').textContent = '';
        return;
      }
      const percent = Math.max(0, Math.min(100, payload.percent >= 0 ? Math.round(payload.percent) : total > 0 ? Math.round(downloaded * 100 / total) : 0));
      meter.value = percent; track.style.setProperty('--p', String(percent / 100));
      bar.querySelector('.isu-progress-label').textContent = label || (percent >= 100 ? 'Verifying…' : 'Downloading…');
      bar.querySelector('.isu-progress-value').textContent = downloaded && total ? size(downloaded) + ' of ' + size(total) : percent + '%';
    }
    function result(payload) {
      if (typeof payload === 'string') { try { payload = JSON.parse(payload); } catch (_) { return; } }
      if (!dialog || !payload) return;
      const install = dialog.querySelector('[data-isu-install]'); if (!install) return;
      const idle = () => { install.closest('[data-state]').dataset.state = 'idle'; };
      if (payload.status === 'downloading') { installing = true; install.disabled = true; progress(payload); status(''); }
      else if (payload.status === 'ready') { installing = false; progress({ ...payload, percent: 100 }); idle(); install.textContent = 'Installing…'; status('Confirm the installation in Android.'); }
      else if (payload.status === 'permission_required') { installing = false; install.disabled = false; install.textContent = 'Continue'; idle(); status('Allow installs from this app, then tap Continue.', 'warn'); }
      else if (payload.status === 'error') { installing = false; install.disabled = false; install.textContent = 'Try again'; idle(); status(payload.message || 'Download failed.', 'error'); }
      dialog.querySelectorAll('[data-isu-idle]').forEach(b => { b.disabled = installing; });
    }
    function show(value) {
      available = value;
      const release = value.latest, body = shell('offer');
      const versions = el('span', null, 'isu-versions'); versions.append(doc.createTextNode(installed() + ' → '), el('b', release.version));
      const subtitle = el('p', null, 'isu-sub'); subtitle.append(versions, el('span', size(release.apkSizeBytes), 'isu-size'));
      const head = header('Update available', { icon: appIcon(), subtitle });
      const channel = button(head, 'Version ' + value.channel, () => settings(), 'isu-chip');
      channel.setAttribute('aria-label', 'Version ' + value.channel + ', change channel'); channel.dataset.isuIdle = ''; channel.append(svg('chevron'));
      body.append(head);
      const changes = value.releases.filter(r => newer(r, installed(), installedCode()));
      if (changes.length) {
        const notes = el('section', null, 'isu-notes'), list = el('ul', null, 'isu-changes');
        changes.slice(0, 3).forEach(r => { const item = el('li'); item.append(el('span', r.changelog)); if (changes.length > 1) item.append(el('small', r.version)); list.append(item); });
        notes.append(el('h3', 'What’s new', 'isu-label'), list);
        if (changes.length > 3) notes.append(el('p', '+' + (changes.length - 3) + ' more', 'isu-more'));
        body.append(notes);
      }
      body.append(statusNode);
      const dock = el('div', null, 'isu-dock'); dock.dataset.state = 'idle';
      const actions = el('div', null, 'isu-actions');
      const later = button(actions, 'Later', () => close(true)); later.dataset.isuIdle = '';
      const install = button(actions, 'Update', () => {
        installing = true; install.disabled = true; dialog.querySelectorAll('[data-isu-idle]').forEach(b => { b.disabled = true; });
        status(''); progress({});
        try {
          if (config.install) config.install(release);
          else if (native()?.installInhouseUpdate) native().installInhouseUpdate(release.apkUrl, release.apkSha256);
          else if (native()?.installAppUpdate) native().installAppUpdate(release.apkUrl, release.apkSha256);
          else throw Error('Open the Android app to install.');
        } catch (error) { result({ status: 'error', message: error.message }); }
      }, 'isu-btn isu-primary'); install.dataset.isuInstall = '';
      const bar = el('div', null, 'isu-progress'); bar.dataset.isuProgress = '';
      const row = el('div', null, 'isu-progress-row'); row.append(el('span', 'Preparing…', 'isu-progress-label'), el('span', '', 'isu-progress-value'));
      const meter = doc.createElement('progress'); meter.max = 100; meter.value = 0; meter.setAttribute('aria-label', 'Download');
      const track = el('div', null, 'isu-track'); track.append(el('i'));
      bar.append(row, track, meter); dock.append(actions, bar); body.append(dock);
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
