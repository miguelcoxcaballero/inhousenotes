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
  --pull-distance: 0px; --pull-progress: 0;
  --pull-rotation: 0deg; --pull-refresh-top: 64px;
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
  opacity: 0;
  transform: translate(-50%, calc(var(--pull-distance) - 100%));
  transition: opacity 0.14s ease;
}
.pull-refresh-icon {
  width: 38px; height: 38px; border-radius: 50%;
  display: grid; place-items: center;
  background: var(--bg-secondary); box-shadow: var(--shadow-medium);
  transition: background 0.16s ease, transform 0.16s ease;
}
.pull-refresh-icon::before {
  content: ''; width: 15px; height: 15px; border-radius: 50%;
  border: 2px solid rgba(102,102,102,0.25);
  border-top-color: var(--accent-orange);
  transform: rotate(var(--pull-rotation));
}
#drive-home.is-pulling .pull-refresh-indicator { opacity: 1; }
#drive-home.is-refresh-ready .pull-refresh-icon {
  background: var(--accent-orange);
  transform: scale(1.08);
}
#drive-home.is-refresh-ready .pull-refresh-icon::before {
  border-color: rgba(255,255,255,0.42);
  border-top-color: #fff;
}
#drive-home.is-pull-resetting .drive-body,
#drive-home.is-pull-resetting .pull-refresh-indicator,
#drive-home.is-refreshing .drive-body,
#drive-home.is-refreshing .pull-refresh-indicator {
  transition: transform 0.2s ease-out, opacity 0.14s ease;
}
#drive-home.is-refreshing .pull-refresh-icon::before {
  animation: pull-refresh-spin 0.65s linear infinite;
}
@keyframes pull-refresh-spin { to { transform: rotate(360deg); } }
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
  .pull-refresh-icon,
  .pull-refresh-indicator,
  #drive-home .drive-body { transition-duration: 0.01ms !important; }
  #drive-home.is-refreshing .pull-refresh-icon::before { animation-duration: 1.2s; }
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
