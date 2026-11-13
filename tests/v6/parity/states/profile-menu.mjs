// The Home profile menu and what it starts (TESTING §3.3 row "profile-menu"): the menu itself, night mode, Refresh,
// the fullscreen button, the Soon toast and the offline status line. Home's lists behind the menu are W4.1's.
import { parity } from '../run.mjs';
import { fixtures, ANA } from '../../mock/fixtures.mjs';

// The only intended difference in these states: the version label of the open profile menu (v5.18.0 → v6.0.0).
const VERSION = { tags: ['VERSION-LABEL'], expected: t => t.page.evaluate(() => [...document.querySelectorAll('[data-app-version]')].map(n => n.getBoundingClientRect()).filter(r => r.width).map(r => ({ x: r.x, y: r.y, width: r.width, height: r.height }))) };
const menu = async t => { await t.open({ account: ANA }); await t.click('#btn-drive-profile'); await t.advance(300); };

parity('profile-menu/home', { ...VERSION, systemDark: true, app: true, drive: fixtures.empty }, menu);

// Night mode from the menu: the toggle moves, the whole app follows; a second tap returns.
parity('profile-menu/night-mode', { ...VERSION, drive: fixtures.empty }, async t => {
  await menu(t);
  await t.click('#theme-toggle');
  await t.advance(400);
  await t.capture('toggled');
  await t.click('#btn-drive-profile');
  await t.advance(300);
  await t.capture('menu-closed');
  await t.click('#btn-drive-profile');
  await t.click('#theme-toggle');
  await t.advance(400);
  await t.capture('back');
});

// Refresh closes the menu and lists Drive again.
parity('profile-menu/refresh', { drive: fixtures.empty }, async t => {
  await menu(t);
  await t.click('#btn-drive-menu-refresh');
  await t.advance(50);
  await t.capture('started');
  await t.advance(3000);
  await t.capture('done');
});

// The fullscreen button (web only; the APK hides it): icon and title follow `fullscreenchange`.
parity('profile-menu/fullscreen', { only: { viewports: ['desktop-1280', 'desktop-1920'], modes: ['web'] }, drive: fixtures.empty }, async t => {
  await t.open({ account: ANA });
  await t.click('#btn-drive-fullscreen');
  await t.advance(300);
  await t.capture('entered');
  await t.hover('#btn-drive-fullscreen');
  await t.click('#btn-drive-fullscreen');
  await t.advance(300);
  await t.capture('left');
});

// Diary and Agenda only answer "Soon" (BEHAVIOUR-CHANGES S-2 is already 5.18.0 behaviour).
parity('profile-menu/soon', { drive: fixtures.empty }, async t => {
  await menu(t);
  await t.click('#btn-drive-menu-diary');
  await t.advance(300);
  await t.capture('toast');
  await t.advance(1700);
  await t.capture('gone');
});

// Drive unreachable with a stored session: the offline status line and the menu over it.
parity('profile-menu/offline', { ...VERSION, drive: mock => { fixtures.empty(mock); mock.offline = true; } }, async t => {
  await t.open({ account: ANA, settleMs: 8000 });
  await t.capture('status-line');
  await t.click('#btn-drive-profile');
  await t.advance(300);
  await t.capture('menu');
});
