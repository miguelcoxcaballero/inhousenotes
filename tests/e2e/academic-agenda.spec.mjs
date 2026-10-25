import { test, expect } from '@playwright/test';

test.beforeEach(async ({ page }) => {
    await page.goto('/?e2e=1');
    await page.evaluate(() => __IHN_TEST_API__.ready());
});

test('academic year rolls over in September, not January; diary remains annual', async ({ page }) => {
    const result = await page.evaluate(() => [
        getDriveQuickAccessPeriod('agenda', new Date(2026, 7, 31)).fileBaseName,
        getDriveQuickAccessPeriod('agenda', new Date(2026, 8, 1)).fileBaseName,
        getDriveQuickAccessPeriod('agenda', new Date(2027, 0, 1)).fileBaseName,
        getDriveQuickAccessPeriod('diary', new Date(2027, 0, 1)).fileBaseName,
        isQuickAccessDriveFile({ name: 'Agenda2026-2027.pdf' })
    ]);
    expect(result).toEqual(['Agenda2025-2026', 'Agenda2026-2027', 'Agenda2026-2027', 'Diary2027', true]);
});

test('weekly agenda covers the entire course with unique dates including leap day', async ({ page }) => {
    const results = await page.evaluate(() => [2026, 2027].map(year => {
        const agenda = buildAcademicAgenda(year);
        const keys = agenda.pages.flatMap(p => p.sidePanel.dateKeys);
        return { count: agenda.pages.length, unique: new Set(keys).size === keys.length,
            september: keys.includes(`${year}-09-01`), august: keys.includes(`${year + 1}-08-31`),
            leap: keys.includes('2028-02-29'), mondays: agenda.pages.every(p => dateFromCalendarKey(p.sidePanel.dateKeys[0]).getDay() === 1),
            ids: new Set(agenda.pages.map(p => p.pageId)).size, mode: agenda.calendarPageConfig.mode };
    }));
    for (const result of results) {
        expect(result.count).toBeGreaterThanOrEqual(52);
        expect(result.count).toBeLessThanOrEqual(54);
        expect(result.ids).toBe(result.count);
        expect(result).toMatchObject({ unique: true, september: true, august: true, mondays: true, mode: 'week' });
    }
    expect(results[1].leap).toBe(true);
});

test('quick access creates this course and then reopens it without touching the old agenda', async ({ page }) => {
    const result = await page.evaluate(async () => {
        const calls = [];
        let created;
        driveIsOffline = () => false;
        ensureDriveToken = async () => {};
        ensureNamedDriveFolder = async () => 'agenda-folder';
        getDriveQuickAccessPeriod = () => ({ label: '2026–2027', fileBaseName: 'Agenda2026-2027', startYear: 2026 });
        findDrivePdfInFolder = async (folder, name) => { calls.push(['find', name]); return created || null; };
        createNewDriveDocumentWithFolder = async (folder, name, options) => {
            calls.push(['create', name, options.academicStartYear]);
            created = { id: 'new-course', name: `${name}.pdf` };
        };
        openDriveFile = async file => calls.push(['open', file.id]);
        refreshDriveFiles = async () => {};
        await openQuickAccessDriveDocument('agenda');
        await openQuickAccessDriveDocument('agenda');
        return { calls, busy: driveQuickAccessInProgress };
    });
    expect(result).toEqual({ calls: [['find', 'Agenda2026-2027'], ['create', 'Agenda2026-2027', 2026],
        ['find', 'Agenda2026-2027'], ['open', 'new-course']], busy: false });
});

test('new agenda initializes all weekly pages before the first save', async ({ page }) => {
    const result = await page.evaluate(async () => {
        await __IHN_TEST_API__.resetLocalDocument(1, 'Previous');
        window.InhouseOfflineLibrary = { ...window.InhouseOfflineLibrary, ready: () => true, capture: async () => {} };
        let snapshot;
        window.InhouseOfflineApp = { ...window.InhouseOfflineApp, newDocument: async () => {
            snapshot = { count: state.pages.length, kind: state.driveFolderKind, name: state.driveFileName,
                config: state.calendarPageConfig, dates: state.pages[0].sidePanel.dateKeys };
        } };
        const ok = await createNewDriveDocumentWithFolder('agenda-folder', 'Agenda2026-2027', {
            templateKind: 'agenda', driveQuickAccessKind: 'agenda', academicStartYear: 2026
        });
        return { ok, snapshot };
    });
    expect(result.ok).toBe(true);
    expect(result.snapshot).toMatchObject({ count: 53, kind: 'agenda', name: 'Agenda2026-2027.pdf',
        config: { mode: 'week', startPage: 0, startDateKey: '2026-08-31', nextDateKey: '2027-09-06' } });
    expect(result.snapshot.dates).toContain('2026-09-01');
});

test('pending local agenda is reused online before Drive upload finishes', async ({ page }) => {
    const result = await page.evaluate(async () => {
        driveIsOffline = () => false;
        window.InhouseOfflineLibrary = { ready: () => true, folder: id => id === 'root'
            ? [{ id: 'local-folder', name: 'Agenda', mimeType: 'application/vnd.google-apps.folder' }]
            : [{ id: 'local-course', name: 'Agenda2026-2027.pdf', mimeType: 'application/pdf' }] };
        driveFetch = async () => { throw new Error('Pending local records must not be queried on Drive'); };
        const folder = await ensureNamedDriveFolder('Agenda');
        const file = await findDrivePdfInFolder(folder, 'Agenda2026-2027');
        return { folder, id: file.id };
    });
    expect(result).toEqual({ folder: 'local-folder', id: 'local-course' });
});

test('course agenda survives offline close and reopen with all dated pages', async ({ page, context }) => {
    test.setTimeout(60_000);
    await page.evaluate(() => {
        localStorage.setItem('drive-hint-email', 'agenda@example.test');
        localStorage.setItem('drive-remember-v1', '1');
    });
    await page.reload();
    await page.evaluate(() => __IHN_TEST_API__.ready());
    await page.waitForFunction(() => InhouseOfflineLibrary.ready());
    await context.setOffline(true);
    const saved = await page.evaluate(async () => {
        await openQuickAccessDriveDocument('agenda');
        await __IHN_TEST_API__.checkpointBeforeLeaving();
        await InhouseOfflineLibrary.capture();
        const record = await InhouseOfflineStore.read('documents', InhouseOfflineLibrary.account(), state.offlineLocalId);
        return { id: state.driveFileId, pages: record.snapshot.pages.length,
            start: record.snapshot.calendarPageConfig.startDateKey, name: state.driveFileName };
    });
    expect(saved.pages).toBeGreaterThanOrEqual(52);
    await page.evaluate(() => showDriveHome());
    const reopened = await page.evaluate(async () => {
        await openQuickAccessDriveDocument('agenda');
        return { id: state.driveFileId, pages: state.pages.length, start: state.calendarPageConfig?.startDateKey };
    });
    expect(reopened).toEqual({ id: saved.id, pages: saved.pages, start: saved.start });
});
