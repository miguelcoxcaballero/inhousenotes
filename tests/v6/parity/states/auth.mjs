// Sign-in and session states owned by W1.3 (TESTING §3.3 row "home-shell / auth"): the Welcome sheet in the Android
// app, the first sign-in landing on Home, a returning session, a paused connection and sign-out. The Welcome
// screen itself is `welcome.mjs`; Home's lists are `home-shell.mjs` / `home.mjs` (W4.1).
//
// What 5.18.0 shows in Home's list area decides which state is which (measured on 5.18.0, see NOTES-W1.3):
//   - skeleton cards: only while its offline library is NOT ready, i.e. no account is known yet — the first sign-in
//     (`showDriveSkeletons` from `refreshDriveFiles`) and a remembered session without `drive-hint-email` (the boot
//     script's skeleton, then the same one from `refreshDriveFiles`);
//   - the cached library (here: the three "empty" messages): a returning session of a known account, at once — its
//     boot skeleton is gone before the page is first idle, so there is no skeleton to compare in that state.
import { parity as register } from '../run.mjs';
import { shellOnly } from '../support/shell-only.mjs';
import { EPOCH } from '../matrix.mjs';
import { fixtures, ANA } from '../../mock/fixtures.mjs';

const parity = (name, options, script) => register(name, options, shellOnly(script));
const EXPIRED = { 'drive-token-v1': JSON.stringify({ token: 'expired', expiry: 1, grantedScopes: 'https://www.googleapis.com/auth/drive' }) };
const signIn = async t => { await t.click('#btn-welcome-start'); await t.advance(400); await t.click('#btn-welcome-login'); };
// Drive never answers the listing or the profile.
const held = mock => { fixtures.empty(mock); mock.inject({ host: 'www.googleapis.com', path: /\/drive\/v3\/(files|about)$/, times: Infinity, hold: true }); };

// The APK never loads Google Identity Services: the sheet's button hands the PKCE URL to the native bridge and
// Welcome stays as it is.
parity('auth/app-pkce-started', { only: { modes: ['app'] }, drive: fixtures.empty }, async t => {
  await t.open();
  await signIn(t);
  await t.advance(600);
});

// Web: the consent was granted a moment ago, Drive has not answered — Home's top bar with the profile button before
// the profile arrives, over the skeleton `refreshDriveFiles` shows (the skeleton itself is W4.1's on `sig::REFRESH`).
parity('auth/signed-in-topbar', { drive: held }, async t => {
  await t.open();
  await signIn(t);
  await t.advance(200);
});

// A remembered session whose account is not known yet (no `drive-hint-email`, no cached profile), Drive not
// answering: the skeleton the boot script painted is what both apps still show — default avatar, 26 skeleton cards.
parity('auth/returning-skeleton', { systemDark: true, drive: held }, async t => {
  const session = t.mock.session(ANA, { now: EPOCH });
  await t.open({ storage: { 'drive-token-v1': session['drive-token-v1'], 'drive-remember-v1': '1' }, settleMs: 400 });
});

// A returning session of a known account, Drive not answering: avatar and name from the cache. 5.18.0 shows its
// cached library at once (W4.1 + the offline library in v6).
parity('auth/returning', { systemDark: true, drive: held }, async t => {
  await t.open({ account: ANA, settleMs: 400 });
});

// The token expired and Google refuses to renew it silently: "Reconnect" beside the profile button.
parity('auth/reconnect', { drive: mock => { fixtures.empty(mock); mock.gis.mode = 'silent-fail'; } }, async t => {
  await t.open({ account: ANA, storage: EXPIRED, settleMs: 12_000 });
});

// The same, then the network goes away: the one visible thing the shell changes when offline — "Reconnect" is
// withdrawn (it could not work). 5.18.0 re-evaluates it when its offline library has reloaded; v6 does when the
// offline library unit calls `auth::update_ui` from the same place, so this state is that unit's gate too.
parity('auth/reconnect-offline', { drive: mock => { fixtures.empty(mock); mock.gis.mode = 'silent-fail'; } }, async t => {
  await t.open({ account: ANA, storage: EXPIRED, settleMs: 12_000 });
  t.mock.offline = true;
  await t.context.setOffline(true);
  await t.advance(1000);
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
