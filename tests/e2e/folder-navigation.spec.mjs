import { test, expect, chromium } from '@playwright/test';
import { createHash } from 'node:crypto';

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

test('cached folders appear during opening; nested navigation and closing retain content inside the moving folder', async ({ page, context }) => {
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
        const ghost = document.querySelector('.drive-folder-ghost');
        const contents = ghost?.querySelector(':scope > .drive-folder-contents');
        const liveView = document.getElementById('drive-folder-view');
        const visibleContent = contents?.textContent.trim() ? contents : liveView;
        return {
            visible: !!visibleContent && getComputedStyle(visibleContent).display !== 'none',
            text: visibleContent?.textContent,
            effects: ghost?.getAnimations({ subtree: true }).length || 0
        };
    });
    expect(closing.visible).toBe(true);
    expect(closing.text).toContain('Proyecto');
    expect(closing.effects).toBeGreaterThan(0);
    await settle(page);
    await expect(page.locator('#drive-folder-view')).toBeHidden();
    await expect(page.locator('.drive-folder-ghost')).toHaveCount(0);
    await expect(page.locator('#drive-folders [data-drive-item-id="projects"]')).toBeFocused();
});

test('opening starts from the exact card and the interior keeps the shadows it lands with', async ({ page, context }) => {
    await setup(page, context);
    const parts = ['.drive-card-title', '.drive-folder-rule', '.drive-card-more'];
    const card = await page.evaluate(parts => {
        const art = document.querySelector('#drive-folders [data-drive-item-id="projects"] .drive-folder-art');
        return parts.map(selector => {
            const rect = art.querySelector(selector).getBoundingClientRect();
            return [rect.left, rect.top, rect.width, rect.height];
        });
    }, parts);
    // Hold the first frame of a real opening: the lid must still be the card,
    // with its name above the rule and its ⋮ menu in the corner.
    const first = await page.evaluate(async parts => {
        document.querySelector('#drive-folders [data-drive-item-id="projects"]').click();
        await new Promise(resolve => {
            const ready = () => driveFolderAnimation?.phase === 'animating' ? resolve() : requestAnimationFrame(ready);
            requestAnimationFrame(ready);
        });
        const moving = document.getAnimations().filter(animation => animation.effect?.target?.closest?.('.drive-folder-transition'));
        moving.forEach(animation => { animation.pause(); animation.currentTime = 0; });
        const lid = document.querySelector('.drive-folder-ghost > .drive-folder-cover');
        const lidParts = parts.map(selector => {
            const el = lid.querySelector(selector);
            if (!el) return null;
            const rect = el.getBoundingClientRect();
            return [rect.left, rect.top, rect.width, rect.height];
        });
        const nested = getComputedStyle(document.querySelector('.drive-folder-ghost .drive-folder-view .drive-folder-back'));
        const interior = { boxShadow: nested.boxShadow, filter: nested.filter, grain: nested.backgroundImage };
        moving.forEach(animation => animation.play());
        return { lidParts, interior };
    }, parts);
    await settle(page);
    const landed = await page.evaluate(() => {
        const nested = getComputedStyle(document.querySelector('#drive-view-folders .drive-folder-back'));
        return { boxShadow: nested.boxShadow, filter: nested.filter, grain: nested.backgroundImage };
    });
    parts.forEach((selector, index) => {
        expect(first.lidParts[index], selector).not.toBeNull();
        first.lidParts[index].forEach((value, axis) =>
            expect(Math.abs(value - card[index][axis]), `${selector} [${axis}]`).toBeLessThan(1));
    });
    // Folders inside the moving scene look exactly as they do once landed.
    expect(first.interior).toEqual(landed);
    expect(landed.grain).not.toBe('none');
});

test('the folder motion starts with the tap, before the next screen is built', async ({ page, context }) => {
    await setup(page, context);
    for (const selector of ['#drive-folders [data-drive-item-id="projects"]', '#btn-folder-view-back']) {
        // Sampled in the tap's own task: the zoom already plays (pending only
        // until the compositor presents its first frame) and nothing of the
        // next screen has been built yet.
        const state = await page.evaluate(selector => {
            document.querySelector(selector).click();
            const zoom = document.querySelector('.drive-folder-ghost').getAnimations()[0];
            return { playState: zoom.playState, pending: zoom.pending, prepared: driveFolderAnimation.prepared };
        }, selector);
        expect(state, selector).toEqual({ playState: 'running', pending: true, prepared: false });
        await settle(page);
    }
});

test('labels move without their backdrop blur and ease it back in once landed', async ({ page, context }) => {
    await setup(page, context);
    const blur = () => page.evaluate(() => [...document.querySelectorAll('#drive-folder-view .drive-card-info')]
        .map(info => getComputedStyle(info, '::before').backdropFilter));
    await openFolder(page, 'projects');
    await page.waitForFunction(() => driveFolderAnimation?.phase === 'animating');
    const moving = await blur();
    expect(moving.length).toBeGreaterThan(0);
    expect(moving.every(value => value === 'none')).toBe(true);
    await settle(page);
    await expect.poll(blur).toEqual(moving.map(() => 'blur(1.5px)'));
    // The sheets inside a folder are plain white paper.
    expect(await page.evaluate(() => getComputedStyle(document.querySelector('#drive-folders .drive-folder-sheets')).backgroundImage)).toBe('none');
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
    let holdTravel = true;
    let releaseTravel = null;
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
        if (parent === 'travel' && !isFolder) {
            if (holdTravel) await new Promise(resolve => { releaseTravel = resolve; });
            files.push({ id: 'new-online', name: 'Online.pdf', mimeType: 'application/pdf', parents: ['travel'] });
        }
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
    await settle(page);
    // Cached contents are exposed by the lid itself. A genuinely later
    // network result still needs a separate arrival fade in the settled view.
    await expect.poll(() => !!releaseTravel).toBe(true);
    holdTravel = false;
    releaseTravel();
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

async function sampleFolderMotion(page, selector, returnCardSelector = null, childId = null, sampleHandoff = false) {
    return page.evaluate(async ({ selector, returnCardSelector, childId, sampleHandoff }) => {
        // Let the native lid intro and deferred scene preparation finish.
        // The principal zoom is sampled synchronously once it starts.
        const nativeAnimate = Element.prototype.animate;
        const startedInsideFolder = !document.getElementById('drive-folder-view').classList.contains('hidden');
        let zoom = null;
        Element.prototype.animate = function (frames, timing) {
            const animation = nativeAnimate.call(this, frames, timing);
            if (this.classList.contains('drive-folder-ghost')
                && Array.isArray(frames) && frames.some(frame => frame.transform)) {
                zoom = animation;
            }
            return animation;
        };
        try {
            document.querySelector(selector).click();
            await new Promise((resolve, reject) => {
                const deadline = performance.now() + 1200;
                const waitForZoom = () => {
                    if (zoom && driveFolderAnimation?.phase === 'animating') return resolve();
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
        const layer = ghost.closest('.drive-folder-transition');
        const contents = ghost.querySelector(':scope > .drive-folder-contents');
        const lid = ghost.querySelector(':scope > .drive-folder-cover');
        // Seek the actual main effects after the interior has been prepared
        // and rasterised. Finishing below exercises the
        // production transition's content swap and cleanup.
        const animations = document.getAnimations().filter(animation => {
            const target = animation.effect?.target;
            return target === layer || layer.contains(target)
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
        const visibleOpacity = element => {
            if (!element) return 0;
            let opacity = 1;
            for (let node = element; node instanceof Element; node = node.parentElement) {
                const style = getComputedStyle(node);
                if (style.display === 'none' || style.visibility === 'hidden') return 0;
                opacity *= Number(style.opacity);
            }
            return opacity;
        };
        const frames = [];
        for (const progress of [0, .05, .1, .2, .3, .4, .5, .6, .7, .8, .9, .95, .999]) {
            for (let i = 0; i < animations.length; i++) animations[i].currentTime = offsets[i] + duration * progress;
            const rect = ghost.getBoundingClientRect();
            const cover = new DOMMatrixReadOnly(getComputedStyle(lid).transform);
            const children = childId && contents ? [...contents.querySelectorAll(`[data-drive-item-id="${childId}"]`)] : [];
            const interior = contents?.getBoundingClientRect();
            const childVisibility = children.map(child => {
                const bounds = child.getBoundingClientRect();
                const area = Math.max(0, Math.min(bounds.right, target.right, interior.right) - Math.max(bounds.left, target.left, interior.left))
                    * Math.max(0, Math.min(bounds.bottom, target.bottom, interior.bottom) - Math.max(bounds.top, target.top, interior.top));
                return { opacity: area > 0 ? visibleOpacity(child) : 0, area };
            });
            frames.push({ progress, duration, target, destinationHidden, destinationRect, startedInsideFolder, sourceInert,
                opacity: Number(getComputedStyle(ghost).opacity) * Number(getComputedStyle(ghost.parentElement).opacity),
                coverOpacity: Number(getComputedStyle(lid).opacity), turn: cover.m11,
                lidDepth: Math.abs(cover.m13), layerOpacity: Number(getComputedStyle(layer).opacity),
                labelOpacity: visibleOpacity(lid.querySelector('.drive-folder-print')),
                childVisibility,
                direction: ghost.dataset.direction,
                left: rect.left, top: rect.top, right: rect.right, bottom: rect.bottom,
                width: rect.width, height: rect.height,
                centerX: rect.left + rect.width / 2, centerY: rect.top + rect.height / 2,
                originalWidth: parseFloat(ghost.style.width) });
        }
        const captureSettledCards = () => {
            const view = document.getElementById('drive-folder-view');
            const container = view.classList.contains('hidden') ? document.querySelector('#drive-home .drive-main') : view;
            return [...container.querySelectorAll('[data-drive-item-id]')]
                .filter(card => card.offsetParent !== null)
                .map(card => ({
                    id: card.dataset.driveItemId,
                    opacity: visibleOpacity(card),
                    unfinishedArrivals: card.getAnimations({ subtree: true }).filter(animation =>
                        animation.animationName === 'drive-folder-item-in'
                        && animation.effect.getComputedTiming().progress < 1).length
                }));
        };
        driveFolderAnimation.finish();
        if (sampleHandoff) {
            frames.at(-1).handoff = [captureSettledCards()];
            // Inspect both the synchronous DOM handoff and the next painted
            // frame: moving a card out of animation:none can restart its CSS
            // arrival effect only after styles have been recalculated.
            await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
            frames.at(-1).handoff.push(captureSettledCards());
        }
        return frames;
    }, { selector, returnCardSelector, childId, sampleHandoff });
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
    expect(frames[0].duration).toBeGreaterThanOrEqual(500);
    expect(frames[0].duration).toBeLessThanOrEqual(800);
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
    if (direction === 'open') {
        expect(initialTravel, 'The lid opens before the large zoom begins').toBeLessThan(.08);
        expect(frames.find(frame => frame.progress === .1).lidDepth).toBeGreaterThan(.1);
    }
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

function expectFolderContentsDuringZoom(frames) {
    // Turning the lid must expose actual cards while the folder is still
    // growing, not replace the screen with a tint and reveal everything last.
    expect(frames.every(frame => frame.layerOpacity > .99), 'The transition must not fade the whole screen').toBe(true);
    expect(frames.every(frame => frame.coverOpacity > .99), 'The lid must open rather than fade away').toBe(true);
    expect(frames.some(frame => frame.progress >= .2 && frame.progress <= .8
        && frame.lidDepth > .65 && frame.coverOpacity > .95), 'The visible lid must rotate in 3D').toBe(true);
    // The printed name rides on the lid while the folder is still small, and
    // must be gone by the time the lid would show it as enlarged text.
    const enlarged = frames.filter(frame => frame.direction === 'open' ? frame.progress >= .5 : frame.progress <= .5);
    expect(enlarged.every(frame => frame.labelOpacity < .05),
        'The cover label must disappear before it becomes enlarged text').toBe(true);
    expect(frames[0].direction === 'open' ? frames[0].labelOpacity : frames.at(-1).labelOpacity,
        'The cover label is visible while the folder rests on its card').toBeGreaterThan(.9);
    const visible = frames.filter(frame => frame.progress >= .3 && frame.progress <= .8)
        .flatMap(frame => frame.childVisibility.filter(child => child.opacity > .1 && child.area > 200));
    expect(visible.length, 'Real folder children must appear before the zoom completes').toBeGreaterThan(1);
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

    test(`the 3D lid reveals actual children throughout root and nested opening and closing (${viewport.width}px)`, async ({ page, context }) => {
        await page.setViewportSize(viewport);
        await setup(page, context);
        for (const [selector, child] of [
            ['#drive-folders [data-drive-item-id="projects"]', 'drafts'],
            ['#drive-view-folders [data-drive-item-id="drafts"]', 'deep'],
            ['#drive-view-folders [data-drive-item-id="deep"]', 'deep-note']
        ]) {
            const frames = await sampleFolderMotion(page, selector, null, child);
            expectDirectFolderZoom(frames, 'open');
            expectFolderContentsDuringZoom(frames);
            await expect(page.locator('.drive-folder-transition')).toHaveCount(0);
            await expect(page.locator(`#drive-folder-view [data-drive-item-id="${child}"]`)).toBeVisible();
        }
        for (const child of ['deep-note', 'draft-note', 'project-note']) {
            const frames = await sampleFolderMotion(page, '#btn-folder-view-back', null, child);
            expectDirectFolderZoom(frames, 'close');
            expectFolderContentsDuringZoom(frames);
            await expect(page.locator('.drive-folder-transition')).toHaveCount(0);
        }
        await expect(page.locator('#drive-folder-view')).toBeHidden();
        await expect(page.locator('#drive-folder-view')).toHaveCount(1);
    });

    test(`cached cards remain opaque when the folder zoom hands off to the live view (${viewport.width}px)`, async ({ page, context }) => {
        await page.setViewportSize(viewport);
        await setup(page, context);
        await page.evaluate(async files => {
            const pdf = await PDFLib.PDFDocument.create();
            pdf.addPage([210, 297]);
            const blob = new Blob([await pdf.save()], { type: 'application/pdf' });
            for (const file of files) {
                await InhouseOfflineLibrary.cache(file.id, blob, { version: '1' }, null, {
                    fileName: file.name, parentIds: file.parents
                });
            }
        }, entries.filter(entry => entry.mimeType === 'application/pdf'));
        for (const [selector, child, direction] of [
            ['#drive-folders [data-drive-item-id="projects"]', 'project-note', 'open'],
            ['#drive-view-folders [data-drive-item-id="drafts"]', 'draft-note', 'open'],
            ['#drive-view-folders [data-drive-item-id="deep"]', 'deep-note', 'open'],
            ['#btn-folder-view-back', 'draft-note', 'close'],
            ['#btn-folder-view-back', 'project-note', 'close'],
            ['#btn-folder-view-back', 'projects', 'close']
        ]) {
            const frames = await sampleFolderMotion(page, selector, null, child, true);
            if (direction === 'open') {
                expect(frames.at(-1).childVisibility.some(card => card.opacity > .99),
                    `${child} must already be visible before the live view takes over`).toBe(true);
            }
            for (const cards of frames.at(-1).handoff) {
                expect(cards.some(card => card.id === child), `${child} remains in the destination`).toBe(true);
                for (const card of cards) {
                    expect(card.opacity, `${card.id} must not blink at the ${direction} handoff`).toBeGreaterThan(.99);
                    expect(card.unfinishedArrivals, `${card.id} must not restart its arrival fade`).toBe(0);
                }
            }
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

test('finishing a folder zoom preserves focus moved to the topbar search', async ({ page, context }) => {
    await setup(page, context);
    for (const selector of [
        '#drive-folders [data-drive-item-id="projects"]',
        '#drive-view-folders [data-drive-item-id="drafts"]'
    ]) {
        const focus = await page.evaluate(selector => {
            const card = document.querySelector(selector);
            card.focus();
            card.click();
            const search = document.getElementById('drive-search');
            search.focus({ preventScroll: true });
            const during = document.activeElement === search;
            driveFolderAnimation.finish();
            return { during, after: document.activeElement === search };
        }, selector);
        expect(focus).toEqual({ during: true, after: true });
        await expect(page.locator('#drive-search')).toBeFocused();
    }
});

test('dense folders paint intermediate zoom frames on a slower CPU and retain every item', async ({ page, context }) => {
    test.setTimeout(90_000);
    await page.setViewportSize({ width: 390, height: 844 });
    await setup(page, context);
    await page.evaluate(async folderType => {
        const folders = ['projects', 'drafts'].flatMap(parent => Array.from({ length: 149 }, (_, index) => ({
            id: `${parent}-dense-${index}`, name: `Z ${String(index).padStart(3, '0')}`,
            mimeType: folderType, parents: [parent]
        })));
        await InhouseOfflineLibrary.observed(folders);
        await document.fonts.ready;
        await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    }, folderType);
    const cpu = await context.newCDPSession(page);
    await cpu.send('Emulation.setCPUThrottlingRate', { rate: 4 });
    // Actual compositor output, as in the blocked-main-thread test: the zoom
    // is a native compositor animation, so its smoothness is what reaches the
    // screen, not how often the busy main thread gets a frame.
    const presented = [];
    cpu.on('Page.screencastFrame', event => {
        presented.push({ timestamp: event.metadata.timestamp * 1000,
            hash: createHash('sha256').update(event.data).digest('hex') });
        void cpu.send('Page.screencastFrameAck', { sessionId: event.sessionId }).catch(() => {});
    });
    await cpu.send('Page.startScreencast', { format: 'jpeg', quality: 50, everyNthFrame: 1 });
    try {
        for (const [selector, expectedFolder, lastItem] of [
            ['#drive-folders [data-drive-item-id="projects"]', 'Proyectos', 'projects-dense-148'],
            ['#drive-view-folders [data-drive-item-id="drafts"]', 'Borradores', 'drafts-dense-148'],
            ['#btn-folder-view-back', 'Proyectos', 'projects-dense-148'],
            ['#btn-folder-view-back', null, null]
        ]) {
            const motion = await page.evaluate(async selector => {
                const started = performance.now();
                const epoch = performance.timeOrigin + started;
                const nativeAnimate = Element.prototype.animate;
                const frames = [];
                let zoom = null, observedTransition = false, zoomStart = null;
                Element.prototype.animate = function (keyframes, timing) {
                    const animation = nativeAnimate.call(this, keyframes, timing);
                    if (this.classList.contains('drive-folder-ghost')) zoom = animation;
                    return animation;
                };
                try {
                    document.querySelector(selector).click();
                    const clickMs = performance.now() - started;
                    await new Promise(resolve => {
                        const sample = () => {
                            const elapsed = performance.now() - started;
                            observedTransition ||= !!driveFolderAnimation || !!zoom;
                            frames.push({ ms: elapsed, progress: zoom?.effect.getComputedTiming().progress ?? null,
                                built: driveFolderAnimation?.phase === 'animating' });
                            if (zoom?.startTime != null) zoomStart = performance.timeOrigin + zoom.startTime;
                            if ((observedTransition && !driveFolderAnimation) || elapsed > 12_000) resolve();
                            else requestAnimationFrame(sample);
                        };
                        requestAnimationFrame(sample);
                    });
                    return { clickMs, frames, epoch, zoomStart, duration: zoom?.effect.getTiming().duration, finished: !driveFolderAnimation };
                } finally {
                    Element.prototype.animate = nativeAnimate;
                }
            }, selector);
            const moving = motion.frames.filter(frame => frame.progress > .05 && frame.progress < .95);
            const firstAnimated = motion.frames.find(frame => frame.progress !== null);
            // The motion starts with the tap and the next screen is built
            // underneath it while the compositor runs the zoom (main-thread
            // stalls cannot stop it: see the blocked-main-thread test). That
            // build, including the one frame that renders it, must land early
            // in the motion, and after it no long main-thread job may follow
            // until the folder lands.
            const builtAt = motion.frames.find(frame => frame.built)?.progress ?? null;
            // The zoom's own timeline bounds the motion on screen.
            const from = motion.zoomStart + motion.duration * .05, to = motion.zoomStart + motion.duration * .95;
            const shown = presented.filter(frame => frame.timestamp >= from && frame.timestamp <= to);
            const maxGap = Math.max(0, ...shown.slice(1).map((frame, i) => frame.timestamp - shown[i].timestamp));
            const diagnostics = JSON.stringify({ selector, destination: expectedFolder || 'root', clickMs: motion.clickMs,
                firstMotionMs: moving[0]?.ms, firstProgress: firstAnimated?.progress, intermediateFrames: moving.length,
                builtAt, presentedFrames: new Set(shown.map(frame => frame.hash)).size, maxGap });
            expect(motion.finished, diagnostics).toBe(true);
            expect(motion.clickMs, diagnostics).toBeLessThan(100);
            expect(firstAnimated?.progress, diagnostics).toBeLessThan(.2);
            expect(motion.zoomStart, diagnostics).not.toBeNull();
            expect(moving[0]?.ms, diagnostics).toBeLessThan(900);
            expect(builtAt, diagnostics).not.toBeNull();
            // Headless screencasts deliver far fewer images than the display
            // refreshes; this guards against a frozen zoom, not frame rate.
            expect(new Set(shown.map(frame => frame.hash)).size, diagnostics).toBeGreaterThanOrEqual(6);
            expect(maxGap, diagnostics).toBeLessThan(150);
            await expect(page.locator('.drive-folder-transition')).toHaveCount(0);
            if (expectedFolder) {
                await expect(page.locator('#drive-folder-view-title')).toHaveText(expectedFolder);
                await expect(page.locator('#drive-view-folders [data-drive-item-id]')).toHaveCount(150);
                expect(await page.locator('#drive-view-folders .drive-card-deferred').count(),
                    'Offscreen cards must remain deferred until needed').toBeGreaterThan(100);
                // Focusing an offscreen placeholder must hydrate the same card
                // and preserve its keyboard handler, without a lost first action.
                await page.locator(`#drive-view-folders [data-drive-item-id="${lastItem}"]`).focus();
                await expect(page.locator(`#drive-view-folders [data-drive-item-id="${lastItem}"]`)).toBeFocused();
                await page.locator(`#drive-view-folders [data-drive-item-id="${lastItem}"]`).scrollIntoViewIfNeeded();
                await expect(page.locator(`#drive-view-folders [data-drive-item-id="${lastItem}"]`)).toBeInViewport();
                await expect(page.locator(`#drive-view-folders [data-drive-item-id="${lastItem}"] .drive-card-title`)).toHaveText('Z 148');
                await page.locator(`#drive-view-folders [data-drive-item-id="${lastItem}"]`).press('Enter');
                await settle(page);
                await expect(page.locator('#drive-folder-view-title')).toHaveText('Z 148');
                await page.locator('#btn-folder-view-back').click();
                await settle(page);
                await expect(page.locator('#drive-folder-view-title')).toHaveText(expectedFolder);
                await page.locator('#drive-folder-view-scroll').evaluate(el => { el.scrollTop = 0; });
                await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
            } else await expect(page.locator('#drive-folder-view')).toBeHidden();
        }
    } finally {
        await cpu.send('Page.stopScreencast').catch(() => {});
        await cpu.send('Emulation.setCPUThrottlingRate', { rate: 1 });
        await cpu.detach();
    }
});

test('native folder transforms keep presenting frames while the main thread is blocked', async ({ page, context }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await setup(page, context);
    const cdp = await context.newCDPSession(page);
    const presented = [];
    cdp.on('Page.screencastFrame', event => {
        presented.push({ timestamp: event.metadata.timestamp * 1000,
            hash: createHash('sha256').update(event.data).digest('hex') });
        void cdp.send('Page.screencastFrameAck', { sessionId: event.sessionId }).catch(() => {});
    });
    await cdp.send('Page.startScreencast', { format: 'png', everyNthFrame: 1 });
    try {
        for (const [selector, direction] of [
            ['#drive-folders [data-drive-item-id="projects"]', 'open'],
            ['#drive-view-folders [data-drive-item-id="drafts"]', 'open'],
            ['#btn-folder-view-back', 'close'],
            ['#btn-folder-view-back', 'close']
        ]) {
            const result = await page.evaluate(async selector => {
                const nativeAnimate = Element.prototype.animate;
                const descriptor = Object.getOwnPropertyDescriptor(Animation.prototype, 'currentTime');
                let zoom = null, writes = 0;
                Element.prototype.animate = function (frames, timing) {
                    const animation = nativeAnimate.call(this, frames, timing);
                    if (this.classList.contains('drive-folder-ghost')) zoom = animation;
                    return animation;
                };
                Object.defineProperty(Animation.prototype, 'currentTime', {
                    ...descriptor,
                    set(value) {
                        if (this.effect?.target?.closest('.drive-folder-transition')) writes++;
                        descriptor.set.call(this, value);
                    }
                });
                const block = ms => {
                    const deadline = performance.now() + ms;
                    while (performance.now() < deadline) { /* Deliberate main-thread stall. */ }
                };
                try {
                    document.querySelector(selector).click();
                    // Setup must not consume the principal native zoom before
                    // the first opportunity to commit its scene.
                    block(220);
                    const initial = zoom.currentTime;
                    await new Promise((resolve, reject) => {
                        const deadline = performance.now() + 4_000;
                        const ready = () => {
                            if (driveFolderAnimation?.phase === 'animating'
                                && zoom.effect.getComputedTiming().progress >= .22) return resolve();
                            if (performance.now() > deadline) return reject(new Error('Native folder motion did not start'));
                            requestAnimationFrame(ready);
                        };
                        requestAnimationFrame(ready);
                    });
                    const before = { time: zoom.currentTime, writes, state: zoom.playState };
                    const blockStart = performance.timeOrigin + performance.now();
                    block(210);
                    const blockEnd = performance.timeOrigin + performance.now();
                    await new Promise(requestAnimationFrame);
                    const after = { time: zoom.currentTime, writes, state: zoom.playState };
                    await new Promise(resolve => {
                        const finished = () => !driveFolderAnimation ? resolve() : requestAnimationFrame(finished);
                        finished();
                    });
                    return { initial, before, after, blockStart, blockEnd,
                        duration: zoom.effect.getComputedTiming().endTime, finalWrites: writes };
                } finally {
                    Element.prototype.animate = nativeAnimate;
                    Object.defineProperty(Animation.prototype, 'currentTime', descriptor);
                }
            }, selector);
            const duringBlock = presented.filter(frame => frame.timestamp > result.blockStart && frame.timestamp < result.blockEnd);
            const diagnostics = JSON.stringify({ direction, ...result, presentedDuringBlock: duringBlock.length });
            expect(result.initial, diagnostics).toBe(0);
            expect(result.duration).toBeGreaterThanOrEqual(direction === 'open' ? 540 : 530);
            expect(result.duration).toBeLessThanOrEqual(direction === 'open' ? 620 : 600);
            expect(result.before.state, diagnostics).toBe('running');
            expect(result.after.time - result.before.time, diagnostics).toBeGreaterThanOrEqual(160);
            expect(result.finalWrites - result.before.writes, diagnostics).toBe(0);
            // A timeline advancing alone does not prove smooth output. These
            // are actual compositor images delivered while JS could not run.
            expect(new Set(duringBlock.map(frame => frame.hash)).size, diagnostics).toBeGreaterThanOrEqual(4);
            await expect(page.locator('.drive-folder-transition')).toHaveCount(0);
        }
        await expect(page.locator('#drive-folder-view')).toBeHidden();
    } finally {
        await cdp.send('Page.stopScreencast');
        await cdp.detach();
    }
});

test('interrupting folder warmup prepares once and cannot cancel the next navigation', async ({ page, context }) => {
    await setup(page, context);
    for (const interruption of ['finish', 'resize', 'hide']) {
        const result = await page.evaluate(async interruption => {
            const originalPrepare = prepareDriveFolderViewContent;
            const hiddenDescriptor = Object.getOwnPropertyDescriptor(document, 'hidden');
            const prepared = [];
            prepareDriveFolderViewContent = (...args) => {
                prepared.push(args[0]?.id);
                return originalPrepare(...args);
            };
            const restoreVisibility = () => {
                if (hiddenDescriptor) Object.defineProperty(document, 'hidden', hiddenDescriptor);
                else delete document.hidden;
            };
            try {
                document.querySelector('#drive-folders [data-drive-item-id="projects"]').click();
                const pending = driveFolderAnimation;
                const wasPending = !!pending && !pending.prepared;
                if (interruption === 'finish') pending.finish();
                else if (interruption === 'resize') window.dispatchEvent(new Event('resize'));
                else {
                    Object.defineProperty(document, 'hidden', { configurable: true, get: () => true });
                    document.dispatchEvent(new Event('visibilitychange'));
                    restoreVisibility();
                }
                const afterInterrupt = {
                    prepared: [...prepared], active: !!driveFolderAnimation,
                    ghosts: document.querySelectorAll('.drive-folder-transition').length,
                    title: document.getElementById('drive-folder-view-title').textContent,
                    inert: document.getElementById('drive-folder-view').inert
                };
                // Start another transition before the abandoned warmup's RAFs
                // could run. They must neither prepare twice nor finish this one.
                document.querySelector('#drive-view-folders [data-drive-item-id="drafts"]').click();
                const successor = driveFolderAnimation;
                await new Promise((resolve, reject) => {
                    const deadline = performance.now() + 1_200;
                    const ready = () => {
                        if (successor.prepared) return resolve();
                        if (performance.now() > deadline) return reject(new Error('Successor did not prepare'));
                        requestAnimationFrame(ready);
                    };
                    requestAnimationFrame(ready);
                });
                const next = {
                    sameHandle: !!successor && driveFolderAnimation === successor,
                    prepared: [...prepared],
                    ghosts: document.querySelectorAll('.drive-folder-transition').length,
                    title: document.getElementById('drive-folder-view-title').textContent
                };
                successor.finish();
                closeDriveFolderView({ animate: false });
                await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
                return { wasPending, afterInterrupt, next, finalPrepared: prepared,
                    finalGhosts: document.querySelectorAll('.drive-folder-transition').length,
                    finalActive: !!driveFolderAnimation };
            } finally {
                prepareDriveFolderViewContent = originalPrepare;
                restoreVisibility();
            }
        }, interruption);
        expect(result.wasPending, interruption).toBe(true);
        expect(result.afterInterrupt, interruption).toEqual({
            prepared: ['projects'], active: false, ghosts: 0, title: 'Proyectos', inert: false
        });
        expect(result.next, interruption).toEqual({
            sameHandle: true, prepared: ['projects', 'drafts'], ghosts: 1, title: 'Borradores'
        });
        expect(result.finalPrepared, interruption).toEqual(['projects', 'drafts']);
        expect(result.finalGhosts, interruption).toBe(0);
        expect(result.finalActive, interruption).toBe(false);
    }
});


// Reproduce actual scene DOM/styles/keyframes on a controlled display source.
// The full-app screencast test above covers live navigation and JS stalls;
// this test isolates interpolation at 120Hz from browser boot and real vsync.
test('real folder scenes interpolate at 120 Hz on accelerated compositor layers', async ({ page, context }) => {
    test.setTimeout(60_000);
    await page.setViewportSize({ width: 390, height: 844 });
    await setup(page, context);
    const scenes = [];
    for (const selector of [
        '#drive-folders [data-drive-item-id="projects"]',
        '#drive-view-folders [data-drive-item-id="drafts"]',
        '#btn-folder-view-back', '#btn-folder-view-back'
    ]) {
        scenes.push(await page.evaluate(async selector => {
            document.querySelector(selector).click();
            await new Promise(resolve => {
                const ready = () => driveFolderAnimation?.phase === 'animating'
                    ? resolve() : requestAnimationFrame(ready);
                requestAnimationFrame(ready);
            });
            const layer = document.querySelector('.drive-folder-transition');
            const targets = ['.drive-folder-ghost', '.drive-folder-ghost > .drive-folder-cover'];
            const effects = targets.map(selector => {
                const animation = layer.querySelector(selector).getAnimations()[0];
                return { selector, frames: animation.effect.getKeyframes(), timing: animation.effect.getTiming() };
            });
            const wrapper = document.getElementById('drive-home').cloneNode(false);
            wrapper.appendChild(layer.cloneNode(true));
            const body = document.body.cloneNode(false);
            body.appendChild(wrapper);
            const css = [...document.styleSheets].flatMap(sheet => {
                try { return [...sheet.cssRules].filter(rule => rule.type !== CSSRule.FONT_FACE_RULE).map(rule => rule.cssText); }
                catch { return []; }
            }).join('\n');
            const scene = { html: body.outerHTML, css, effects,
                htmlClass: document.documentElement.className, direction: layer.querySelector('.drive-folder-ghost').dataset.direction };
            driveFolderAnimation.finish();
            return scene;
        }, selector));
    }
    const browser = await chromium.launch({ args: [
        '--enable-begin-frame-control', '--run-all-compositor-stages-before-draw'
    ] });
    try {
        for (const scene of scenes) {
            const replay = await browser.newPage({ viewport: { width: 390, height: 844 } });
            // The runner's trace screencast drives extra 60Hz frames. Keep a
            // trace without its screenshot stream for this controlled display.
            await replay.context().tracing.stop();
            await replay.context().tracing.start({ screenshots: false, snapshots: false });
            const cdp = await replay.context().newCDPSession(replay);
            let layers = [];
            await cdp.send('Performance.enable');
            await cdp.send('DOM.enable');
            await cdp.send('LayerTree.enable');
            cdp.on('LayerTree.layerTreeDidChange', event => { layers = event.layers || []; });
            await replay.setContent(`<html class="${scene.htmlClass}"><head><style>${scene.css}</style></head>${scene.html}</html>`);
            await replay.evaluate(effects => {
                window.replayedFolderEffects = effects.map(({ selector, frames, timing }) =>
                    document.querySelector(selector).animate(frames, timing));
                // Keep the document timeline observable at each injected
                // display tick too. This never seeks a native animation.
                const observeDisplayTick = () => requestAnimationFrame(observeDisplayTick);
                requestAnimationFrame(observeDisplayTick);
            }, scene.effects);
            const { metrics } = await cdp.send('Performance.getMetrics');
            const start = metrics.find(metric => metric.name === 'Timestamp').value * 1000;
            const { root } = await cdp.send('DOM.getDocument');
            const samples = [], images = [], accelerated = new Set();
            for (let index = 0; index < 40; index++) {
                let frameTimeout;
                const frame = await Promise.race([
                    cdp.send('HeadlessExperimental.beginFrame', {
                        frameTimeTicks: start + index * 1000 / 120, interval: 1000 / 120,
                        screenshot: { format: 'png' }
                    }),
                    new Promise((_, reject) => {
                        frameTimeout = setTimeout(() => reject(new Error(`Controlled display frame ${index} did not present`)), 2_000);
                    })
                ]).finally(() => clearTimeout(frameTimeout));
                images.push(createHash('sha256').update(frame.screenshotData || '').digest('hex'));
                samples.push(await replay.evaluate(() => replayedFolderEffects.map(animation => ({
                    time: animation.currentTime, state: animation.playState
                }))));
                if (index === 8) {
                    for (const { selector } of scene.effects) {
                        const { nodeId } = await cdp.send('DOM.querySelector', { nodeId: root.nodeId, selector });
                        const { node } = await cdp.send('DOM.describeNode', { nodeId });
                        for (const layer of layers.filter(layer => layer.backendNodeId === node.backendNodeId)) {
                            const reasons = await cdp.send('LayerTree.compositingReasons', { layerId: layer.layerId });
                            if (reasons.compositingReasonIds.includes('ActiveTransformAnimation')) accelerated.add(selector);
                        }
                    }
                }
            }
            const diagnostics = JSON.stringify({ direction: scene.direction, accelerated: [...accelerated],
                uniqueImages: new Set(images).size, times: samples.map(sample => sample[0].time) });
            expect(accelerated.size, diagnostics).toBe(2);
            expect(new Set(images).size, diagnostics).toBeGreaterThanOrEqual(35);
            for (let index = 1; index < samples.length; index++) {
                for (let effect = 0; effect < 2; effect++) {
                    expect(samples[index][effect].state, diagnostics).toBe('running');
                    const step = samples[index][effect].time - samples[index - 1][effect].time;
                    // A 60Hz JS driver would duplicate poses or jump 16.67ms.
                    expect(step, diagnostics).toBeGreaterThan(8.1);
                    expect(step, diagnostics).toBeLessThan(8.6);
                }
            }
            await cdp.detach();
            await replay.close();
        }
    } finally {
        await browser.close();
    }
});
