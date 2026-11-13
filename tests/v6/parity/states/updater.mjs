// The shared updater's screens in the Android app (TESTING §3.3 row "updater"; spec/shell §14.5): the JavaScript
// is the same file in both apps, so this pins how v6 creates it — version, channel, native bridge, container.
import { parity as register } from '../run.mjs';
import { shellOnly } from '../support/shell-only.mjs';
import { fixtures, ANA } from '../../mock/fixtures.mjs';

const parity = (name, options, script) => register(name, options, shellOnly(script));
const APP = { only: { modes: ['app'] } };
const offer = mock => { fixtures.empty(mock); mock.github.setCatalog('B', [{ version: '1.0.29', versionCode: 30, changelog: 'Inhouse Notes 6.' }]); };
const dialog = '#inhouse-services-update';

// The only intended difference in these states: the version label of the open profile menu (v5.18.0 → v6.0.0).
const VERSION = { tags: ['VERSION-LABEL'], expected: t => t.page.evaluate(() => [...document.querySelectorAll('[data-app-version]')].map(n => n.getBoundingClientRect()).filter(r => r.width).map(r => ({ x: r.x, y: r.y, width: r.width, height: r.height }))) };

// Signed out: the offer covers Welcome.
parity('updater/offer', { ...APP, drive: offer }, async t => {
  await t.open();
  await t.until(() => !!document.querySelector('#inhouse-services-update [data-isu-install]'), { timeout: 8000 });
  await t.advance(600);
});

// Update: the native bridge takes over and reports progress.
parity('updater/installing', { ...APP, drive: offer }, async t => {
  await t.open();
  await t.until(() => !!document.querySelector('#inhouse-services-update [data-isu-install]'), { timeout: 8000 });
  await t.click(`${dialog} [data-isu-install]`);
  await t.advance(300);
  await t.capture('started');
  await t.page.evaluate(() => window.handleInhouseUpdateResult({ status: 'downloading', percent: 50, downloadedBytes: 3407876, totalBytes: 6815752 }));
  await t.advance(600);
  await t.capture('half');
});

// Details → Change → the A/B settings → Version A asks for the development warning.
parity('updater/channels', { ...APP, drive: offer }, async t => {
  await t.open();
  await t.until(() => !!document.querySelector('#inhouse-services-update [data-isu-install]'), { timeout: 8000 });
  await t.click(`${dialog} .isu-disclosure`);
  await t.advance(600);
  await t.capture('details');
  await t.click(`${dialog} [aria-label="Change channel"]`);
  await t.advance(600);
  await t.capture('settings');
  await t.click(`${dialog} .isu-item:has(#isu-channel-A)`);
  await t.advance(600);
  await t.capture('version-a-warning');
});

// Later snoozes the offer; the settings entry of the profile menu opens the same settings with nothing to install.
parity('updater/menu-entry', { ...APP, ...VERSION, drive: mock => fixtures.empty(mock) }, async t => {
  await t.open({ account: ANA });
  await t.click('#btn-drive-profile');
  await t.advance(300);
  await t.capture('menu');
  await t.click('#drive-profile-menu [data-inhouse-update-settings]');
  await t.advance(800);
  await t.capture('settings');
});
