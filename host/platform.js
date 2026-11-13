// Effect family 3.6 (0xa0..0xdf): Google Identity Services, the Android `InhouseNative` bridges and the callbacks
// Android evaluates, the shared updater, file lists, clipboard, share, download, fullscreen and history. A dumb
// executor of rust/crates/ihn-app/src/shell/fx.rs; app mode is the one thing decided here, because it must be
// known before the wasm starts (ARCHITECTURE §3.7).
//
// Results: host.io(req, status, obj, bytes) with status 0 done, -1 not available, -2 failed (bytes = message),
// -3 cancelled by the user. `req` 0 means "no answer wanted".

const FX = 3 << 8;
const CHAN = { OAUTH: 1, PDF_EXPORT: 2, POPSTATE: 0x10 };
const MODE_APP = 1, BOOT_SIGNED_IN = 256;
const encoder = new TextEncoder();

// v5: app-v5.js:detectInhouseApp
function detectApp(params) {
  const flag = params.get('inhouse_app');
  const store = fn => { try { return fn(); } catch (_) { return null; } };
  if (flag === '0' || flag === 'false') {
    store(() => localStorage.removeItem('inhouseNotesAppMode'));
    return false;
  }
  if (flag === '1' || flag === 'true' || /\bInhouseNotesApp\/\d+(?:\.\d+)?\b/i.test(navigator.userAgent || '')) {
    store(() => localStorage.setItem('inhouseNotesAppMode', '1'));
    return true;
  }
  return store(() => localStorage.getItem('inhouseNotesAppMode')) === '1';
}

export default function install(host) {
  const app = detectApp(host.config.params);
  document.documentElement.dataset.inhouseApp = app ? 'true' : 'false';
  window.INHOUSE_NOTES_APP = app;
  host.config.flags |= (app ? MODE_APP : 0) | (window.__INHOUSE_BOOT_SIGNED_IN__ === true ? BOOT_SIGNED_IN : 0);

  const text = value => encoder.encode(String(value ?? ''));
  const done = (req, status = 0, value = '') => { if (req) host.io(req, status, 0, text(value)); };
  const failed = (req, error) => done(req, error?.name === 'AbortError' ? -3 : -2, error?.message || error);
  const strings = r => { const list = []; for (let n = r.u32(); n > 0; n -= 1) list.push(r.str()); return list; };
  let tokenClient = null, updates = null;

  // v5: app-v5.js:handleInhouseNativeOAuth — Android evaluates it with the redirect's query or fragment.
  window.handleInhouseNativeOAuth = payload => {
    const raw = String(payload || '').replace(/^[#?]/, '');
    host.message(CHAN.OAUTH, raw);
    return app && !!new URLSearchParams(raw).get('code');
  };
  // v5: app-v5.js:handleInhousePdfExportResult
  window.handleInhousePdfExportResult = payload => {
    host.message(CHAN.PDF_EXPORT, typeof payload === 'string' ? payload : JSON.stringify(payload ?? null));
  };
  window.addEventListener('popstate', () => host.message(CHAN.POPSTATE, location.href));

  return {
    // gis_init(u32 req, str client_id, str scope) → 0, or -1 while the GIS script has not loaded.
    // v5: app-v5.js:initDriveAuth
    [FX | 0xa0]: r => {
      const req = r.u32(), client_id = r.str(), scope = r.str();
      const oauth = window.google?.accounts?.oauth2;
      if (!oauth) return done(req, -1);
      tokenClient = oauth.initTokenClient({
        client_id, scope, include_granted_scopes: true, callback: () => {},
        error_callback: error => { console.warn('Google popup auth failed:', error); }
      });
      done(req);
    },
    // gis_request(u32 req, u8 interactive, str scope, str login_hint) → 0, the token response as JSON.
    // v5: app-v5.js:requestDriveToken
    [FX | 0xa1]: r => {
      const req = r.u32(), interactive = r.u8() !== 0, scope = r.str(), hint = r.str();
      if (!tokenClient) return done(req, -1);
      tokenClient.callback = response => done(req, 0, JSON.stringify(response || {}));
      const options = { scope, include_granted_scopes: true, prompt: interactive ? 'consent' : 'none' };
      if (hint) options.login_hint = hint;
      try { tokenClient.requestAccessToken(options); } catch (error) { failed(req, error); }
    },
    // gis_revoke(str token)
    [FX | 0xa2]: r => { const token = r.str(); window.google?.accounts?.oauth2?.revoke?.(token, () => {}); },
    // native_call(u32 req, str method, u32 n, n × str) → 0 and String(result); -1 when the bridge lacks the method.
    [FX | 0xa4]: r => {
      const req = r.u32(), method = r.str(), args = strings(r), native = window.InhouseNative;
      if (typeof native?.[method] !== 'function') return done(req, -1);
      try { done(req, 0, native[method](...args)); } catch (error) { failed(req, error); }
    },
    // native_blob(u32 req, u32 blob, str method, u32 chunk_chars): the blob as base64, `chunk_chars` (a multiple of
    // 4) per call of InhouseNative[method], yielding between calls. v5: app-v5.js:deliverExportedPdf
    [FX | 0xa5]: r => {
      const req = r.u32(), blob = host.obj(r.u32()), method = r.str(), step = r.u32() / 4 * 3, native = window.InhouseNative;
      if (!blob || typeof native?.[method] !== 'function') return done(req, -1);
      (async () => {
        const data = new Uint8Array(await blob.arrayBuffer());
        for (let at = 0; at < data.length; at += step) {
          let binary = '';
          for (let p = at; p < Math.min(at + step, data.length); p += 0x8000) {
            binary += String.fromCharCode.apply(null, data.subarray(p, Math.min(p + 0x8000, at + step, data.length)));
          }
          native[method](btoa(binary));
          await new Promise(resolve => setTimeout(resolve));
        }
      })().then(() => done(req), error => failed(req, error));
    },
    // updater_create(u32 req) → 0, or -1 when the shared updater script is missing. Verbatim from 5.18.0.
    // v5: app-v5.js § 4.1 "Inhouse services updater"
    [FX | 0xa8]: r => {
      const req = r.u32();
      if (!window.InhouseServicesUpdater) return done(req, -1);
      updates = window.InhouseServicesUpdater.create({
        appId: 'notes', appName: 'Inhouse Notes', repository: 'miguelcoxcaballero/inhousenotes',
        installedVersion: '1.0.2',
        install: release => {
          // Update stays inside the Android installer. GitHub is an
          // explicit recovery option, never an automatic redirect.
          if (window.InhouseNative?.installInhouseUpdate) window.InhouseNative.installInhouseUpdate(release.apkUrl, release.apkSha256);
          else if (window.InhouseNative?.installAppUpdate) window.InhouseNative.installAppUpdate(release.apkUrl);
          else throw new Error('Abre la app Android para instalar.');
        }
      });
      window.InhouseUpdater = updates;
      updates.start();
      done(req);
    },
    [FX | 0xa9]: () => updates?.settings(),
    // boot_signed_in(u8 value): the flag boot.js computed for the first paint.
    [FX | 0xaa]: r => { window.__INHOUSE_BOOT_SIGNED_IN__ = r.u8() !== 0; },
    // background(u32 req, u8 op, str json): InhouseBackground.schedule() / cancel() / complete(json) (boot.js).
    [FX | 0xab]: r => {
      const req = r.u32(), name = ['schedule', 'cancel', 'complete'][r.u8()], json = r.str(), bridge = window.InhouseBackground;
      if (typeof bridge?.[name] !== 'function') return done(req, -1);
      try { bridge[name](...(name === 'complete' ? [json] : [])); done(req); } catch (error) { failed(req, error); }
    },
    // files_take(u32 req, u32 src, u32 base, u8 images_only): `src` is the `obj` of a change, drop or paste event
    // (FileList or DataTransfer). The files become objects base, base+1, … → status = their count,
    // JSON `[{name, type, size, lastModified}]`. During a paste or drop it must run inside the event.
    // v5: app-v5.js:extractImageBlobsFromPasteEvent
    [FX | 0xb0]: r => {
      const req = r.u32(), src = host.obj(r.u32()), base = r.u32(), images = r.u8() !== 0;
      const wanted = file => !!file && (!images || String(file.type || '').startsWith('image/'));
      let files = [];
      if (images && src?.items) {
        files = Array.from(src.items).filter(wanted).map(item => item.getAsFile?.()).filter(Boolean);
      }
      if (!files.length) files = Array.from(src?.files || (src && src.length !== undefined ? src : [])).filter(wanted);
      files.forEach((file, i) => host.objs.set(base + i, file));
      if (req) host.io(req, files.length, 0, text(JSON.stringify(files.map(file => ({
        name: file.name || '', type: file.type || '', size: file.size, lastModified: file.lastModified || 0
      })))));
    },
    // clipboard_write(u32 req, str text)
    [FX | 0xb4]: r => {
      const req = r.u32(), value = r.str();
      if (!navigator.clipboard?.writeText) return done(req, -1);
      navigator.clipboard.writeText(value).then(() => done(req), error => failed(req, error));
    },
    // share(u32 req, u32 blob, str name, str type, str title) → -1 when the file cannot be shared here.
    [FX | 0xb8]: r => {
      const req = r.u32(), blob = host.obj(r.u32()), name = r.str(), type = r.str(), title = r.str();
      let file = null;
      try { file = blob && new File([blob], name, { type }); } catch (_) {}
      if (!file || !navigator.canShare || !navigator.share || !navigator.canShare({ files: [file] })) return done(req, -1);
      navigator.share({ files: [file], title }).then(() => done(req), error => failed(req, error));
    },
    // download(u32 blob, str name, f64 open_ms, f64 revoke_ms): `<a download>`; `open_ms` > 0 also opens the
    // object URL in a new window after that delay (Android WebView fallback).
    [FX | 0xb9]: r => {
      const blob = host.obj(r.u32()), name = r.str(), openMs = r.f64(), revokeMs = r.f64();
      if (!blob) return;
      const url = URL.createObjectURL(blob);
      try {
        const link = document.createElement('a');
        link.href = url;
        link.download = name;
        document.body.appendChild(link);
        link.click();
        link.remove();
        if (openMs > 0) setTimeout(() => { try { window.open(url, '_blank'); } catch (_) {} }, openMs);
      } finally {
        setTimeout(() => URL.revokeObjectURL(url), revokeMs);
      }
    },
    // fullscreen(u8 on). v5: app-v5.js:toggleFullscreen
    [FX | 0xbc]: r => {
      const root = document.documentElement;
      const result = r.u8()
        ? (root.requestFullscreen || root.webkitRequestFullscreen)?.call(root)
        : (document.exitFullscreen || document.webkitExitFullscreen)?.call(document);
      result?.catch?.(() => {});
    },
    // dispatch_resize(): `window.dispatchEvent(new Event('resize'))`.
    [FX | 0xbd]: () => window.dispatchEvent(new Event('resize')),
    // navigate(u8 op, str url): 0 pushState, 1 replaceState, 2 back, 3 location.assign, 4 location.replace,
    // 5 reload, 6 window.open(url, '_blank').
    [FX | 0xc0]: r => {
      const op = r.u8(), url = r.str();
      if (op === 0) history.pushState(null, '', url);
      else if (op === 1) history.replaceState(null, '', url);
      else if (op === 2) history.back();
      else if (op === 3) location.assign(url);
      else if (op === 4) location.replace(url);
      else if (op === 5) location.reload();
      else if (op === 6) window.open(url, '_blank');
    }
  };
}
