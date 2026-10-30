(function bootInhouseNotes(root, documentRef) {
    'use strict';

    let nativeAssets={};
    try {nativeAssets=JSON.parse(documentRef.getElementById('inhouse-native-assets')?.textContent||'{}');}catch(_){}
    root.InhouseAssetUrl=path=>{
        const url=new URL(path,root.location.href),digest=nativeAssets[url.pathname];
        if(url.origin===root.location.origin&&digest)url.searchParams.set('native',digest);
        return url.href;
    };

    function applySavedTheme() {
        let dark = false;
        try {
            const storedTheme = localStorage.getItem('inhouse-theme');
            dark = storedTheme === 'dark'
                || (!storedTheme && root.matchMedia('(prefers-color-scheme: dark)').matches);
        } catch (error) {
            dark = !!(root.matchMedia && root.matchMedia('(prefers-color-scheme: dark)').matches);
        }
        const theme = dark ? 'dark' : 'light';
        const color = dark ? '#151515' : '#f5f5f0';
        documentRef.documentElement.dataset.theme = theme;
        documentRef.documentElement.style.backgroundColor = color;
        documentRef.getElementById('app-theme-color')?.setAttribute('content', color);
    }

    function handleOAuthCallback() {
        const search = root.location.search || '';
        const hash = root.location.hash || '';
        if (!search.includes('oauth_callback=1')) return;
        const hasToken = hash.includes('access_token=');
        const errorMatch = search.match(/[?&]error=([^&]*)/);
        if (hasToken) {
            root.location.replace(`inhousenotes://oauth2callback${hash}`);
        } else if (errorMatch) {
            root.location.replace(`inhousenotes://oauth2callback#error=${errorMatch[1]}`);
        }
        setTimeout(() => {
            const target = new URL('/', root.location.origin);
            target.searchParams.set('inhouse_app', '1');
            target.hash = hash || '';
            root.location.replace(target.toString());
        }, 500);
    }

    function selectBootDestination() {
        try {
            const raw = localStorage.getItem('drive-token-v1');
            const hasCurrentScopes = localStorage.getItem('google-scope-version-v1') === 'drive-calendar-v2';
            const grantedScopes = localStorage.getItem('google-granted-scopes-v1') || '';
            const hasCalendarScope = grantedScopes.split(/\s+/)
                .includes('https://www.googleapis.com/auth/calendar.events');
            let hasToken = false;
            if (raw) {
                const parsed = JSON.parse(raw);
                hasToken = !!(hasCurrentScopes && parsed?.token && parsed?.expiry
                    && parsed.scopeVersion === 'drive-calendar-v2'
                    && String(parsed.grantedScopes || '').split(/\s+/)
                        .includes('https://www.googleapis.com/auth/calendar.events')
                    && Date.now() < parsed.expiry - 60000);
            }
            const hasRefresh = hasCurrentScopes
                && hasCalendarScope
                && !!localStorage.getItem('drive-refresh-token-v1');
            const returning = localStorage.getItem('drive-remember-v1') === '1' || !!localStorage.getItem('drive-hint-email');
            root.__INHOUSE_BOOT_SIGNED_IN__ = hasToken || hasRefresh || returning;
        } catch (error) {
            root.__INHOUSE_BOOT_SIGNED_IN__ = false;
        }
        documentRef.documentElement.dataset.bootDestination = root.__INHOUSE_BOOT_SIGNED_IN__
            ? 'drive'
            : 'welcome';
    }

    function revealInitialView() {
        const welcome = documentRef.getElementById('welcome-view');
        const home = documentRef.getElementById('drive-home');
        if (welcome) {
            if (root.__INHOUSE_BOOT_SIGNED_IN__) welcome.classList.add('hidden');
            const reveal = () => welcome.classList.add('fonts-ready');
            if (documentRef.fonts?.load) documentRef.fonts.load('600 2.25rem Comfortaa').then(reveal, reveal);
            else reveal();
            setTimeout(reveal, 3000);
        }
        if (!root.__INHOUSE_BOOT_SIGNED_IN__) return;
        welcome?.classList.add('hidden');
        home?.classList.remove('hidden');

        const cardMarkup = '<div class="drive-card-skeleton" aria-hidden="true"><div class="skeleton-bar title"></div><div class="skeleton-preview"></div></div>';
        const folderMarkup = '<div class="drive-folder-card-skeleton" aria-hidden="true"><div class="skeleton-icon"></div><div class="skeleton-bar"></div></div>';
        const fill = (id, markup, count) => {
            const container = documentRef.getElementById(id);
            if (container) container.innerHTML = new Array(count + 1).join(markup);
        };
        fill('drive-recents', cardMarkup, 6);
        fill('drive-starred', cardMarkup, 6);
        fill('drive-shared', cardMarkup, 6);
        fill('drive-folders', folderMarkup, 4);
        fill('drive-folder-files', cardMarkup, 8);
        const status = documentRef.getElementById('drive-status');
        if (status) {
            status.innerHTML = '<span class="drive-status-skeleton" aria-hidden="true"></span>';
            status.setAttribute('aria-label', 'Loading your Drive');
        }
    }

    // ── Published release check ──
    // The browser can keep showing an older release after a reload: GitHub
    // Pages answers with HTTP caching, and an older service worker serves the
    // page from its offline copy. Ask the live site which release is
    // published (service-worker.js carries it), bypassing every cache. When
    // it is newer than the running one, drop the old offline copy of the
    // pages and reload, once per release per tab, and never while a document
    // is open in the editor (that waits until the tab is next shown).
    function readAppVersion() {
        const source = documentRef.getElementById('inhouse-app-version')?.textContent || '';
        return source.match(/APP_VERSION\s*=\s*['"](\d+\.\d+\.\d+)['"]/)?.[1] || '';
    }

    function compareVersions(a, b) {
        const x = a.split('.').map(Number);
        const y = b.split('.').map(Number);
        for (let i = 0; i < 3; i += 1) {
            if (x[i] !== y[i]) return x[i] - y[i];
        }
        return 0;
    }

    function watchPublishedVersion() {
        const running = readAppVersion();
        if (!running || typeof root.fetch !== 'function') return;
        if ((root.location.search || '').includes('oauth_callback')) return;
        // A hidden document processor finishes its run with the code it
        // loaded; the visible app owns release checks and reloads.
        if (new URLSearchParams(root.location.search || '').has('offline-worker')) return;
        const RELOAD_KEY = 'ihn-release-reload';
        let checking = false;
        let lastCheck = 0;

        const isEditing = () => {
            const editor = documentRef.getElementById('editor-view');
            return !!editor && !editor.classList.contains('hidden');
        };

        const reloadInto = async (published) => {
            try {
                if (root.sessionStorage.getItem(RELOAD_KEY) === published) return;
                root.sessionStorage.setItem(RELOAD_KEY, published);
            } catch (error) {
                return; // Without a guard against reload loops, do nothing.
            }
            try {
                const registration = await root.navigator.serviceWorker?.getRegistration();
                await registration?.update();
            } catch (error) { /* the reload below still revalidates the page */ }
            try {
                for (const key of await root.caches.keys()) {
                    if (!key.startsWith('inhouse-shell-') || key === `inhouse-shell-${published}`) continue;
                    const cache = await root.caches.open(key);
                    await cache.delete('/index.html');
                }
            } catch (error) { /* no Cache Storage: nothing stale to drop */ }
            root.location.reload();
        };

        const check = async () => {
            if (checking || root.navigator.onLine === false) return;
            checking = true;
            lastCheck = Date.now();
            try {
                const response = await root.fetch(`/service-worker.js?release-check=${Date.now()}`, { cache: 'no-store' });
                if (!response.ok) return;
                const published = (await response.text()).match(/const VERSION='(\d+\.\d+\.\d+)'/)?.[1] || '';
                if (published && compareVersions(published, running) > 0 && !isEditing()) await reloadInto(published);
            } catch (error) {
                // Offline or blocked: keep the running release.
            } finally {
                checking = false;
            }
        };

        const start = () => setTimeout(check, 1200);
        if (documentRef.readyState === 'complete') start();
        else root.addEventListener('load', start, { once: true });
        documentRef.addEventListener('visibilitychange', () => {
            if (documentRef.visibilityState === 'visible' && Date.now() - lastCheck > 60000) void check();
        });
        root.addEventListener('pageshow', (event) => {
            if (event.persisted) void check();
        });
    }

    if (root.InhouseBackgroundChannel?.postMessage) {
        const send = (type, result) => root.InhouseBackgroundChannel.postMessage(JSON.stringify({type,result}));
        root.InhouseBackground = Object.freeze({schedule:()=>send('schedule'),cancel:()=>send('cancel'),complete:value=>send('complete',JSON.parse(value))});
    }
    applySavedTheme();
    handleOAuthCallback();
    selectBootDestination();
    watchPublishedVersion();
    if (documentRef.readyState === 'loading') {
        let revealed = false;
        const observer = new MutationObserver(() => {
            if (!revealed && documentRef.getElementById('drive-folder-files')) {
                revealed = true; revealInitialView(); observer.disconnect();
            }
        });
        observer.observe(documentRef.documentElement,{childList:true,subtree:true});
        documentRef.addEventListener('DOMContentLoaded', () => { observer.disconnect(); if (!revealed) revealInitialView(); }, { once: true });
    } else {
        revealInitialView();
    }
})(window, document);
