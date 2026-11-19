// 07-share (TESTING §2, BUILD-PLAN W5.2): the Share modal against the mock Drive — add a person (permissions POST),
// change and remove a role, link to anyone, its role, Copy link with 5.18.0's exact URL — then that link opened
// signed out: a read-only document from the anonymous download, an owner's later export picked up by the 2.5 s
// poll, the writer link's sign-in gate, the preview fallback, and never an API `key=` in any request.
import zlib from 'node:zlib';
import { test, expect, seedSession, ANA, BEN } from '../mock/fixtures.mjs';

test.use({ serviceWorkers: 'block' });

const PNG = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==', 'base64');

/** A PDF with `/Keywords` and embedded files (the v6 container of ARCHITECTURE §8.2, as internal-doc builds it). */
function containerPdf({ keywords, attachments = [], pages = 3 }) {
  const chunks = [Buffer.from('%PDF-1.7\n%\xe2\xe3\xcf\xd3\n', 'latin1')], offsets = [];
  let size = chunks[0].length;
  const push = buffer => { chunks.push(buffer); size += buffer.length; };
  const object = (number, body, stream) => {
    offsets[number] = size;
    push(Buffer.from(`${number} 0 obj\n${body}\n`, 'latin1'));
    if (stream) { push(Buffer.from('stream\n', 'latin1')); push(stream); push(Buffer.from('\nendstream\n', 'latin1')); }
    push(Buffer.from('endobj\n', 'latin1'));
  };
  const first = 4, files = first + pages;
  const names = attachments.map((a, i) => `(${a.name}) ${files + i * 2} 0 R`).join(' ');
  object(1, `<< /Type /Catalog /Pages 2 0 R${attachments.length ? ` /Names << /EmbeddedFiles << /Names [${names}] >> >>` : ''} >>`);
  object(2, `<< /Type /Pages /Kids [${Array.from({ length: pages }, (_, i) => `${first + i} 0 R`).join(' ')}] /Count ${pages} >>`);
  object(3, `<< /Title (Cuaderno Digital) /Keywords (${keywords}) >>`);
  for (let i = 0; i < pages; i++) object(first + i, '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595.5 842.25] >>');
  attachments.forEach((a, i) => {
    object(files + i * 2, `<< /Type /Filespec /F (${a.name}) /UF (${a.name}) /EF << /F ${files + i * 2 + 1} 0 R >> >>`);
    object(files + i * 2 + 1, `<< /Type /EmbeddedFile /Length ${a.data.length} >>`, a.data);
  });
  const count = files + attachments.length * 2, xref = size;
  let table = `xref\n0 ${count}\n0000000000 65535 f \n`;
  for (let n = 1; n < count; n++) table += `${String(offsets[n]).padStart(10, '0')} 00000 n \n`;
  push(Buffer.from(`${table}trailer\n<< /Size ${count} /Root 1 0 R /Info 3 0 R >>\nstartxref\n${xref}\n%%EOF\n`, 'latin1'));
  return Buffer.concat(chunks);
}

const cmd = (page, name, args) => page.evaluate(([n, a]) => window.__IHN_TEST_API__.cmd(n, a), [name, args]);
async function open(page, url) {
  await page.goto(url);
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  expect(await page.evaluate(() => window.__IHN_TEST_API__.ready())).toBe(true);
}
const opened = (page, id) => expect.poll(async () => (await cmd(page, 'doc.open.state')).log.some(e => e[0] === id && e[1]), { timeout: 15_000 }).toBe(true);
const status = page => page.locator('#save-indicator');
let snapshot;
async function container(page, extraPages = 0) {
  snapshot ??= Buffer.from(await cmd(page, 'doc.session.fixture'), 'base64');
  return containerPdf({ keywords: `STROKES_Z:!;IHN6:${zlib.deflateSync(snapshot).toString('base64')}`, attachments: [{ name: `IHN6-${'6a'.repeat(32)}`, data: PNG }], pages: 3 + extraPages });
}
const noApiKey = mock => expect(mock.calls.filter(c => /[?&]key=/.test(c.url)).map(c => c.url)).toEqual([]);
const perms = (mock, id) => mock.drive.file(id).permissions.map(p => `${p.type}:${p.role}:${p.emailAddress || ''}`);

test('share modal: add, change and remove a person, link to anyone, copy the exact link', async ({ page, context, mock, problems }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await seedSession(context, mock);
  await open(page, '/?e2e=1');
  mock.drive.addFile({ id: 'v6-doc', name: 'Fixture.pdf', owner: ANA, data: await container(page), modifiedTime: '2026-10-29T09:00:00.000Z' });
  await open(page, '/?drive=v6-doc&e2e=1');
  await opened(page, 'v6-doc');
  await expect(page.locator('#btn-drive-share')).toBeVisible();

  await page.click('#btn-drive-share');
  await expect(page.locator('#share-modal-overlay')).toHaveClass(/visible/);
  await expect(page.locator('#share-modal-overlay')).not.toHaveClass(/loading/);
  await expect(page.locator('#share-access-list .share-person-row')).toHaveCount(1);
  await expect(page.locator('.share-person-primary')).toHaveText('Ana Owner');
  await expect(page.locator('.share-person-role')).toHaveText('Owner');
  await expect(page.locator('#share-invite-form')).toBeVisible();
  await expect(page.locator('#share-link-access-select')).toHaveValue('restricted');
  await expect(page.locator('#share-link-role-select')).toBeHidden();

  // Add Ben as an editor.
  await page.fill('#share-email-input', BEN);
  await page.selectOption('#share-role-select', 'writer');
  await page.click('#btn-send-share');
  await expect(page.locator('#share-access-list .share-person-row')).toHaveCount(2);
  const post = mock.calls.find(c => c.method === 'POST' && /\/permissions\?sendNotificationEmail=true$/.test(c.url));
  expect(post).toBeTruthy();
  expect(perms(mock, 'v6-doc')).toContain(`user:writer:${BEN}`);
  await expect(page.locator('#share-email-input')).toHaveValue('');
  await expect(page.locator('#save-indicator')).toHaveAttribute('aria-label', /Invitation sent/);
  const benRole = page.locator('select.share-person-role-select');
  await expect(benRole).toHaveAttribute('aria-label', 'Role for Ben Writer');
  await expect(benRole).toHaveValue('writer');

  // Change his role, then remove him.
  await benRole.selectOption('reader');
  await expect.poll(() => perms(mock, 'v6-doc')).toContain(`user:reader:${BEN}`);
  await expect(page.locator('select.share-person-role-select')).toHaveValue('reader');
  await page.click('.share-person-remove');
  await expect(page.locator('#share-access-list .share-person-row')).toHaveCount(1);
  expect(perms(mock, 'v6-doc')).not.toContain(`user:reader:${BEN}`);

  // General access: anyone with the link (Viewer), then Editor.
  await page.selectOption('#share-link-access-select', 'anyone');
  await expect.poll(() => perms(mock, 'v6-doc')).toContain('anyone:reader:');
  await expect(page.locator('#share-link-role-select')).toBeVisible();
  await page.selectOption('#share-link-role-select', 'writer');
  await expect.poll(() => perms(mock, 'v6-doc')).toContain('anyone:writer:');
  await expect(page.locator('#share-link-role-select')).toHaveValue('writer');

  // Copy link: the editor link, exactly 5.18.0's format.
  await page.click('#btn-copy-share-link');
  await expect.poll(async () => (await cmd(page, 'panels.share.state')).copied).not.toBe('');
  const base = new URL(page.url()).origin;
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(`${base}/?drive=v6-doc&public=1&role=writer&name=Fixture.pdf&mode=edit`);
  await expect(page.locator('#btn-copy-share-link')).toHaveText('Copy link');
  await expect(status(page)).toHaveAttribute('aria-label', 'Editor link copied. Recipients sign in to save changes to Drive.');
  // Back to a viewer link; Restricted removes the link permission.
  await page.selectOption('#share-link-role-select', 'reader');
  await expect.poll(() => perms(mock, 'v6-doc')).toContain('anyone:reader:');
  await page.click('#btn-copy-share-link');
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(`${base}/?drive=v6-doc&public=1&role=reader&name=Fixture.pdf`);
  await page.selectOption('#share-link-access-select', 'restricted');
  await expect.poll(() => perms(mock, 'v6-doc').some(p => p.startsWith('anyone'))).toBe(false);

  await page.click('#btn-close-share-modal');
  await expect(page.locator('#share-modal-overlay')).not.toHaveClass(/visible/);
  noApiKey(mock);
  expect(problems).toEqual([]);
});

test('a reader of another account sees only the owner and no sharing controls', async ({ page, context, mock, problems }) => {
  await seedSession(context, mock, BEN);
  await open(page, '/?e2e=1');
  mock.drive.addFile({ id: 'v6-doc', name: 'Fixture.pdf', owner: ANA, data: await container(page), permissions: [{ type: 'user', role: 'reader', emailAddress: BEN }] });
  await open(page, '/?drive=v6-doc&e2e=1');
  await opened(page, 'v6-doc');
  await page.click('#btn-drive-share');
  await expect(page.locator('#share-modal-overlay')).not.toHaveClass(/loading/);
  await expect(page.locator('#share-access-list .share-person-row')).toHaveCount(1);
  await expect(page.locator('.share-person-primary')).toHaveText('Ana Owner');
  await expect(page.locator('#share-invite-form')).toBeHidden();
  await expect(page.locator('#btn-copy-share-link')).toBeHidden();
  // A click on the backdrop closes it; one inside the dialog does not.
  await page.click('#share-modal-title');
  await expect(page.locator('#share-modal-overlay')).toHaveClass(/visible/);
  await page.mouse.click(5, 5);
  await expect(page.locator('#share-modal-overlay')).not.toHaveClass(/visible/);
  expect(problems).toEqual([]);
});

test('signed out: the public link opens read-only, follows the owner\'s next export within the poll, never sends key=', async ({ page, mock, problems }) => {
  await open(page, '/?e2e=1');
  mock.drive.addFile({ id: 'v6-doc', name: 'Fixture.pdf', owner: ANA, data: await container(page), permissions: [{ type: 'anyone', role: 'reader' }] });
  await open(page, '/?drive=v6-doc&public=1&role=reader&name=Fixture.pdf&e2e=1');
  await opened(page, 'v6-doc');
  await expect(page.locator('#editor-view')).toBeVisible();
  expect(await cmd(page, 'doc.session.state')).toMatchObject({ docId: 'v6-doc', canEdit: false, writable: false, persistent: false, pages: 3 });
  await expect(page.locator('#readonly-banner')).toHaveClass(/hidden/);
  await expect(page.locator('#mode-toggle-label')).toHaveText('Read only');
  await expect(status(page)).toHaveAttribute('aria-label', 'Shared PDF loaded (View Only)');
  expect(mock.calls.some(c => c.host === 'drive.usercontent.google.com')).toBe(true);
  expect(mock.calls.filter(c => c.host === 'www.googleapis.com' && c.path.includes('/files'))).toEqual([]);
  await expect.poll(async () => (await cmd(page, 'doc.public.state')).polling).toBe(true);

  // The owner exports again: the signed-out view takes it within one poll.
  const before = (await cmd(page, 'doc.public.state'));
  const t0 = Date.now();
  mock.drive.handle({ method: 'PATCH', url: 'https://www.googleapis.com/upload/drive/v3/files/v6-doc?uploadType=media', headers: { authorization: `Bearer ${mock.drive.accounts.get(ANA).token}`, 'content-type': 'application/pdf' }, body: await container(page, 1) });
  await expect.poll(async () => (await cmd(page, 'doc.public.state')).updates, { timeout: 6000, intervals: [100] }).toBe(1);
  expect(Date.now() - t0).toBeLessThan(2500 + 2500);
  const after = await cmd(page, 'doc.public.state');
  expect(after.fingerprint).not.toBe(before.fingerprint);
  await expect(status(page)).toHaveAttribute('aria-label', 'Public document updated.');
  expect((await cmd(page, 'doc.session.state')).canEdit).toBe(false);
  noApiKey(mock);
  expect(problems).toEqual([]);
});

test('signed out: an editor link shows the sign-in gate; a refused download falls back to the preview', async ({ page, mock, problems }) => {
  await open(page, '/?e2e=1');
  mock.drive.addFile({ id: 'v6-doc', name: 'Fixture.pdf', owner: ANA, data: await container(page), permissions: [{ type: 'anyone', role: 'writer' }] });
  await open(page, '/?drive=v6-doc&public=1&role=writer&name=Fixture.pdf&mode=edit&e2e=1');
  await opened(page, 'v6-doc');
  await expect(page.locator('#readonly-banner')).not.toHaveClass(/hidden/);
  await expect(page.locator('#readonly-banner')).toHaveAttribute('data-shared-editor-gate', 'true');
  await expect(page.locator('#readonly-banner-message')).toHaveText('Editor link Sign in to edit and save.');
  await expect(page.locator('#btn-ask-edit')).toHaveText('Sign in');
  await expect(page.locator('#btn-make-copy')).toBeHidden();
  await expect(status(page)).toHaveAttribute('aria-label', 'Shared PDF loaded. Sign in to edit and save.');
  expect((await cmd(page, 'doc.public.state')).gate).toBe(true);

  // Not a PDF behind the link: Google's preview iframe, polled and reloaded.
  mock.drive.addFile({ id: 'not-pdf', name: 'x.pdf', owner: ANA, data: Buffer.from('hello'), permissions: [{ type: 'anyone', role: 'reader' }] });
  await open(page, '/?drive=not-pdf&public=1&role=reader&e2e=1');
  await expect(page.locator('#public-inline-preview iframe')).toHaveCount(1);
  await expect(page.locator('#public-inline-preview iframe')).toHaveAttribute('src', /^https:\/\/drive\.google\.com\/file\/d\/not-pdf\/preview\?embedded=true&ihn_refresh=\d+$/);
  await expect(page.locator('#public-inline-preview iframe')).toHaveClass(/ready/);
  await expect(page.locator('#drive-status')).toHaveText('Opening public view — no sign-in required.');
  await expect(status(page)).toHaveAttribute('aria-label', 'Public document opened — no sign-in required.');
  await expect(page.locator('#readonly-banner')).toHaveClass(/hidden/);
  await expect(page.locator('body')).toHaveClass(/public-preview-mode/);
  expect(await cmd(page, 'doc.public.state')).toMatchObject({ preview: true, previewOnly: true, polling: true });
  // Zoom controls.
  await page.click('[data-public-preview-action="in"]');
  await expect(page.locator('[data-public-preview-action="fit"]')).toHaveText('125%');
  await page.click('[data-public-preview-action="fit"]');
  await expect(page.locator('[data-public-preview-action="fit"]')).toHaveText('Fit');
  // The failing poll keeps downloading every 2.5 s.
  const downloads = (await cmd(page, 'doc.public.state')).downloads;
  await expect.poll(async () => (await cmd(page, 'doc.public.state')).downloads, { timeout: 6000 }).toBeGreaterThan(downloads);
  noApiKey(mock);
  expect(problems).toEqual([]);
});
