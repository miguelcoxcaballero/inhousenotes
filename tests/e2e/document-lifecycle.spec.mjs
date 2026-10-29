import { test, expect } from '@playwright/test';

// Home keeps the closed document in memory (a pending exit upload still
// builds from it), but none of its recurring Drive work may keep running:
// before v5.17.22 Home kept ~400 Drive requests/min alive for a closed file.

const DOC = 'lifecycle-doc';

async function installFakeDrive(page) {
  const net = { calls: [], files: new Map(), comments: [], pdf: null };
  net.files.set(DOC, { id: DOC, name: 'Lifecycle.pdf', mimeType: 'application/pdf', version: '5', headRevisionId: 'h5',
    modifiedTime: '2026-10-29T10:00:00.000Z', capabilities: { canEdit: true, canShare: true, canComment: true, canModifyContent: true },
    parents: ['root'], ownedByMe: true, properties: {}, appProperties: {} });
  let serial = 0;
  await page.route('https://accounts.google.com/**', r => r.fulfill({ body: '' }));
  await page.route('https://www.googleapis.com/**', async route => {
    const req = route.request(), url = new URL(req.url()), method = req.method(), path = url.pathname;
    const post = req.postDataBuffer() || Buffer.alloc(0);
    net.calls.push({ at: Date.now(), method, path, media: url.searchParams.get('alt') === 'media', q: url.searchParams.get('q') || '' });
    const doc = net.files.get(DOC);
    const json = (body, status = 200) => route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body),
      headers: { ETag: `"${doc.headRevisionId}"`, 'Access-Control-Expose-Headers': 'ETag' } });
    if (path.endsWith('/about')) return json({ user: { emailAddress: 'lifecycle@example.test', displayName: 'Life', permissionId: 'me' }, storageQuota: {} });
    if (path.endsWith('/generateIds')) return json({ ids: ['gen-' + (++serial)] });
    const id = path.match(/\/files\/([^/]+)/)?.[1];
    if (path.includes('/comments')) {
      if (method === 'POST') { const c = { id: 'c' + (++serial), ...JSON.parse(post.toString() || '{}'), createdTime: new Date().toISOString(), modifiedTime: new Date().toISOString() }; net.comments.push(c); return json(c); }
      if (method === 'DELETE') return json({}, 204);
      return json({ comments: net.comments.slice(-50) });
    }
    if (path.includes('/revisions/') && url.searchParams.get('alt') === 'media') return route.fulfill({ status: 200, contentType: 'application/pdf', body: net.pdf });
    if (path.includes('/revisions')) return json({ revisions: [{ id: doc.headRevisionId, modifiedTime: doc.modifiedTime }] });
    if (path.includes('/permissions')) return json({ permissions: [{ id: 'me', role: 'owner' }] });
    if (path.includes('/upload/')) {
      if (id === DOC) { doc.version = String(+doc.version + 1); doc.headRevisionId = 'h' + (++serial); doc.modifiedTime = new Date().toISOString(); if (post.length > 1000) net.pdf = post; return json({ ...doc }); }
      const nid = id || 'f' + (++serial); net.files.set(nid, { id: nid, version: '1', headRevisionId: 'h' + (++serial), appProperties: {}, properties: {} });
      return json(net.files.get(nid));
    }
    if (path.endsWith('/files') && method === 'POST') { const nid = 'f' + (++serial); net.files.set(nid, { id: nid, ...JSON.parse(post.toString() || '{}'), version: '1' }); return json(net.files.get(nid)); }
    if (path.endsWith('/files')) return json({ files: url.searchParams.get('q')?.includes('ihnKind') ? [] : [{ ...doc }] });
    if (id) {
      const file = net.files.get(id); if (!file) return json({ error: { code: 404 } }, 404);
      if (url.searchParams.get('alt') === 'media') return route.fulfill({ status: 200, contentType: 'application/pdf', body: net.pdf });
      if (method === 'PATCH') { const p = JSON.parse(post.toString() || '{}'); file.properties = { ...file.properties, ...(p.properties || {}) }; file.appProperties = { ...file.appProperties, ...(p.appProperties || {}) }; file.version = String(+file.version + 1); }
      return json({ ...file });
    }
    return json({});
  });
  return net;
}

async function openSignedIn(page, net) {
  await page.addInitScript(() => localStorage.setItem('drive-hint-email', 'lifecycle@example.test'));
  await page.goto('/?e2e=1');
  await page.evaluate(() => __IHN_TEST_API__.ready());
  const bytes = await page.evaluate(async () => {
    await __IHN_TEST_API__.resetLocalDocument(2, 'Lifecycle');
    state.pages[0].strokes.push({ id: 'kept-ink', tool: 'pen', color: '#111111', width: 2, points: [{ x: 20, y: 20, p: .5 }, { x: 80, y: 90, p: .5 }] });
    return [...new Uint8Array(await (await buildPdfBlob({ status: false })).arrayBuffer())];
  });
  net.pdf = Buffer.from(bytes);
  await page.evaluate(async id => {
    const scopes = 'https://www.googleapis.com/auth/drive https://www.googleapis.com/auth/calendar.events';
    localStorage.setItem('google-scope-version-v1', 'drive-calendar-v2'); localStorage.setItem('google-granted-scopes-v1', scopes);
    localStorage.setItem('drive-token-v1', JSON.stringify({ token: 'life-token', expiry: Date.now() + 36e5, scopeVersion: 'drive-calendar-v2', grantedScopes: scopes }));
    driveAccessToken = 'life-token'; driveTokenExpiry = Date.now() + 36e5; driveSignedOut = false;
    await openDriveFile({ id, name: 'Lifecycle.pdf' });
  }, DOC);
  await expect(page.locator('#editor-view')).toBeVisible();
  await expect.poll(() => page.evaluate(() => !!state.collabPollInterval)).toBe(true);
}

// The PDF checkpoint is written in place (Drive v2 PUT or v3 PATCH).
const isPdfUpload = call => call.path.includes('/upload/drive/') && call.path.endsWith(`/files/${DOC}`);

// Requests that belong to the document (not the Home listing / account).
const documentScoped = call => call.path.includes(`/files/${DOC}`) || call.q.includes('ihnKind') || call.path.includes('/comments');

test('Home stops every document-scoped loop and nothing restarts it', async ({ page }) => {
  const net = await installFakeDrive(page);
  await openSignedIn(page, net);
  await page.locator('#btn-drive-home').click();
  await expect(page.locator('#drive-home')).toBeVisible({ timeout: 4000 });
  // Let Home's own one-shot cleanup (presence removal, rendezvous delete) land.
  await page.waitForTimeout(2000);
  const before = net.calls.length;
  // Paths that used to revive the closed document from Home: a save that
  // finishes there, an app resume (visibility/focus/pageshow) and a metadata
  // read carrying presence/mailbox properties.
  await page.evaluate(() => {
    startCollabPolling();
    document.dispatchEvent(new Event('visibilitychange'));
    window.dispatchEvent(new Event('focus'));
    window.dispatchEvent(new Event('pageshow'));
    document.dispatchEvent(new Event('resume'));
    updatePresenceFromProps({ p_other: JSON.stringify({ n: 'Other', c: 'other-device', t: Date.now(), h: Date.now() }) });
  });
  await page.waitForTimeout(6000);
  const idle = net.calls.slice(before).filter(documentScoped);
  expect(idle.map(c => `${c.method} ${c.path}${c.media ? '?alt=media' : ''}${c.q ? '?q' : ''}`)).toEqual([]);
  expect(await page.evaluate(() => ({
    poll: !!state.collabPollInterval, ping: !!_presencePingInterval, heartbeat: !!_presenceHeartbeatInterval,
    live: !!ihnLiveSupervisorTimer, signal: !!ihnLiveSignalTimer, calendar: !!spSyncInterval,
    // The document is still in memory for exit uploads, just inactive.
    file: state.driveFileId, active: isDocumentEditorActive()
  }))).toEqual({ poll: false, ping: false, heartbeat: false, live: false, signal: false, calendar: false, file: DOC, active: false });
});

test('reopening from Home restarts collaboration for the document', async ({ page }) => {
  const net = await installFakeDrive(page);
  await openSignedIn(page, net);
  await page.locator('#btn-drive-home').click();
  await expect(page.locator('#drive-home')).toBeVisible({ timeout: 4000 });
  await expect.poll(() => page.evaluate(() => !!state.collabPollInterval)).toBe(false);
  await page.evaluate(id => openDriveFile({ id, name: 'Lifecycle.pdf' }), DOC);
  await expect(page.locator('#editor-view')).toBeVisible();
  await expect.poll(() => page.evaluate(() => !!state.collabPollInterval && !!_presenceHeartbeatInterval && isDocumentEditorActive())).toBe(true);
  const before = net.calls.length;
  await expect.poll(() => net.calls.slice(before).some(c => c.method === 'GET' && c.path.endsWith(`/files/${DOC}`)), { timeout: 5000 }).toBe(true);
  expect(await page.evaluate(() => state.pages[0].strokes.map(s => s.id))).toContain('kept-ink');
});

test('a document left before its Drive save finished is still uploaded from Home', async ({ page }) => {
  const net = await installFakeDrive(page);
  await openSignedIn(page, net);
  const uploadsBefore = net.calls.filter(c => isPdfUpload(c)).length;
  await page.evaluate(() => {
    const stroke = { id: 'left-before-save', tool: 'pen', color: '#111111', width: 2, points: [{ x: 30, y: 30, p: .5 }, { x: 90, y: 70, p: .5 }] };
    stampCollaborativeItems([stroke]);
    state.pages[0].strokes.push(stroke); markPageDirty(0, 'full');
    bumpDriveContentVersion(); driveDirty = true;
    // Keep the editor's own debounce from saving first.
    clearDriveSyncTimers();
  });
  await page.locator('#btn-drive-home').click();
  await expect(page.locator('#drive-home')).toBeVisible({ timeout: 4000 });
  await expect.poll(() => net.calls.filter(c => isPdfUpload(c)).length, { timeout: 20000 }).toBeGreaterThan(uploadsBefore);
  await expect.poll(() => page.evaluate(() => readPendingExitUpload()), { timeout: 20000 }).toBeNull();
  expect(net.pdf.toString('latin1').length).toBeGreaterThan(1000);
  // The finished exit upload must not have revived polling for the closed file.
  expect(await page.evaluate(() => !!state.collabPollInterval || !!ihnLiveSupervisorTimer)).toBe(false);
});

test('a hidden editor pauses Drive polling and calendar sync, then resumes when visible', async ({ page }) => {
  const net = await installFakeDrive(page);
  await openSignedIn(page, net);
  const setVisibility = value => page.evaluate(v => {
    Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => v });
    Object.defineProperty(document, 'hidden', { configurable: true, get: () => v === 'hidden' });
    document.dispatchEvent(new Event('visibilitychange'));
  }, value);
  await setVisibility('hidden');
  expect(await page.evaluate(() => ({ poll: !!state.collabPollInterval, heartbeat: !!_presenceHeartbeatInterval, calendar: !!spSyncInterval })))
    .toEqual({ poll: false, heartbeat: false, calendar: false });
  await page.waitForTimeout(800);
  const before = net.calls.length;
  await page.waitForTimeout(3000);
  expect(net.calls.slice(before).filter(c => c.method === 'GET' && c.path.endsWith(`/files/${DOC}`) && !c.media).length).toBeLessThanOrEqual(1);
  await setVisibility('visible');
  expect(await page.evaluate(() => ({ poll: !!state.collabPollInterval, heartbeat: !!_presenceHeartbeatInterval, calendar: !!spSyncInterval })))
    .toEqual({ poll: true, heartbeat: true, calendar: true });
});
