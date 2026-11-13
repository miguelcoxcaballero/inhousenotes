// Capture matrix and browser settings of the parity harness (TESTING §3.1, §3.3).
import fs from 'node:fs';

/** Fixed start of time for every scripted state. */
export const EPOCH = Date.parse('2026-10-30T10:00:00.000Z');
export const SEED = 0x1badc0de;

// Software raster, fixed colour profile and font rendering. Proven stable by `run.mjs --self` (5.18.0 against itself,
// two runs, 0 differing pixels): see docs/v6/NOTES-W1.2.md.
export const CHROMIUM_FLAGS = [
  // raster: software, one colour profile, no LCD text, no hinting
  '--disable-gpu', '--force-color-profile=srgb', '--font-render-hinting=none', '--disable-lcd-text', '--disable-font-subpixel-positioning',
  // a frame is drawn only when every stage is complete (no checkerboarded images, no low-resolution tiles)
  '--run-all-compositor-stages-before-draw', '--disable-checker-imaging', '--disable-low-res-tiling', '--disable-partial-raster',
  '--disable-new-content-rendering-timeout', '--disable-image-animation-resync', '--num-raster-threads=1',
  // nothing moves on the compositor thread behind the main thread's back
  '--disable-threaded-animation', '--disable-threaded-scrolling', '--disable-smooth-scrolling',
  '--disable-features=PaintHolding,Translate,MediaRouter,OptimizationHints', '--mute-audio'
];

/** The pinned Playwright headless shell; never downloaded (`playwright install` is forbidden here). */
export function chromiumExecutable() {
  const candidates = [process.env.IHN_CHROMIUM, '/opt/pw-browsers/chromium_headless_shell-1194/chrome-linux/headless_shell'].filter(Boolean);
  return candidates.find(file => fs.existsSync(file)) || undefined;
}

const UA_CHROME = 'AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0';
const UA = {
  desktop: `Mozilla/5.0 (X11; Linux x86_64) ${UA_CHROME} Safari/537.36`,
  phone: `Mozilla/5.0 (Linux; Android 14; SM-S911B) ${UA_CHROME} Mobile Safari/537.36`,
  tablet: `Mozilla/5.0 (Linux; Android 14; SM-X700) ${UA_CHROME} Safari/537.36`
};

export const VIEWPORTS = {
  'phone-360': { width: 360, height: 800, dpr: 3, kind: 'phone', touch: true },
  'phone-412': { width: 412, height: 915, dpr: 2.625, kind: 'phone', touch: true },
  'tablet-800': { width: 800, height: 1280, dpr: 2, kind: 'tablet', touch: true },
  'desktop-1280': { width: 1280, height: 800, dpr: 1, kind: 'desktop', touch: false },
  'desktop-1920': { width: 1920, height: 1080, dpr: 2, kind: 'desktop', touch: false }
};
export const THEMES = ['light', 'dark', 'system-dark'];
export const MODES = ['web', 'app'];
export const APP_VERSION_NAME = '1.0.28';
export const APP_VERSION_CODE = 29;

/** Playwright context options for one matrix cell. */
export function contextOptions(cell) {
  const v = VIEWPORTS[cell.viewport];
  const userAgent = UA[v.kind] + (cell.mode === 'app' ? ` InhouseNotesApp/${APP_VERSION_NAME}` : '');
  return {
    viewport: { width: v.width, height: v.height }, deviceScaleFactor: v.dpr, hasTouch: v.touch, isMobile: v.kind !== 'desktop',
    userAgent, locale: 'en-US', timezoneId: 'UTC', colorScheme: cell.theme === 'light' ? 'light' : 'dark', reducedMotion: 'no-preference',
    serviceWorkers: 'block', bypassCSP: false
  };
}
/** localStorage seeded for the theme axis (`inhouse-theme`); system-dark stores nothing. */
export const themeStorage = cell => cell.theme === 'system-dark' ? {} : { 'inhouse-theme': cell.theme };
/** Query added to the app URL for the mode axis. */
export const modeQuery = cell => cell.mode === 'app' ? '&inhouse_app=1' : '';
export const cellId = cell => `${cell.viewport}.${cell.theme}.${cell.mode}`;

/**
 * Cells a state runs in. `quick` = desktop 1280×800 + phone 360×800, light, web (what each unit runs before merging).
 * A state may narrow the axes with `only: {viewports, themes, modes}`; system-dark applies to `systemDark: true` states
 * (Home and Welcome), app mode to `app: true` states.
 */
export function cellsFor(state, { quick = false } = {}) {
  const only = state.only || {};
  const viewports = (quick ? ['desktop-1280', 'phone-360'] : Object.keys(VIEWPORTS)).filter(v => !only.viewports || only.viewports.includes(v));
  let themes = quick ? ['light'] : ['light', 'dark', ...(state.systemDark ? ['system-dark'] : [])];
  let modes = quick ? ['web'] : ['web', ...(state.app ? ['app'] : [])];
  if (only.themes) themes = only.themes.slice();
  if (only.modes) modes = only.modes.slice();
  const cells = [];
  for (const mode of modes) for (const theme of themes) for (const viewport of viewports) cells.push({ viewport, theme, mode });
  return cells;
}

// `window.InhouseNative` of the Android host (spec/persistence §3.6): records every call in `__ihnNativeCalls` and
// answers the getters. Installed only in app mode, identically for both apps.
export const NATIVE_STUB = `(() => {
  if (window.InhouseNative) return;
  const calls = window.__ihnNativeCalls = [];
  const record = name => (...args) => { calls.push([name, ...args.map(a => typeof a === 'string' && a.length > 256 ? a.slice(0, 64) + '…(' + a.length + ')' : a)]); };
  let updateState = null;
  window.InhouseNative = {
    getAppVersion: () => '${APP_VERSION_NAME}', getAppVersionCode: () => ${APP_VERSION_CODE}, getInstallState: () => '',
    getUpdateState: () => updateState, saveUpdateState: json => { updateState = String(json); calls.push(['saveUpdateState', updateState]); },
    openAuthUrl: record('openAuthUrl'), installInhouseUpdate: record('installInhouseUpdate'), installAppUpdate: record('installAppUpdate'),
    beginPdfExport: record('beginPdfExport'), appendPdfExportChunk: record('appendPdfExportChunk'), finishPdfExport: record('finishPdfExport')
  };
})();`;
