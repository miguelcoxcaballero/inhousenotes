// Welcome (signed out): TESTING §3.3 row "welcome".
import { parity } from '../run.mjs';

// The document as it is when loading finishes, before any timer has run (boot skeleton off: nobody is signed in).
parity('welcome/first-paint', { systemDark: true }, async t => {
  await t.open({ settleMs: 0, ready: false });
});

parity('welcome/welcome', { systemDark: true, app: true }, async t => {
  await t.open();
});

// "Continue with Google" opens the consent sheet; captured while it slides in and when it rests.
parity('welcome/sheet', { systemDark: true, app: true }, async t => {
  await t.open();
  await t.click('#btn-welcome-start');
  await t.advance(100);
  await t.capture('opening-100ms');
  await t.advance(900);
  await t.capture('open');
});
