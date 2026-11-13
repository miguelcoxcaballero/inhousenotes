// Sign-in and session states owned by W1.3 (TESTING §3.3 row "home-shell / auth"): the Welcome sheet in the Android
// app, the first sign-in landing on Home, a returning session, a paused connection and sign-out. The Welcome
// screen itself is `welcome.mjs`; Home's lists are `home-shell.mjs` / `home.mjs` (W4.1).
import { parity } from '../run.mjs';
import { fixtures, ANA } from '../../mock/fixtures.mjs';

const EXPIRED = { 'drive-token-v1': JSON.stringify({ token: 'expired', expiry: 1, grantedScopes: 'https://www.googleapis.com/auth/drive' }) };
const signIn = async t => { await t.click('#btn-welcome-start'); await t.advance(400); await t.click('#btn-welcome-login'); };

// The APK never loads Google Identity Services: the sheet's button hands the PKCE URL to the native bridge and
// Welcome stays as it is.
parity('auth/app-pkce-started', { only: { modes: ['app'] }, drive: fixtures.empty }, async t => {
  await t.open();
  await signIn(t);
  await t.advance(600);
});

// Web: the consent was granted a moment ago — Home's top bar with the profile button, before the profile arrives.
parity('auth/signed-in-topbar', { drive: mock => { fixtures.empty(mock); mock.inject({ host: 'www.googleapis.com', path: /\/drive\/v3\/(files|about)$/, times: Infinity, hold: true }); } }, async t => {
  await t.open();
  await signIn(t);
  await t.advance(200);
});

// A stored session with a cached profile, Drive not answering yet: avatar from the cache, boot skeleton.
parity('auth/returning-skeleton', { systemDark: true, drive: mock => { fixtures.empty(mock); mock.inject({ host: 'www.googleapis.com', path: /\/drive\/v3\/(files|about)$/, times: Infinity, hold: true }); } }, async t => {
  await t.open({ account: ANA, settleMs: 400 });
});

// The token expired and Google refuses to renew it silently: "Reconnect" beside the profile button.
parity('auth/reconnect', { drive: mock => { fixtures.empty(mock); mock.gis.mode = 'silent-fail'; } }, async t => {
  await t.open({ account: ANA, storage: EXPIRED, settleMs: 12_000 });
});

// Sign out from the profile menu: back on Welcome, the "Signed out" status line while it lasts and after it.
parity('auth/signed-out', { drive: fixtures.empty }, async t => {
  await t.open({ account: ANA });
  await t.click('#btn-drive-profile');
  await t.click('#btn-drive-menu-logout');
  await t.advance(300);
  await t.capture('status');
  await t.advance(6000);
  await t.capture('rest');
});
