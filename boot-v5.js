(function bootInhouseNotes(root, documentRef) {
    'use strict';

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
            root.__INHOUSE_BOOT_SIGNED_IN__ = hasToken || hasRefresh;
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

    applySavedTheme();
    handleOAuthCallback();
    selectBootDestination();
    if (documentRef.readyState === 'loading') {
        documentRef.addEventListener('DOMContentLoaded', revealInitialView, { once: true });
    } else {
        revealInitialView();
    }
})(window, document);
