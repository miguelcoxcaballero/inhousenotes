// Share (TESTING §3.3 row "share"), owned by W5.2: the Share modal while Drive answers (skeleton), the people list
// of an owner (owner, editor and viewer rows with avatars and role menus), General access off and on, the Copy link
// confirmation, a reader's owner-only view, and the signed-out public link: read-only viewer, the writer link's
// sign-in gate and the anonymous preview fallback. Scenes are 5.x fixtures (TESTING §3.1), so the states with an
// opened document need W3.2/W3.4 on v6 as well.
import { parity } from '../run.mjs';
import { ANA, BEN, CHO, addAccounts } from '../../mock/fixtures.mjs';

const people = [{ type: 'user', role: 'writer', emailAddress: BEN }, { type: 'user', role: 'reader', emailAddress: CHO }];
const share = async t => { await t.click('#btn-drive-share'); await t.advance(600); };

// The modal before Drive answers: the skeleton rows.
parity('share/loading', { drive: mock => mock.inject({ host: 'www.googleapis.com', path: /\/permissions$/, times: Infinity, hold: true }) }, async t => {
  await t.open({ account: ANA, scene: 'template' });
  await t.click('#btn-drive-share');
  await t.advance(300);
});

// An owner with an editor and a viewer: rows, avatars (photo and fallback), role selects, Remove buttons.
parity('share/people', { systemDark: true }, async t => {
  await t.open({ account: ANA, scene: 'template', sceneOptions: { permissions: people } });
  await share(t);
});

// A role menu used: the editor becomes a viewer and the list is drawn again.
parity('share/role-change', {}, async t => {
  await t.open({ account: ANA, scene: 'template', sceneOptions: { permissions: people } });
  await share(t);
  await t.page.locator('select.share-person-role-select').first().selectOption('reader');
  await t.advance(1200);
});

// General access: Restricted, then Anyone with the link (the role select appears).
parity('share/link-on', {}, async t => {
  await t.open({ account: ANA, scene: 'template' });
  await share(t);
  await t.capture('restricted');
  await t.page.locator('#share-link-access-select').selectOption('anyone');
  await t.advance(1200);
});

// A document already shared with anyone as an editor, then Copy link and its confirmation (or 5.18.0's error).
parity('share/copy', {}, async t => {
  await t.open({ account: ANA, scene: 'template', sceneOptions: { permissions: [{ type: 'anyone', role: 'writer' }] } });
  await share(t);
  await t.capture('anyone-editor');
  await t.click('#btn-copy-share-link');
  await t.advance(1500);
});

// A reader of somebody else's document: only the owner, no sharing controls.
parity('share/non-owner', {}, async t => {
  await t.open({ account: BEN, scene: 'template', sceneOptions: { permissions: [{ type: 'user', role: 'reader', emailAddress: BEN }] } });
  await share(t);
});

// Signed out, a public viewer link: the read-only document from the anonymous download.
parity('share/public-viewer', {}, async t => {
  await t.open({ scene: 'template', sceneOptions: { permissions: [{ type: 'anyone', role: 'reader' }] }, query: '&public=1&role=reader&name=template.pdf' });
  await t.advance(2000);
});

// Signed out, an editor link: the same document behind the sign-in gate.
parity('share/sign-in-gate', {}, async t => {
  await t.open({ scene: 'template', sceneOptions: { permissions: [{ type: 'anyone', role: 'writer' }] }, query: '&public=1&role=writer&name=template.pdf&mode=edit' });
  await t.advance(2000);
});

// Signed out, the download is not a PDF: Google's preview in the editor with the zoom controls.
parity('share/public-preview', { drive: mock => addAccounts(mock).drive.addFile({ id: 'share-not-pdf', name: 'Notes.pdf', owner: ANA, data: Buffer.from('<html>not a PDF</html>'), permissions: [{ type: 'anyone', role: 'reader' }] }) }, async t => {
  await t.open({ query: '&drive=share-not-pdf&public=1&role=reader&name=Notes.pdf' });
  await t.advance(1000);
  await t.capture('fit');
  await t.click('[data-public-preview-action="in"]');
  await t.advance(300);
});
