// Application entry point: URL routing, welcome/home screens with the
// legacy visual shell, Android WebView detection and global error handling.

import { App } from './app';
import './assets/fonts/fonts.css';
import { BRAND_SVG, injectLegacyTheme } from './ui/legacyTheme';
import { buttonStyle, makeOverlay, promptText, showMessage, smallText } from './ui/modal';

const app = new App();

async function bootstrap(): Promise<void> {
  injectLegacyTheme();
  if (handleOAuthCallback()) return;

  const params = new URLSearchParams(window.location.search);
  const docId = params.get('docId');
  const fileId = params.get('fileId');
  const resourceKey = params.get('resourceKey');

  if (docId) {
    await app.open(docId);
    if (app.docId === null) await showHome();
  } else if (fileId) {
    await app.openDrive(fileId, resourceKey);
    if (app.docId === null) await showHome();
  } else {
    await showHome();
  }

  mountAppContainer();
  detectAndroidAppMode();
  setupErrorHandler();
}

function handleOAuthCallback(): boolean {
  if (!window.location.pathname.endsWith('/oauth-callback')) return false;
  const params = new URLSearchParams(window.location.search);
  const code = params.get('code');
  const error = params.get('error');
  if (code && window.opener) {
    window.opener.postMessage({ code }, window.location.origin);
    window.close();
    return true;
  }
  document.body.textContent = error ? `Google sign-in failed: ${error}` : 'Google sign-in did not return a code.';
  return true;
}

// ── Welcome + home (legacy shell) ──────────────────────────────────────────

async function showHome(): Promise<void> {
  const appEl = document.getElementById('app');
  if (!appEl) return;
  appEl.innerHTML = '';
  if (!app.isDriveSignedIn() && sessionStorage.getItem('ihn_offline_session') !== '1') {
    renderWelcome(appEl);
  } else {
    renderHome(appEl);
  }
}

function renderWelcome(root: HTMLElement): void {
  const view = document.createElement('div');
  view.id = 'welcome-view';
  view.innerHTML = `
    <div class="welcome-logo">${BRAND_SVG}<span class="brand-name">inhouse notes</span></div>`;
  const signIn = document.createElement('button');
  signIn.className = 'btn btn-primary';
  signIn.type = 'button';
  signIn.textContent = 'Sign in';
  signIn.addEventListener('click', () => openEntryOptions(root, view));
  view.appendChild(signIn);
  root.appendChild(view);
}

function openEntryOptions(root: HTMLElement, welcome: HTMLElement): void {
  const modal = makeOverlay('Open Inhouse Notes');
  modal.panel.style.width = 'min(460px, calc(100vw - 32px))';
  modal.body.appendChild(smallText('Choose where this session stores and syncs your notes.'));
  const actions = document.createElement('div');
  Object.assign(actions.style, { display: 'grid', gap: '10px', marginTop: '16px' });
  const google = document.createElement('button');
  google.type = 'button';
  google.textContent = 'Sign in with Google';
  Object.assign(google.style, buttonStyle('primary'));
  google.addEventListener('click', async () => {
    google.disabled = true;
    try {
      await app.signInDrive();
      modal.close();
      welcome.remove();
      renderHome(root);
    } catch (err) {
      google.disabled = false;
      showMessage('Sign in failed', err instanceof Error ? err.message : String(err));
    }
  });
  const offline = document.createElement('button');
  offline.type = 'button';
  offline.textContent = 'Continue offline';
  Object.assign(offline.style, buttonStyle());
  offline.addEventListener('click', () => {
    sessionStorage.setItem('ihn_offline_session', '1');
    modal.close();
    welcome.remove();
    renderHome(root);
  });
  actions.append(google, offline);
  modal.body.appendChild(actions);
  document.body.appendChild(modal.overlay);
}

function renderHome(root: HTMLElement): void {
  root.innerHTML = '';
  const home = document.createElement('div');
  home.id = 'drive-home';

  const topbar = document.createElement('div');
  topbar.className = 'drive-topbar';
  const brand = document.createElement('div');
  brand.className = 'drive-brand';
  brand.innerHTML = `${BRAND_SVG}<span>inhouse notes</span>`;
  topbar.appendChild(brand);

  const actions = document.createElement('div');
  actions.className = 'drive-topbar-actions';
  const search = document.createElement('input');
  search.className = 'drive-search';
  search.placeholder = 'Search in Drive';
  actions.appendChild(search);

  const newBtn = document.createElement('button');
  newBtn.className = 'btn btn-primary';
  newBtn.textContent = 'New notebook';
  newBtn.addEventListener('click', async () => {
    const name = await promptText('New notebook', {
      label: 'Document name',
      initialValue: 'cuaderno',
      confirmLabel: 'Create'
    });
    if (name) openDoc(await app.createNew(name));
  });
  actions.appendChild(newBtn);

  const pdfBtn = document.createElement('button');
  pdfBtn.className = 'btn';
  pdfBtn.type = 'button';
  pdfBtn.textContent = 'New PDF';
  pdfBtn.addEventListener('click', () => void pickPdf());
  actions.appendChild(pdfBtn);

  const auth = document.createElement('button');
  auth.className = 'btn';
  auth.textContent = app.isDriveSignedIn() ? 'Sign out' : 'Sign in';
  auth.addEventListener('click', async () => {
    try {
      if (app.isDriveSignedIn()) app.signOutDrive();
      else await app.signInDrive();
    } catch (err) {
      showMessage('Drive sign in failed', err instanceof Error ? err.message : String(err));
    }
    renderHome(root);
  });
  actions.appendChild(auth);
  topbar.appendChild(actions);
  home.appendChild(topbar);

  const body = document.createElement('div');
  body.className = 'drive-body';
  home.appendChild(body);

  const localTitle = document.createElement('div');
  localTitle.className = 'drive-section-title';
  localTitle.textContent = 'On this device';
  body.appendChild(localTitle);
  const localGrid = document.createElement('div');
  localGrid.className = 'drive-grid';
  body.appendChild(localGrid);

  void app.listDocuments().then((docs) => {
    if (docs.length === 0) {
      localGrid.replaceWith(empty('No documents on this device yet.'));
      return;
    }
    for (const doc of docs) {
      localGrid.appendChild(
        card(doc.name, `${doc.pageCount} page${doc.pageCount !== 1 ? 's' : ''} · ${shortDate(doc.savedAt)}`, () =>
          openDoc(doc.id)
        )
      );
    }
  });

  const driveTitle = document.createElement('div');
  driveTitle.className = 'drive-section-title';
  driveTitle.textContent = app.isDriveSignedIn() ? 'Google Drive' : 'Google Drive (optional)';
  body.appendChild(driveTitle);
  const driveArea = document.createElement('div');
  body.appendChild(driveArea);

  const loadDrive = async () => {
    driveArea.innerHTML = '';
    if (!app.isDriveSignedIn()) {
      driveArea.appendChild(empty('Offline mode is ready. Sign in only if you want Drive sync.'));
      return;
    }
    driveArea.appendChild(empty('Loading…'));
    try {
      const files = await app.listDriveDocuments(search.value);
      driveArea.innerHTML = '';
      if (files.length === 0) {
        driveArea.appendChild(empty('No Drive documents found.'));
        return;
      }
      const grid = document.createElement('div');
      grid.className = 'drive-grid';
      for (const file of files) {
        grid.appendChild(
          card(
            file.name.replace(/\.ihn\.json$/i, ''),
            `Drive · ${file.modifiedTime ? shortDate(Date.parse(file.modifiedTime)) : ''}`,
            () => {
              const url = new URL(window.location.href);
              url.searchParams.set('fileId', file.id);
              if (file.resourceKey) url.searchParams.set('resourceKey', file.resourceKey);
              window.location.href = url.toString();
            }
          )
        );
      }
      driveArea.appendChild(grid);
    } catch (err) {
      driveArea.innerHTML = '';
      driveArea.appendChild(empty(err instanceof Error ? err.message : String(err)));
    }
  };
  search.addEventListener('input', () => void loadDrive());
  void loadDrive();

  root.appendChild(home);
  mountPullToRefresh(home, topbar, body);
}

function mountPullToRefresh(home: HTMLElement, topbar: HTMLElement, scroller: HTMLElement): void {
  if (!window.matchMedia('(max-width: 700px)').matches) return;
  if (navigator.maxTouchPoints <= 0 && !('ontouchstart' in window)) return;

  const indicator = document.createElement('div');
  indicator.className = 'pull-refresh-indicator';
  indicator.setAttribute('role', 'status');
  indicator.setAttribute('aria-label', 'Pull to refresh');
  indicator.innerHTML = `
    <span class="pull-refresh-surface" aria-hidden="true">
      <span class="pull-refresh-spinner"></span>
    </span>`;
  home.appendChild(indicator);

  const threshold = 96;
  const maxOffset = 82;
  const axisLockThreshold = 8;
  const carouselVerticalIntentRatio = 1.25;
  let startX: number | null = null;
  let startY: number | null = null;
  let gestureAxis: 'pending' | 'horizontal' | 'vertical' | null = null;
  let startedInCarousel = false;
  let pullDistance = 0;
  let refreshing = false;
  let resetTimer: number | null = null;

  const updateTop = () => {
    home.style.setProperty('--pull-refresh-top', `${topbar.offsetHeight}px`);
  };
  updateTop();
  window.requestAnimationFrame(updateTop);

  const setPull = (distance: number) => {
    pullDistance = Math.max(0, distance);
    const progress = Math.min(1, pullDistance / threshold);
    const easedProgress = 1 - Math.pow(1 - progress, 3);
    const offset = maxOffset * (1 - Math.exp(-pullDistance / 78));
    home.style.setProperty('--pull-distance', `${offset}px`);
    home.style.setProperty('--pull-scale', String(0.84 + easedProgress * 0.16));
    home.style.setProperty('--pull-opacity', String(Math.min(1, progress * 2)));
    home.classList.toggle('is-pulling', pullDistance > 0);
    indicator.setAttribute('aria-label', progress >= 1 ? 'Release to refresh' : 'Pull to refresh');
  };

  const reset = () => {
    startX = null;
    startY = null;
    gestureAxis = null;
    startedInCarousel = false;
    pullDistance = 0;
    home.classList.add('is-pull-resetting');
    home.classList.remove('is-pulling');
    home.style.setProperty('--pull-distance', '0px');
    home.style.setProperty('--pull-scale', '0.84');
    home.style.setProperty('--pull-opacity', '0');
    indicator.setAttribute('aria-label', 'Pull to refresh');
    if (resetTimer !== null) window.clearTimeout(resetTimer);
    resetTimer = window.setTimeout(() => {
      home.classList.remove('is-pull-resetting');
      resetTimer = null;
    }, 220);
  };

  const refresh = () => {
    refreshing = true;
    startX = null;
    startY = null;
    gestureAxis = null;
    startedInCarousel = false;
    if (resetTimer !== null) window.clearTimeout(resetTimer);
    resetTimer = null;
    home.classList.add('is-pulling', 'is-refreshing');
    home.style.setProperty('--pull-distance', '52px');
    home.style.setProperty('--pull-scale', '1');
    home.style.setProperty('--pull-opacity', '1');
    indicator.setAttribute('aria-label', 'Refreshing');
    window.setTimeout(() => {
      home.classList.add('is-pull-resetting');
      home.style.setProperty('--pull-distance', '0px');
      home.style.setProperty('--pull-scale', '0.84');
      home.style.setProperty('--pull-opacity', '0');
      window.setTimeout(() => window.location.reload(), 220);
    }, 420);
  };

  scroller.addEventListener('touchstart', (event) => {
    if (refreshing || event.touches.length !== 1 || scroller.scrollTop > 0) return;
    const touch = event.touches[0]!;
    startX = touch.clientX;
    startY = touch.clientY;
    startedInCarousel = !!(
      event.target instanceof Element
      && event.target.closest('.drive-recents')
    );
    // Wait for the user's direction before assigning the gesture. Inside a
    // carousel, favour horizontal scrolling unless the pull is clearly vertical.
    gestureAxis = 'pending';
    pullDistance = 0;
    if (resetTimer !== null) window.clearTimeout(resetTimer);
    resetTimer = null;
    home.classList.remove('is-pull-resetting');
  }, { passive: true });

  scroller.addEventListener('touchmove', (event) => {
    if (refreshing || startX === null || startY === null || event.touches.length !== 1) return;
    if (scroller.scrollTop > 0) {
      reset();
      return;
    }
    const touch = event.touches[0]!;
    const deltaX = touch.clientX - startX;
    const deltaY = touch.clientY - startY;
    if (gestureAxis === 'pending') {
      const absoluteX = Math.abs(deltaX);
      const absoluteY = Math.abs(deltaY);
      if (Math.max(absoluteX, absoluteY) < axisLockThreshold) return;
      gestureAxis = startedInCarousel
        ? (absoluteY > absoluteX * carouselVerticalIntentRatio ? 'vertical' : 'horizontal')
        : (absoluteX > absoluteY ? 'horizontal' : 'vertical');
    }
    if (gestureAxis === 'horizontal') {
      if (pullDistance > 0) setPull(0);
      return;
    }
    const distance = deltaY;
    if (distance <= 0) {
      setPull(0);
      return;
    }
    event.preventDefault();
    setPull(distance);
  }, { passive: false });

  scroller.addEventListener('touchend', () => {
    if (refreshing || startY === null) return;
    if (gestureAxis !== 'vertical') {
      reset();
      return;
    }
    if (pullDistance >= threshold) refresh();
    else reset();
  }, { passive: true });

  scroller.addEventListener('touchcancel', () => {
    if (!refreshing) reset();
  }, { passive: true });
}

async function pickPdf(): Promise<void> {
  const input = document.createElement('input');
  input.type = 'file';
  input.accept = 'application/pdf';
  input.style.display = 'none';
  document.body.appendChild(input);
  input.addEventListener('change', async () => {
    const file = input.files?.[0];
    if (!file) {
      input.remove();
      return;
    }
    try {
      openDoc(await app.createFromPdf(file));
    } catch (err) {
      showMessage('PDF import failed', err instanceof Error ? err.message : String(err));
    } finally {
      input.remove();
    }
  }, { once: true });
  window.addEventListener('focus', () => {
    window.setTimeout(() => {
      if (!input.files?.length) input.remove();
    }, 500);
  }, { once: true });
  input.click();
}

function openDoc(docId: string): void {
  const url = new URL(window.location.href);
  url.searchParams.set('docId', docId);
  window.location.href = url.toString();
}

function card(name: string, meta: string, onOpen: () => void): HTMLButtonElement {
  const el = document.createElement('button');
  el.type = 'button';
  el.className = 'drive-card';
  const title = document.createElement('div');
  title.className = 'card-title';
  title.textContent = name;
  const sub = document.createElement('div');
  sub.className = 'card-meta';
  sub.textContent = meta;
  el.append(title, sub);
  el.addEventListener('click', onOpen);
  return el;
}

function empty(text: string): HTMLElement {
  const el = document.createElement('div');
  el.className = 'drive-empty';
  el.textContent = text;
  return el;
}

function shortDate(ts: number): string {
  if (!Number.isFinite(ts)) return '';
  return new Intl.DateTimeFormat(undefined, { month: 'short', day: '2-digit', hour: '2-digit', minute: '2-digit' }).format(new Date(ts));
}

// ── App container / platform glue ──────────────────────────────────────────

function mountAppContainer(): void {
  let el = document.getElementById('app');
  if (!el) {
    el = document.createElement('div');
    el.id = 'app';
    document.body.appendChild(el);
  }
  el.style.margin = '0';
  el.style.padding = '0';
  el.style.width = '100vw';
  el.style.height = '100vh';
  el.style.overflow = 'hidden';
}

function detectAndroidAppMode(): void {
  const ua = navigator.userAgent;
  if (/InhouseNotesApp|Android.*WebView/i.test(ua)) {
    document.documentElement.classList.add('android-app');
    document.addEventListener(
      'touchmove',
      (e) => {
        if (e.touches.length > 1) e.preventDefault();
      },
      { passive: false }
    );
  }
}

function setupErrorHandler(): void {
  window.addEventListener('error', (e) => {
    console.error('[uncaught error]', e.message, e.filename, e.lineno, e.colno);
  });
  window.addEventListener('unhandledrejection', (e) => {
    console.error('[unhandled rejection]', e.reason);
  });
}

document.addEventListener('DOMContentLoaded', bootstrap);
