// Visual shell ported from the legacy app (index.html §2): design tokens,
// fonts, welcome screen, drive-home look, and the floating toolbar style.
// The goal is that the rebuilt engine is indistinguishable from the
// original app's chrome.

export const BRAND_SVG = `<svg viewBox="0 0 40 24" fill="none" aria-hidden="true">
  <path d="M4 22 L20 6 L36 22" stroke="#E07A3C" stroke-width="4.2"
    stroke-linecap="round" stroke-linejoin="round" fill="none"/></svg>`;

const CSS = `
:root {
  --bg-primary: #f5f5f0;
  --bg-secondary: #ffffff;
  --text-primary: #1a1a1a;
  --text-secondary: #666666;
  --accent-yellow: #ffde00;
  --accent-blue: #002FD9;
  --accent-red: #E81010;
  --accent-black: #4D4D4D;
  --accent-orange: #E07A3C;
  --accent-green: #34c759;
  --shadow-soft: 0 2px 20px rgba(0,0,0,0.08);
  --shadow-medium: 0 4px 30px rgba(0,0,0,0.12);
  --radius-sm: 8px; --radius-md: 12px; --radius-lg: 16px;
  --tool-size: 44px;
}
[data-theme="dark"] {
  --bg-primary: #151515;
  --bg-secondary: #1c1c1c;
  --text-primary: #f5f5f5;
  --text-secondary: #a2a2a2;
  --shadow-soft: 0 2px 20px rgba(0,0,0,0.35);
  --shadow-medium: 0 4px 30px rgba(0,0,0,0.45);
}
html, body {
  margin: 0; height: 100%; overflow: hidden;
  font-family: 'DM Sans', -apple-system, BlinkMacSystemFont, sans-serif;
  background: var(--bg-primary); color: var(--text-primary);
}
.btn {
  border: none; cursor: pointer; font-family: inherit; font-weight: 600;
  border-radius: var(--radius-md); padding: 10px 22px; font-size: 0.95rem;
  background: var(--bg-secondary); color: var(--text-primary);
  box-shadow: var(--shadow-soft); transition: transform 0.12s, box-shadow 0.12s;
}
.btn:hover { transform: translateY(-1px); box-shadow: var(--shadow-medium); }
.btn-primary { background: #f5f5f5; color: #1a1a1a; }

/* ── Welcome (legacy §2.4) ─────────────────────────────────────────────── */
#welcome-view {
  position: fixed; inset: 0; background: #151515;
  display: flex; flex-direction: column; align-items: center;
  justify-content: center; gap: 56px; z-index: 1300; padding: 48px 32px;
}
#welcome-view.hidden { display: none; }
#welcome-view .welcome-logo { display: flex; flex-direction: column; align-items: center; gap: 24px; }
#welcome-view .welcome-logo svg { width: 180px; height: 108px; }
#welcome-view .welcome-logo .brand-name {
  font-family: 'Comfortaa', cursive; font-size: 2.25rem; font-weight: 600;
  color: #f5f5f5; letter-spacing: 0.5px;
}
#welcome-view .btn { min-height: 48px; padding: 12px 40px; font-size: 1.05rem; }

/* ── Home (legacy drive-home look, §2.5) ───────────────────────────────── */
#drive-home {
  position: fixed; inset: 0; background: var(--bg-primary);
  display: flex; flex-direction: column; overflow: hidden;
  overscroll-behavior-y: none;
  --pull-distance: 0px; --pull-progress: 0deg;
  --pull-scale: 0.78; --pull-opacity: 0;
  --pull-arrow-rotation: 0deg; --pull-refresh-top: 64px;
}
.drive-topbar {
  display: flex; align-items: center; justify-content: space-between;
  padding: 12px 20px; min-height: 64px; background: var(--bg-secondary);
  border-bottom: 1px solid rgba(0,0,0,0.06); flex-shrink: 0;
}
.drive-brand {
  font-family: 'Comfortaa', cursive; font-size: 1.25rem; font-weight: 600;
  color: var(--text-primary); display: flex; align-items: center; gap: 10px;
}
.drive-brand svg { width: 32px; height: 24px; }
.drive-brand span { letter-spacing: -0.5px; }
.drive-topbar-actions { display: flex; gap: 10px; align-items: center; }
.drive-body {
  flex: 1; overflow-y: auto; padding: 22px clamp(16px, 4vw, 42px);
  overscroll-behavior-y: contain;
  transform: translateY(var(--pull-distance));
  will-change: transform;
}
.pull-refresh-indicator {
  position: absolute; z-index: 5; pointer-events: none;
  left: 50%; top: var(--pull-refresh-top);
  width: 48px; height: 48px; display: grid; place-items: center;
  opacity: var(--pull-opacity);
  transform: translate3d(-50%, calc(var(--pull-distance) - 108%), 0) scale(var(--pull-scale));
  transform-origin: center; will-change: transform, opacity;
}
.pull-refresh-surface {
  position: relative; width: 44px; height: 44px; border-radius: 50%;
  display: grid; place-items: center; color: var(--accent-orange);
  background: rgba(102,102,102,0.16);
  background: conic-gradient(
    from -90deg,
    var(--accent-orange) 0deg var(--pull-progress),
    rgba(102,102,102,0.16) var(--pull-progress) 360deg
  );
  box-shadow: 0 8px 24px rgba(20,20,20,0.16), 0 2px 6px rgba(20,20,20,0.08);
  transition: box-shadow 0.24s ease, color 0.24s ease;
}
.pull-refresh-surface::before {
  content: ''; position: absolute; inset: 2px; border-radius: 50%;
  background: var(--bg-secondary); box-shadow: inset 0 0 0 1px rgba(0,0,0,0.035);
}
.pull-refresh-arrow, .pull-refresh-spinner {
  position: absolute; z-index: 1; left: 50%; top: 50%;
}
.pull-refresh-arrow {
  width: 16px; height: 16px; margin: -8px 0 0 -8px;
  transform: rotate(var(--pull-arrow-rotation));
  transition: transform 0.28s cubic-bezier(0.22,1,0.36,1), opacity 0.16s ease;
}
.pull-refresh-arrow::before {
  content: ''; position: absolute; left: 7px; top: 1px;
  width: 2px; height: 12px; border-radius: 2px; background: currentColor;
}
.pull-refresh-arrow::after {
  content: ''; position: absolute; left: 4px; bottom: 1px;
  width: 7px; height: 7px; border-right: 2px solid currentColor;
  border-bottom: 2px solid currentColor; border-radius: 1px; transform: rotate(45deg);
}
.pull-refresh-spinner {
  width: 18px; height: 18px; margin: -9px 0 0 -9px;
  border-radius: 50%; opacity: 0; transform: scale(0.64);
  transition: opacity 0.16s ease, transform 0.24s cubic-bezier(0.22,1,0.36,1);
}
.pull-refresh-spinner::before {
  content: ''; position: absolute; inset: 0; border-radius: 50%;
  border: 2px solid rgba(255,255,255,0.42); border-top-color: #fff;
}
#drive-home.is-refresh-ready .pull-refresh-surface {
  box-shadow: 0 10px 28px rgba(224,122,60,0.28), 0 0 0 5px rgba(224,122,60,0.1);
}
#drive-home.is-pull-resetting .drive-body,
#drive-home.is-pull-resetting .pull-refresh-indicator {
  transition: transform 0.44s cubic-bezier(0.22,1,0.36,1), opacity 0.24s ease;
}
#drive-home.is-refreshing .drive-body,
#drive-home.is-refreshing .pull-refresh-indicator {
  transition: transform 0.34s cubic-bezier(0.22,1,0.36,1), opacity 0.18s ease;
}
#drive-home.is-refreshing .pull-refresh-surface {
  color: #fff; background: var(--accent-orange);
  box-shadow: 0 10px 30px rgba(224,122,60,0.32), 0 0 0 5px rgba(224,122,60,0.1);
  animation: pull-refresh-breathe 1.1s ease-in-out infinite;
}
#drive-home.is-refreshing .pull-refresh-surface::before {
  background: var(--accent-orange); box-shadow: none;
}
#drive-home.is-refreshing .pull-refresh-arrow {
  opacity: 0; transform: rotate(var(--pull-arrow-rotation)) scale(0.58);
}
#drive-home.is-refreshing .pull-refresh-spinner {
  opacity: 1; transform: scale(1);
}
#drive-home.is-refreshing .pull-refresh-spinner::before {
  animation: pull-refresh-spin 0.68s linear infinite;
}
@keyframes pull-refresh-spin { to { transform: rotate(360deg); } }
@keyframes pull-refresh-breathe { 50% { transform: scale(1.035); } }
.drive-section-title {
  font-size: 0.85rem; font-weight: 600; color: var(--text-secondary);
  text-transform: uppercase; letter-spacing: 0.8px; margin: 18px 0 12px;
}
.drive-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(190px, 1fr)); gap: 14px; }
.drive-card {
  display: flex; flex-direction: column; gap: 6px; min-height: 96px;
  padding: 16px; text-align: left; border: none; cursor: pointer;
  border-radius: var(--radius-lg); background: var(--bg-secondary);
  box-shadow: var(--shadow-soft); color: var(--text-primary);
  font-family: inherit; transition: transform 0.12s, box-shadow 0.12s;
}
.drive-card:hover { transform: translateY(-2px); box-shadow: var(--shadow-medium); }
.drive-card .card-title { font-size: 0.98rem; font-weight: 600; overflow-wrap: anywhere; }
.drive-card .card-meta { font-size: 0.8rem; color: var(--text-secondary); }
.drive-search {
  height: 40px; min-width: min(320px, 50vw); border: none; border-radius: var(--radius-md);
  padding: 0 16px; background: var(--bg-primary); color: var(--text-primary);
  font-family: inherit; font-size: 0.9rem; box-shadow: inset 0 0 0 1px rgba(0,0,0,0.07);
}
.drive-empty { color: var(--text-secondary); font-size: 0.9rem; padding: 8px 0; }

/* ── Editor chrome: original floating toolbar (legacy §2.8) ────────────── */
/* !important: the editor builds buttons with inline styles; the legacy skin
   wins until that inline styling is removed (tracked tech debt). */
#app-viewport { background: var(--bg-primary) !important; }
.toolbar {
  flex-direction: column !important;
  left: 18px !important; right: auto !important;
  top: 50% !important; bottom: auto !important;
  transform: translateY(-50%) !important;
  background: var(--bg-secondary) !important;
  border-radius: var(--radius-lg) !important;
  box-shadow: var(--shadow-medium) !important;
  padding: 10px 8px !important; gap: 6px !important;
}
.toolbar button {
  width: var(--tool-size) !important; height: var(--tool-size) !important;
  border-radius: var(--radius-md) !important; font-size: 19px !important;
  color: var(--text-primary) !important;
}
.toolbar [data-tool].active { background: var(--accent-yellow) !important; }
.toolbar input[type="range"] { width: 44px !important; }
.toolbar [data-drag-handle] { cursor: grab; height: 24px !important; font-size: 12px !important; }
.toolbar button[title="Toggle theme"], .toolbar button[title="Insert image"] { font-size: 11px !important; font-weight: 700; }
.editor-action-bar {
  border: none !important; border-radius: var(--radius-md) !important;
  background: var(--bg-secondary) !important; box-shadow: var(--shadow-soft) !important;
  font-family: inherit !important;
}
.editor-action-bar button {
  border: none !important; background: transparent !important;
  color: var(--text-primary) !important; font-weight: 600 !important;
  border-radius: var(--radius-sm) !important;
}
.editor-action-bar button:hover { background: var(--bg-primary) !important; }
.sync-status-bar { font-family: inherit !important; border-radius: var(--radius-md) !important; box-shadow: var(--shadow-soft) !important; }

@media (max-width: 700px) {
  .drive-topbar {
    display: grid; grid-template-columns: 1fr; gap: 10px;
    padding: 10px 14px;
  }
  .drive-topbar-actions {
    display: grid; grid-template-columns: repeat(3, minmax(0, 1fr));
    width: 100%; min-width: 0; gap: 8px;
  }
  .drive-search {
    grid-column: 1 / -1; box-sizing: border-box;
    width: 100%; min-width: 0;
  }
  .drive-topbar-actions .btn {
    min-width: 0; padding: 9px 6px; font-size: 0.82rem;
    white-space: nowrap;
  }
  .editor-action-bar {
    max-width: calc(100vw - 36px) !important;
    overflow-x: auto !important;
    overflow-y: hidden !important;
    flex-wrap: nowrap !important;
    scrollbar-width: none;
  }
  .editor-action-bar::-webkit-scrollbar { display: none; }
  .sync-status-bar { top: 68px !important; }
  .toolbar {
    top: calc(50% + 32px) !important;
    max-height: calc(100vh - 144px) !important;
    overflow-y: auto !important;
    scrollbar-width: none;
  }
  .toolbar::-webkit-scrollbar { display: none; }
}
@media (prefers-reduced-motion: reduce) {
  .pull-refresh-surface,
  .pull-refresh-arrow,
  .pull-refresh-spinner,
  .pull-refresh-indicator,
  #drive-home .drive-body { transition-duration: 0.01ms !important; }
  #drive-home.is-refreshing .pull-refresh-surface { animation: none; }
  #drive-home.is-refreshing .pull-refresh-spinner::before { animation-duration: 1.2s; }
}
`;

let injected = false;

export function injectLegacyTheme(): void {
  if (injected) return;
  const style = document.createElement('style');
  style.dataset.ihnLegacyTheme = '1';
  style.textContent = CSS;
  document.head.appendChild(style);
  injected = true;
}
