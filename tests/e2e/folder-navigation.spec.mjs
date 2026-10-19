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
        // Reconnecting also refreshes the whole cached catalogue. Returning
        // an empty catalogue here would mark the fixture's folders trashed.
        if (!parent && query.includes(' or ')) {
            return route.fulfill({ json: { files: entries } });
        }
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

async function sampleFolderMotion(page, selector, returnCardSelector = null) {
    return page.evaluate(async ({ selector, returnCardSelector }) => {
        // Pause the actual zoom at creation. On close it starts after the short
        // cover crossfade; observing its creation avoids racing that phase.
        const nativeAnimate = Element.prototype.animate;
        const startedInsideFolder = !document.getElementById('drive-folder-view').classList.contains('hidden');
        let zoom = null;
        Element.prototype.animate = function (frames, timing) {
            const animation = nativeAnimate.call(this, frames, timing);
            if (this.classList.contains('drive-folder-ghost')
                && Array.isArray(frames) && frames.some(frame => frame.transform)) {
                zoom = animation;
                zoom.pause();
            }
            return animation;
        };
        try {
            document.querySelector(selector).click();
            await new Promise((resolve, reject) => {
                const deadline = performance.now() + 1200;
                const waitForZoom = () => {
                    if (zoom) return resolve();
                    if (!driveFolderAnimation || performance.now() > deadline) return reject(new Error('No visible folder zoom started'));
                    requestAnimationFrame(waitForZoom);
                };
                waitForZoom();
            });
        } finally {
            Element.prototype.animate = nativeAnimate;
        }
        const ghost = document.querySelector('.drive-folder-ghost');
        if (!ghost || !driveFolderAnimation) return [];
        // Seek the actual compositor effects instead of racing a 300ms
        // animation on a loaded CI worker. Finishing below still exercises the
        // production transition's content swap and cleanup.
        const animations = document.getAnimations().filter(animation => {
            const target = animation.effect?.target;
            return target === ghost || ghost.contains(target)
                || document.getElementById('drive-folder-view').contains(target);
        });
        const offsets = animations.map(animation => (animation.currentTime || 0) - (zoom.currentTime || 0));
        animations.forEach(animation => animation.pause());
        const duration = zoom.effect.getComputedTiming().endTime;
        const home = document.getElementById('drive-home').getBoundingClientRect();
        const top = document.querySelector('#drive-home .drive-topbar').getBoundingClientRect().bottom;
        const target = { left: home.left, top, right: home.right, bottom: home.bottom,
            width: home.width, height: home.bottom - top };
        const destination = returnCardSelector ? document.querySelector(`${returnCardSelector} .drive-folder-art`) : null;
        const destinationHidden = returnCardSelector ? !!destination && getComputedStyle(destination).visibility === 'hidden' : undefined;
        const destinationRect = destination?.getBoundingClientRect().toJSON();
        const sourceInert = document.getElementById('drive-folder-view').inert;
        const frames = [];
        for (const progress of [0, .05, .1, .2, .3, .4, .5, .6, .7, .8, .9, .95, .999]) {
            for (let i = 0; i < animations.length; i++) animations[i].currentTime = offsets[i] + duration * progress;
            const rect = ghost.getBoundingClientRect();
            const cover = new DOMMatrixReadOnly(getComputedStyle(ghost.querySelector('.drive-folder-cover')).transform);
            frames.push({ progress, duration, target, destinationHidden, destinationRect, startedInsideFolder, sourceInert,
                opacity: Number(getComputedStyle(ghost).opacity) * Number(getComputedStyle(ghost.parentElement).opacity),
                coverOpacity: Number(getComputedStyle(ghost.querySelector('.drive-folder-cover')).opacity), turn: cover.m11,
                direction: ghost.dataset.direction,
                left: rect.left, top: rect.top, right: rect.right, bottom: rect.bottom,
                width: rect.width, height: rect.height,
                centerX: rect.left + rect.width / 2, centerY: rect.top + rect.height / 2,
                originalWidth: parseFloat(ghost.style.width) });
        }
        driveFolderAnimation.finish();
        return frames;
    }, { selector, returnCardSelector });
}

function expectDirectFolderZoom(frames, direction) {
    expect(frames.length).toBeGreaterThan(5);
    expect(frames.every(frame => frame.direction === direction)).toBe(true);
    if (direction === 'open' && frames[0].startedInsideFolder) expect(frames[0].sourceInert).toBe(true);
    if (direction === 'close' && frames[0].destinationHidden !== undefined) {
        expect(frames[0].destinationHidden).toBe(true);
        const final = frames.at(-1);
        for (const field of ['left', 'top', 'width', 'height']) {
            expect(Math.abs(final[field] - final.destinationRect[field]), `${field}: closing must land on the real card`).toBeLessThan(1.5);
        }
    }
    expect(frames[0].duration).toBeGreaterThanOrEqual(200);
    expect(frames[0].duration).toBeLessThanOrEqual(420);
    const ordered = direction === 'open' ? frames : [...frames].reverse();
    const start = ordered[0], end = ordered.at(-1);
    expect(end.width).toBeGreaterThan(start.width * 1.3);
    const widthTravel = Math.abs(frames.at(-1).width - frames[0].width);
    const initialTravel = Math.abs(frames.find(frame => frame.progress === .1).width - frames[0].width) / widthTravel;
    const finalTravel = Math.abs(frames.at(-1).width - frames.find(frame => frame.progress === .9).width) / widthTravel;
    // Restrained ease-in/out: neither end can spend a large fraction of the
    // journey in just a tenth of the animation.
    expect(initialTravel).toBeLessThan(.2);
    expect(finalTravel).toBeLessThan(.2);
    expect(end.left).toBeLessThanOrEqual(end.target.left + 2);
    expect(end.top).toBeLessThanOrEqual(end.target.top + 2);
    expect(end.right).toBeGreaterThanOrEqual(end.target.right - 2);
    expect(end.bottom).toBeGreaterThanOrEqual(end.target.bottom - 2);
    // A single eased zoom may travel from the card to the viewport centre,
    // but cannot sway sideways, overshoot, or shrink again on the way there.
    for (const field of ['width', 'height', 'centerX', 'centerY']) {
        const sign = end[field] >= start[field] ? 1 : -1;
        for (let i = 1; i < ordered.length; i++) {
            expect((ordered[i][field] - ordered[i - 1][field]) * sign,
                `${direction}: ${field} reverses at ${ordered[i].progress}`).toBeGreaterThanOrEqual(-.75);
        }
    }
    expect(frames.some(frame => frame.progress >= .2 && frame.progress <= .8
        && frame.opacity > .5 && frame.width > start.width * 1.2)).toBe(true);
    // The folder lid itself still opens/closes; a content-only fade must not
    // accidentally replace the visual again.
    expect(frames.some(frame => frame.opacity > .5 && frame.coverOpacity > .4 && frame.turn < .95)).toBe(true);
}

for (const viewport of [{ width: 390, height: 844 }, { width: 1280, height: 900 }]) {
    test(`folder zoom fills the content viewport directly in both directions at every nesting level (${viewport.width}px)`, async ({ page, context }) => {
        await page.setViewportSize(viewport);
        await setup(page, context);
        for (const [selector, direction, title, returnCard] of [
            ['#drive-folders [data-drive-item-id="projects"]', 'open', 'Proyectos'],
            ['#drive-view-folders [data-drive-item-id="drafts"]', 'open', 'Borradores'],
            ['#drive-view-folders [data-drive-item-id="deep"]', 'open', 'Dentro'],
            ['#btn-folder-view-back', 'close', 'Borradores', '#drive-view-folders [data-drive-item-id="deep"]'],
            ['#btn-folder-view-back', 'close', 'Proyectos', '#drive-view-folders [data-drive-item-id="drafts"]'],
            ['#btn-folder-view-back', 'close', null, '#drive-folders [data-drive-item-id="projects"]']
        ]) {
            const frames = await sampleFolderMotion(page, selector, returnCard);
            expectDirectFolderZoom(frames, direction);
            if (title) await expect(page.locator('#drive-folder-view-title')).toHaveText(title);
            else await expect(page.locator('#drive-folder-view')).toBeHidden();
            await expect(page.locator('.drive-folder-ghost')).toHaveCount(0);
            expect(await page.locator('#drive-folder-view').evaluate(el => el.inert)).toBe(false);
        }
    });

    test(`breadcrumb jumps zoom back to the ancestor card without losing navigation (${viewport.width}px)`, async ({ page, context }) => {
        await page.setViewportSize(viewport);
        await setup(page, context);
        for (const [id, container] of [['projects', '#drive-folders'], ['drafts', '#drive-view-folders'], ['deep', '#drive-view-folders']]) {
            await openFolder(page, id, container);
            await settle(page);
        }
        expectDirectFolderZoom(await sampleFolderMotion(page, '#drive-breadcrumbs button:nth-of-type(2)', '#drive-view-folders [data-drive-item-id="drafts"]'), 'close');
        await expect(page.locator('#drive-folder-view-title')).toHaveText('Proyectos');
        await expect(page.locator('#drive-view-folders [data-drive-item-id="drafts"]')).toBeVisible();
        expectDirectFolderZoom(await sampleFolderMotion(page, '#drive-breadcrumbs button:first-of-type', '#drive-folders [data-drive-item-id="projects"]'), 'close');
        await expect(page.locator('#drive-folder-view')).toBeHidden();
        await expect(page.locator('.drive-folder-ghost')).toHaveCount(0);
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
    // The browser can adjust scroll to expose the click target; restore the
    // position captured at activation, after that adjustment.
    const activatedAt = await page.evaluate(() => driveFolderStack.at(-1).origin.scrollTop);
    expect(activatedAt).toBeGreaterThan(0);
    const frames = await sampleFolderMotion(page, '#btn-folder-view-back', '#drive-view-folders [data-drive-item-id="drafts"]');
    expectDirectFolderZoom(frames, 'close');
    await expect.poll(() => page.locator('#drive-folder-view-scroll').evaluate(el => el.scrollTop)).toBe(activatedAt);
    await openFolder(page, 'drafts', '#drive-view-folders');await settle(page);
    await page.setViewportSize({ width: 844, height: 390 });
    expectDirectFolderZoom(await sampleFolderMotion(page, '#btn-folder-view-back', '#drive-view-folders [data-drive-item-id="drafts"]'), 'close');
    await settle(page);
    await expect(page.locator('#drive-folder-view-title')).toHaveText('Proyectos');
    await expect(page.locator('.drive-folder-ghost')).toHaveCount(0);
});
