import { test, expect } from '@playwright/test';

const folderType = 'application/vnd.google-apps.folder';
const entries = [
    { id: 'projects', name: 'Proyectos', mimeType: folderType, parents: ['root'] },
    { id: 'travel', name: 'Viajes', mimeType: folderType, parents: ['root'], folderColorRgb: '#4986e7' },
    { id: 'drafts', name: 'Borradores', mimeType: folderType, parents: ['projects'] },
    { id: 'project-note', name: 'Proyecto.pdf', mimeType: 'application/pdf', parents: ['projects'] },
    { id: 'deep', name: 'Dentro', mimeType: folderType, parents: ['drafts'] },
    { id: 'deep-note', name: 'Dentro.pdf', mimeType: 'application/pdf', parents: ['deep'] },
    { id: 'draft-note', name: 'Borrador.pdf', mimeType: 'application/pdf', parents: ['drafts'] },
    { id: 'trip-note', name: 'Viaje.pdf', mimeType: 'application/pdf', parents: ['travel'] }
];

async function setup(page, context, reducedMotion = false) {
    await page.emulateMedia({ reducedMotion: reducedMotion ? 'reduce' : 'no-preference' });
    await page.route('https://accounts.google.com/**', route => route.fulfill({ status: 200, body: '' }));
    await page.addInitScript(() => {
        localStorage.setItem('drive-remember-v1', '1');
        localStorage.setItem('drive-hint-email', 'folders@example.test');
    });
    await page.goto('/?e2e=1');
    await page.evaluate(async entries => {
        await __IHN_TEST_API__.ready();
        stopDriveHomeAutoRefresh();
        await InhouseOfflineLibrary.observed(entries);
        setDriveHomeTab('files');
        InhouseOfflineApp.renderFolder();
    }, entries);
    await context.setOffline(true);
}

async function openFolder(page, id, container = '#drive-folders') {
    await page.locator(`${container} [data-drive-item-id="${id}"]`).click();
}

async function settle(page) {
    await page.waitForFunction(() => !driveFolderAnimation);
}

test('cached folders appear during opening; nested navigation and closing retain content until it fades', async ({ page, context }) => {
    await setup(page, context);
    await openFolder(page, 'projects');
    await expect(page.locator('#drive-view-files')).toContainText('Proyecto');
    await expect(page.locator('#drive-view-files .drive-card-skeleton')).toHaveCount(0);
    await settle(page);
    await openFolder(page, 'drafts', '#drive-view-folders');
    await expect(page.locator('#drive-folder-view-title')).toHaveText('Borradores');
    await expect(page.locator('#drive-view-files')).toContainText('Borrador');
    await settle(page);
    await page.locator('#btn-folder-view-back').click();
    await expect(page.locator('#drive-folder-view-title')).toHaveText('Proyectos');
    await settle(page);
    // Start from a real UI event and sample before the first exit frame.
    const closing = await page.evaluate(() => {
        document.getElementById('btn-folder-view-back').click();
        const view = document.getElementById('drive-folder-view');
        return { visible: !view.classList.contains('hidden'), text: view.textContent, effects: view.getAnimations().length };
    });
    expect(closing.visible).toBe(true);
    expect(closing.text).toContain('Proyecto');
    expect(closing.effects).toBeGreaterThan(0);
    await settle(page);
    await expect(page.locator('#drive-folder-view')).toBeHidden();
    await expect(page.locator('.drive-folder-ghost')).toHaveCount(0);
    await expect(page.locator('#drive-folders [data-drive-item-id="projects"]')).toBeFocused();
});

test('rapid back, reopen and tab changes cannot leave a stale folder, hidden card or animation', async ({ page, context }) => {
    await setup(page, context);
    await openFolder(page, 'projects');
    await settle(page);
    // Interrupt both the nested exit and the root close before they finish.
    await page.evaluate(() => {
        document.querySelector('#drive-view-folders [data-drive-item-id="drafts"]').click();
        document.getElementById('btn-folder-view-back').click();
        document.getElementById('btn-folder-view-back').click();
        document.querySelector('#drive-folders [data-drive-item-id="travel"]').click();
    });
    await settle(page);
    await expect(page.locator('#drive-folder-view-title')).toHaveText('Viajes');
    await expect(page.locator('#drive-view-files')).toContainText('Viaje');
    await expect(page.locator('#drive-view-files')).not.toContainText('Borrador');
    await page.locator('#home-nav [data-home-tab="home"]').click();
    await expect(page.locator('#drive-folder-view')).toBeHidden();
    await expect(page.locator('.drive-folder-ghost')).toHaveCount(0);
    expect(await page.locator('#drive-folders .drive-folder-art').evaluateAll(cards => cards.every(card => card.style.visibility !== 'hidden'))).toBe(true);
});

test('reduced motion keeps nested navigation immediate and restores focus on close', async ({ page, context }) => {
    await setup(page, context, true);
    await openFolder(page, 'projects');
    await openFolder(page, 'drafts', '#drive-view-folders');
    await expect(page.locator('#drive-folder-view-title')).toHaveText('Borradores');
    expect(await page.evaluate(() => driveFolderAnimation)).toBeNull();
    await page.locator('#btn-folder-view-back').click();
    await page.locator('#btn-folder-view-back').click();
    await expect(page.locator('#drive-folder-view')).toBeHidden();
    await expect(page.locator('#drive-folders [data-drive-item-id="projects"]')).toBeFocused();
});

test('slow Drive responses do not block cached contents or overwrite a newer navigation', async ({ page, context }) => {
    await setup(page, context);
    const pending = [];
    let holdProjects = true;
    await page.route('https://www.googleapis.com/drive/v3/files?*', async route => {
        const query = new URL(route.request().url()).searchParams.get('q') || '';
        if (query.includes("'projects' in parents") && holdProjects) {
            await new Promise(resolve => pending.push(resolve));
            return route.fulfill({ json: { files: [{ id: 'obsolete', name: 'Obsolete.pdf', mimeType: 'application/pdf', parents: ['projects'] }] } });
        }
        const parent = /'([^']+)' in parents/.exec(query)?.[1];
        const isFolder = query.includes("mimeType='application/vnd.google-apps.folder'");
        const files = entries.filter(entry => entry.parents.includes(parent) && (entry.mimeType === folderType) === isFolder);
        if (parent === 'travel' && !isFolder) files.push({ id: 'new-online', name: 'Online.pdf', mimeType: 'application/pdf', parents: ['travel'] });
        return route.fulfill({ json: { files } });
    });
    await page.route('https://www.googleapis.com/drive/v3/about?*', route => route.fulfill({json:{user:{displayName:'Folders',emailAddress:'folders@example.test'}}}));
    await page.route('https://www.googleapis.com/drive/v3/files/root?*', route => route.fulfill({json:{id:'root'}}));
    await context.setOffline(false);
    await page.evaluate(() => { stopDriveHomeAutoRefresh(); driveAccessToken = 'test'; driveTokenExpiry = Date.now() + 3600000; });
    await page.waitForFunction(() => !InhouseOfflineLibrary.offline());
    await openFolder(page, 'projects');
    await expect(page.locator('#drive-view-files')).toContainText('Proyecto');
    await expect.poll(() => pending.length).toBe(2);
    await settle(page);
    await page.locator('#btn-folder-view-back').click();
    await settle(page);
    await page.evaluate(() => {
        window.folderArrivalOpacity = null;
        const observer = new MutationObserver(records => {
            for (const record of records) for (const node of record.addedNodes) {
                if (node.dataset?.driveItemId === 'new-online') {
                    window.folderArrivalOpacity = Number(getComputedStyle(node).opacity);
                    observer.disconnect();
                }
            }
        });
        observer.observe(document.getElementById('drive-view-files'), { childList: true });
    });
    await openFolder(page, 'travel');
    await expect(page.locator('#drive-view-files')).toContainText('Viaje');
    await expect(page.locator('#drive-view-files')).toContainText('Online');
    expect(await page.evaluate(() => window.folderArrivalOpacity)).toBeLessThan(0.1);
    await settle(page);
    holdProjects = false;
    await page.locator('#btn-folder-view-back').click();
    await settle(page);
    await openFolder(page, 'projects');
    await expect(page.locator('#drive-view-files')).toContainText('Proyecto');
    for (const release of pending) release();
    await settle(page);
    await expect(page.locator('#drive-view-files')).not.toContainText('Obsolete');
    await expect(page.locator('#drive-folder-view-title')).toHaveText('Proyectos');
});

async function sampleFolderMotion(page, selector) {
    return page.evaluate(async selector => {
        document.querySelector(selector).click();
        const frames = [];
        await new Promise(resolve => {
            const sample = () => {
                const ghost = document.querySelector('.drive-folder-ghost');
                if (ghost) {
                    const rect = ghost.getBoundingClientRect();
                    const cover = new DOMMatrixReadOnly(getComputedStyle(ghost.querySelector('.drive-folder-cover')).transform);
                    frames.push({ opacity: Number(getComputedStyle(ghost).opacity), turn: cover.m11,
                        direction: ghost.dataset.direction, width: rect.width,
                        originalWidth: parseFloat(ghost.style.width) });
                }
                if (driveFolderAnimation) requestAnimationFrame(sample); else resolve();
            };requestAnimationFrame(sample);
        });
        return frames;
    }, selector);
}

for (const viewport of [{ width: 390, height: 844 }, { width: 1280, height: 900 }]) {
    test(`lid is visible in both directions at every nesting level without full-screen zoom (${viewport.width}px)`, async ({ page, context }) => {
        await page.setViewportSize(viewport);
        await setup(page, context);
        for (const [selector, direction, title] of [
            ['#drive-folders [data-drive-item-id="projects"]', 'open', 'Proyectos'],
            ['#drive-view-folders [data-drive-item-id="drafts"]', 'open', 'Borradores'],
            ['#drive-view-folders [data-drive-item-id="deep"]', 'open', 'Dentro'],
            ['#btn-folder-view-back', 'close', 'Borradores'],
            ['#btn-folder-view-back', 'close', 'Proyectos'],
            ['#btn-folder-view-back', 'close', null]
        ]) {
            const frames = await sampleFolderMotion(page, selector);
            expect(frames.some(frame => frame.direction === direction && frame.opacity > .9 && frame.turn < .94 && frame.turn > .5)).toBe(true);
            expect(frames.every(frame => frame.width <= frame.originalWidth * 1.03)).toBe(true);
            if (title) await expect(page.locator('#drive-folder-view-title')).toHaveText(title);
            else await expect(page.locator('#drive-folder-view')).toBeHidden();
            await expect(page.locator('.drive-folder-ghost')).toHaveCount(0);
            expect(await page.locator('#drive-folder-view').evaluate(el => el.inert)).toBe(false);
        }
    });
}

test('returning from a scrolled nested folder restores its card and position; resize still transitions', async ({ page, context }) => {
    await page.setViewportSize({ width: 390, height: 600 });
    await setup(page, context);
    await page.evaluate(async folderType => {
        await InhouseOfflineLibrary.observed(Array.from({ length: 15 }, (_, i) => ({
            id: `filler-${i}`, name: `Antes ${i}`, mimeType: folderType, parents: ['projects']
        })));
    }, folderType);
    await openFolder(page, 'projects');await settle(page);
    await page.locator('#drive-view-folders [data-drive-item-id="drafts"]').scrollIntoViewIfNeeded();
    const before = await page.locator('#drive-folder-view-scroll').evaluate(el => el.scrollTop);
    expect(before).toBeGreaterThan(0);
    await openFolder(page, 'drafts', '#drive-view-folders');await settle(page);
    const frames = await sampleFolderMotion(page, '#btn-folder-view-back');
    expect(frames.some(frame => frame.direction === 'close' && frame.opacity > .9)).toBe(true);
    await expect.poll(() => page.locator('#drive-folder-view-scroll').evaluate(el => el.scrollTop)).toBe(before);
    await openFolder(page, 'drafts', '#drive-view-folders');await settle(page);
    await page.setViewportSize({ width: 844, height: 390 });
    const state = await page.evaluate(() => {
        document.getElementById('btn-folder-view-back').click();
        return { active: !!driveFolderAnimation, effects: document.getElementById('drive-folder-view').getAnimations({ subtree: true }).length };
    });
    expect(state.active).toBe(true);expect(state.effects).toBeGreaterThan(0);
    await settle(page);
    await expect(page.locator('#drive-folder-view-title')).toHaveText('Proyectos');
    await expect(page.locator('.drive-folder-ghost')).toHaveCount(0);
});
