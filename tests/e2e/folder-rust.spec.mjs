import { test, expect } from '@playwright/test';

const folderType = 'application/vnd.google-apps.folder';

async function setup(page, context, { folders = 0, files = 0, rust = true } = {}) {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.route('https://accounts.google.com/**', route => route.fulfill({ status: 200, body: '' }));
    if (!rust) await page.route('**/wasm/inhouse_notes_core.wasm*', route => route.abort());
    await page.addInitScript(() => {
        localStorage.setItem('drive-remember-v1', '1');
        localStorage.setItem('drive-hint-email', 'folder-rust@example.test');
    });
    await page.goto('/?e2e=1');
    const available = await page.evaluate(async ({ folders, files, folderType }) => {
        await __IHN_TEST_API__.ready();
        await InhouseRust.ready;
        stopDriveHomeAutoRefresh();
        const entries = [{ id: 'collection', name: 'Collection', mimeType: folderType, parents: ['root'] }];
        for (let i = 0; i < folders; i++) entries.push({
            id: `folder-${i}`, name: `Folder ${String(i).padStart(4, '0')}`,
            mimeType: folderType, parents: ['collection']
        });
        const previews = [[210, 297], [420, 210], [168, 420]].map(([width, height], index) => {
            const canvas = document.createElement('canvas');
            canvas.width = width; canvas.height = height;
            const ctx = canvas.getContext('2d');
            ctx.fillStyle = ['#f8f3e8', '#e9f2fc', '#faf0f4'][index];
            ctx.fillRect(0, 0, width, height);
            ctx.fillStyle = '#222'; ctx.font = '18px sans-serif';
            ctx.fillText(`Aspect ${width}/${height}`, 10, 30);
            return canvas.toDataURL('image/png');
        });
        for (let i = 0; i < files; i++) entries.push({
            id: `pdf-${i}`, name: `Document ${String(i).padStart(4, '0')}.pdf`,
            mimeType: 'application/pdf', parents: ['collection'], localPreview: previews[i % previews.length],
            modifiedTime: new Date(Date.UTC(2026, 0, 1) - i * 1000).toISOString()
        });
        await InhouseOfflineLibrary.observed(entries);
        setDriveHomeTab('files');
        InhouseOfflineApp.renderFolder();
        const original = InhouseRust;
        window.folderRustCalls = [];
        // Observe the public numeric boundary without adding product test flags.
        window.InhouseRust = { ...original };
        for (const method of ['folderMotion', 'folderRows', 'folderFade']) {
            window.InhouseRust[method] = (...args) => {
                const result = original[method](...args);
                folderRustCalls.push({ method });
                if (method === 'folderRows' && result) return {
                    ...result,
                    range(...args) {
                        const range = result.range(...args);
                        folderRustCalls.push({ method: 'folderRange' });
                        return range;
                    }
                };
                return result;
            };
        }
        return original.available();
    }, { folders, files, folderType });
    expect(available).toBe(rust);
    await context.setOffline(true);
}

async function settle(page) {
    await page.waitForFunction(() => !driveFolderAnimation);
    // The completed scene reconciles and hydrates after its final paint.
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => setTimeout(resolve, 0))));
}

async function openCollection(page) {
    await page.locator('#drive-folders [data-drive-item-id="collection"]').click();
    await settle(page);
}

async function measureLayoutReads(page, action) {
    return page.evaluate(action => {
        let reads = 0;
        const isCard = element => element.hasAttribute('data-drive-item-id')
            && !!element.closest('.drive-folder-view');
        const rect = Element.prototype.getBoundingClientRect;
        const descriptors = new Map();
        Element.prototype.getBoundingClientRect = function () {
            if (isCard(this)) reads++;
            return rect.call(this);
        };
        for (const name of ['offsetTop', 'offsetHeight', 'offsetWidth']) {
            const descriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, name);
            if (!descriptor?.get) continue;
            descriptors.set(name, descriptor);
            Object.defineProperty(HTMLElement.prototype, name, {
                ...descriptor,
                get() {
                    if (isCard(this)) reads++;
                    return descriptor.get.call(this);
                }
            });
        }
        try {
            if (action === 'hydrate') hydrateVisibleDriveFolderCards();
            else {
                const snapshot = snapshotDriveFolderView();
                window.folderSnapshotItems = snapshot.view.querySelectorAll('[data-drive-item-id]').length;
            }
            return reads;
        } finally {
            Element.prototype.getBoundingClientRect = rect;
            for (const [name, descriptor] of descriptors) Object.defineProperty(HTMLElement.prototype, name, descriptor);
        }
    }, action);
}

async function visibleCards(page, container) {
    return page.evaluate(container => {
        const viewport = document.getElementById('drive-folder-view-scroll').getBoundingClientRect();
        return [...document.querySelectorAll(`${container} [data-drive-item-id]`)].flatMap(card => {
            const rect = card.getBoundingClientRect();
            return rect.bottom > viewport.top + 1 && rect.top < viewport.bottom - 1
                ? [{ id: card.dataset.driveItemId, deferred: card.classList.contains('drive-card-deferred'),
                    top: rect.top, height: rect.height, width: rect.width }]
                : [];
        });
    }, container);
}

async function expectSnapshotGeometry(page) {
    const geometry = await page.evaluate(() => {
        const viewport = driveFolderViewScrollEl.getBoundingClientRect();
        const visible = [...driveFolderViewEl.querySelectorAll('[data-drive-item-id]')]
            .map(card => ({ id: card.dataset.driveItemId, rect: card.getBoundingClientRect() }))
            .filter(({ rect }) => rect.bottom > viewport.top + 1 && rect.top < viewport.bottom - 1);
        const snapshot = snapshotDriveFolderView();
        snapshot.view.style.visibility = 'hidden';
        driveFolderViewEl.parentElement.append(snapshot.view);
        try {
            const scroll = snapshot.view.querySelector('.drive-folder-view-scroll');
            scroll.scrollTop = snapshot.scrollTop;
            return {
                scrollHeightError: Math.abs(scroll.scrollHeight - driveFolderViewScrollEl.scrollHeight),
                cards: visible.map(({ id, rect }) => {
                    const clone = snapshot.view.querySelector(`[data-drive-item-id="${id}"]`);
                    if (!clone) return { id, missing: true };
                    const actual = clone.getBoundingClientRect();
                    return { id, errors: ['top', 'left', 'width', 'height'].map(axis => Math.abs(actual[axis] - rect[axis])) };
                })
            };
        } finally {
            snapshot.view.remove();
        }
    });
    expect(geometry.scrollHeightError).toBeLessThanOrEqual(1);
    expect(geometry.cards.length).toBeGreaterThan(0);
    for (const card of geometry.cards) {
        expect(card.missing, card.id).toBeUndefined();
        expect(Math.max(...card.errors), card.id).toBeLessThan(1);
    }
}

test('folder plans drive real folder motion and bound layout reads independently of a thousand cards', async ({ page, context }) => {
    test.setTimeout(45_000);
    await setup(page, context, { folders: 1000 });
    await openCollection(page);
    await expect(page.locator('#drive-view-folders > [data-drive-item-id]')).toHaveCount(1000);
    expect(await page.locator('#drive-view-folders .drive-card-deferred').count()).toBeGreaterThan(900);
    const calls = await page.evaluate(() => folderRustCalls);
    for (const method of ['folderMotion', 'folderRows', 'folderRange', 'folderFade']) {
        expect(calls.some(call => call.method === method), method).toBe(true);
    }
    expect(await measureLayoutReads(page, 'hydrate')).toBeLessThan(40);
    expect(await measureLayoutReads(page, 'snapshot')).toBeLessThan(40);
    expect(await page.evaluate(() => folderSnapshotItems)).toBeLessThan(40);
    const visible = await visibleCards(page, '#drive-view-folders');
    expect(visible.length).toBeGreaterThan(3);
    expect(visible.every(card => !card.deferred)).toBe(true);
});

test('deep scrolling and nested back navigation keep the last item usable with bounded measurements', async ({ page, context }) => {
    test.setTimeout(45_000);
    await setup(page, context, { folders: 1000 });
    await openCollection(page);
    await page.locator('#drive-folder-view-scroll').evaluate(element => { element.scrollTop = element.scrollHeight; });
    const last = page.locator('#drive-view-folders [data-drive-item-id="folder-999"]');
    await expect(last.locator('.drive-card-title')).toHaveText('Folder 0999');
    expect(await measureLayoutReads(page, 'hydrate')).toBeLessThan(40);
    expect(await measureLayoutReads(page, 'snapshot')).toBeLessThan(40);
    expect(await page.evaluate(() => folderSnapshotItems)).toBeLessThan(40);
    const visible = await visibleCards(page, '#drive-view-folders');
    expect(visible.some(card => card.id === 'folder-999')).toBe(true);
    expect(visible.every(card => !card.deferred)).toBe(true);
    await expectSnapshotGeometry(page);
    await last.focus();
    await page.keyboard.press('Enter');
    await settle(page);
    await expect(page.locator('#drive-folder-view-title')).toHaveText('Folder 0999');
    const savedScroll = await page.evaluate(() => driveFolderStack.at(-1).origin.scrollTop);
    expect(savedScroll).toBeGreaterThan(10_000);
    await page.locator('#btn-folder-view-back').click();
    await settle(page);
    await expect(page.locator('#drive-folder-view-title')).toHaveText('Collection');
    await expect.poll(() => page.locator('#drive-folder-view-scroll').evaluate(element => element.scrollTop)).toBe(savedScroll);
    await expect(last.locator('.drive-card-title')).toHaveText('Folder 0999');
    await expect(last).toBeInViewport();
});

test('mixed PDF aspect ratios keep visible slots hydrated through deep scrolling and resize', async ({ page, context }) => {
    // The final row is incomplete on mobile and desktop; New PDF shares it.
    await setup(page, context, { files: 119 });
    await openCollection(page);
    await expect.poll(() => page.locator('#drive-view-files .drive-card-preview.is-loaded').count()).toBeGreaterThanOrEqual(3);
    const first = await visibleCards(page, '#drive-view-files');
    expect(first.length).toBeGreaterThanOrEqual(3);
    expect(first.every(card => !card.deferred)).toBe(true);
    for (const [id, ratio] of [['pdf-0', 210 / 297], ['pdf-1', 2], ['pdf-2', .4]]) {
        await expect.poll(() => page.locator(`#drive-view-files [data-drive-item-id="${id}"] .drive-card-preview`)
            .evaluate(element => {
                const rect = element.getBoundingClientRect();
                return Math.abs(rect.width / rect.height - Number(getComputedStyle(element.closest('.drive-card')).getPropertyValue('--sheet-ratio')));
            })).toBeLessThan(.01);
        expect(await page.locator(`#drive-view-files [data-drive-item-id="${id}"]`)
            .evaluate(element => Number(getComputedStyle(element).getPropertyValue('--sheet-ratio')))).toBeCloseTo(ratio, 2);
    }
    await expectSnapshotGeometry(page);
    await page.locator('#drive-folder-view-scroll').evaluate(element => { element.scrollTop = element.scrollHeight; });
    await expect(page.locator('#drive-view-files [data-drive-item-id="pdf-118"] .drive-card-title')).toHaveText('Document 0118');
    for (const viewport of [{ width: 1280, height: 800 }, { width: 390, height: 844 }]) {
        await page.setViewportSize(viewport);
        await page.locator('#drive-folder-view-scroll').evaluate(element => { element.scrollTop = element.scrollHeight; });
        await expect.poll(async () => {
            const visible = await visibleCards(page, '#drive-view-files');
            return visible.length > 0 && visible.every(card => !card.deferred);
        }).toBe(true);
        expect(await measureLayoutReads(page, 'hydrate')).toBeLessThan(40);
        const visible = await visibleCards(page, '#drive-view-files');
        expect(visible.some(card => card.id === 'pdf-118')).toBe(true);
        await expectSnapshotGeometry(page);
    }
});

test('decoded thumbnails rebuild row offsets even when the grid keeps its total height', async ({ page, context }) => {
    await setup(page, context, { files: 6 });
    await page.setViewportSize({ width: 320, height: 844 });
    await openCollection(page);
    await expect(page.locator('#drive-view-files .drive-card-preview.is-loaded')).toHaveCount(6);
    const refreshRows = reverse => page.evaluate(async reverse => {
        const sources = [[210, 297], [420, 210]].map(([width, height]) => {
            const canvas = document.createElement('canvas');
            canvas.width = width; canvas.height = height;
            canvas.getContext('2d').fillRect(0, 0, width, height);
            return canvas.toDataURL('image/png');
        });
        await Promise.all([...driveViewFilesEl.children].map((card, index) => {
            const row = Math.floor(index / 3);
            const file = { ...card.driveFile, localPreview: sources[reverse ? 1 - row : row],
                modifiedTime: reverse ? '2026-02-02T00:00:00Z' : '2026-02-01T00:00:00Z' };
            card.driveFile = file;
            return card.querySelector('.drive-card-preview').refreshFile(file);
        }));
    }, reverse);
    await refreshRows(false);
    const before = await page.evaluate(async () => {
        await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
        const plan = readDriveFolderGridPlans().find(plan => plan.container === driveViewFilesEl);
        // Swapping two rows leaves the observed grid box unchanged. Disconnect
        // during decoding so incidental intermediate frames cannot mask a
        // missing explicit invalidation from refreshFile.
        driveFolderGridObserver.disconnect();
        return { rows: [...plan.rows], columns: plan.columns,
            height: driveViewFilesEl.parentElement.getBoundingClientRect().height,
            plans: folderRustCalls.filter(call => call.method === 'folderRows').length };
    });
    let after;
    try {
        await refreshRows(true);
        after = await page.evaluate(() => {
            const grid = driveViewFilesEl.parentElement;
            const rows = (getComputedStyle(grid).gridTemplateRows.match(/[\d.]+px/g) || []).map(parseFloat);
            readDriveFolderGridPlans();
            return { rows, height: grid.getBoundingClientRect().height,
                plans: folderRustCalls.filter(call => call.method === 'folderRows').length };
        });
    } finally {
        await page.evaluate(() => {
            driveFolderGridObserver.observe(driveViewFoldersEl);
            driveFolderGridObserver.observe(driveViewFilesEl.parentElement);
        });
    }
    expect(before.columns).toBe(3);
    expect(Math.abs(before.rows[0] - before.rows[1])).toBeGreaterThan(1);
    expect(Math.abs(after.height - before.height)).toBeLessThan(1);
    expect(after.rows[0]).toBeCloseTo(before.rows[1], 1);
    expect(after.rows[1]).toBeCloseTo(before.rows[0], 1);
    expect(after.plans).toBeGreaterThan(before.plans);
    await expectSnapshotGeometry(page);
});

test('unavailable WASM retains offline folder navigation and viewport hydration', async ({ page, context }) => {
    await setup(page, context, { folders: 80, rust: false });
    await openCollection(page);
    await page.locator('#drive-folder-view-scroll').evaluate(element => { element.scrollTop = element.scrollHeight; });
    const last = page.locator('#drive-view-folders [data-drive-item-id="folder-79"]');
    await expect(last.locator('.drive-card-title')).toHaveText('Folder 0079');
    await last.click();
    await settle(page);
    await expect(page.locator('#drive-folder-view-title')).toHaveText('Folder 0079');
    await page.locator('#btn-folder-view-back').click();
    await settle(page);
    await expect(page.locator('#drive-folder-view-title')).toHaveText('Collection');
    const calls = await page.evaluate(() => folderRustCalls);
    expect(calls.some(call => call.method === 'folderMotion')).toBe(true);
    expect(calls.some(call => call.method === 'folderRange')).toBe(true);
    expect(await measureLayoutReads(page, 'hydrate')).toBeLessThan(40);
});
