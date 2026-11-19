// Home shell (TESTING §3.3 row "home-shell / auth"): boot skeletons, empty Home, Home with lists, reconnect state,
// offline line, failed sign-in. Home content beyond the shell belongs to W4.1 (`home.mjs`).
import { parity } from '../run.mjs';
import { fixtures, ANA } from '../../mock/fixtures.mjs';

const signIn = async t => { await t.click('#btn-welcome-start'); await t.advance(400); await t.click('#btn-welcome-login'); };

// A stored session: the boot script paints the Home skeleton before the app starts.
parity('home-shell/boot-skeleton', { systemDark: true, drive: fixtures.library }, async t => {
  await t.open({ account: ANA, settleMs: 0, ready: false });
});

// Lists requested and not answered yet: skeleton cards and the status skeleton.
parity('home-shell/lists-loading', { drive: mock => { fixtures.library(mock); mock.inject({ host: 'www.googleapis.com', path: /\/drive\/v3\/files$/, times: Infinity, hold: true }); } }, async t => {
  await t.open({ account: ANA });
});

parity('home-shell/empty', { systemDark: true, app: true, drive: fixtures.empty }, async t => {
  await t.open({ account: ANA });
});

// First sign-in through the Google button (GIS stub grants Ana): Home with recents, starred, shared.
parity('home-shell/lists', { systemDark: true, drive: fixtures.library }, async t => {
  await t.open();
  await signIn(t);
  await t.advance(3000);
});

// A returning user (stored session), after the first periodic Home refresh (30 s).
parity('home-shell/lists-returning', { drive: fixtures.library }, async t => {
  await t.open({ account: ANA });
  await t.advance(32_000, { step: 100 });
});

// The stored token has expired and Google refuses a silent refresh: Home asks to reconnect.
parity('home-shell/reconnect', { drive: mock => { fixtures.library(mock); mock.gis.mode = 'silent-fail'; } }, async t => {
  await t.open({ account: ANA, storage: { 'drive-token-v1': JSON.stringify({ token: 'expired', expiry: 1, grantedScopes: 'https://www.googleapis.com/auth/drive' }) }, settleMs: 12_000 });
});

// Every request fails (connectivity probe included): cached session, offline status line.
parity('home-shell/offline-line', { drive: mock => { fixtures.library(mock); mock.offline = true; } }, async t => {
  await t.open({ account: ANA, settleMs: 8000 });
});

// Google denies the consent: back on Welcome with "Sign-in failed. Try again."
parity('home-shell/sign-in-failed', { drive: mock => { fixtures.empty(mock); mock.gis.mode = 'deny'; } }, async t => {
  await t.open();
  await signIn(t);
  await t.advance(1500);
});
