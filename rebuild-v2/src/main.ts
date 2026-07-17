// Application entry point: URL routing, welcome/home screens with the
// legacy visual shell, Android WebView detection and global error handling.

import { App } from './app';
import { BRAND_SVG, injectLegacyTheme } from './ui/legacyTheme';

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
  renderHome(appEl);
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
  signIn.addEventListener('click', async () => {
    try {
      await app.signInDrive();
      view.remove();
      renderHome(root);
    } catch (err) {
      window.alert(err instanceof Error ? err.message : String(err));
    }
  });
  view.appendChild(signIn);

  const local = document.createElement('button');
  local.className = 'btn';
  local.type = 'button';
  local.textContent = 'Use without account';
  local.addEventListener('click', async () => {
    const newId = await app.createNew('cuaderno');
    openDoc(newId);
  });
  view.appendChild(local);
  root.appendChild(view);
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
    const name = window.prompt('Document name:', 'cuaderno');
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
      window.alert(err instanceof Error ? err.message : String(err));
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
}

async function pickPdf(): Promise<void> {
  const input = document.createElement('input');
  input.type = 'file';
  input.accept = 'application/pdf';
  input.addEventListener('change', async () => {
    const file = input.files?.[0];
    if (!file) return;
    try {
      openDoc(await app.createFromPdf(file));
    } catch (err) {
      window.alert(err instanceof Error ? err.message : String(err));
    }
  });
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
