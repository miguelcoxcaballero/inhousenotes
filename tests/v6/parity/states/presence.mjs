// Presence (TESTING §3.3 row "presence"), owned by W4.5: the avatar row with another editor present while the
// handshake runs (`p2p-connecting`), a relay-only device (B-Y5: v6 shows a present device without a P2P class while
// it is not in a handshake), a remote save ring, and the `Actividad del documento` popup.
//
// The remote device is seeded per side: 5.18.0 reads `p_<key>_q` / `p_<key>_r` presence properties; v6 reads the
// `ihn6_<inst>` record and the relay file it seals (SYNC §8.3–§8.4), written here into Ben's link-shared sync
// folder. `p2p-connected` needs a second browser on both sides and is covered by e2e 11 (a).
import crypto from 'node:crypto';
import { parity } from '../run.mjs';
import { EPOCH } from '../matrix.mjs';
import { ANA, BEN, addAccounts, addScene } from '../../mock/fixtures.mjs';
import { FOLDER } from '../../mock/drive.mjs';

const SCENE = 'template', DOC = `scene-${SCENE}`;

// ---- v6 relay file + record (SYNC §8.3, §8.4) ----
const leb = n => { const out = []; let v = BigInt(n); do { let b = Number(v & 0x7fn); v >>= 7n; if (v) b |= 0x80; out.push(b); } while (v); return Buffer.from(out); };
const str = s => { const b = Buffer.from(s, 'utf8'); return Buffer.concat([leb(b.length), b]); };
const u64 = v => { const b = Buffer.alloc(8); b.writeBigUInt64LE(BigInt(v)); return b; };
const b36 = v => BigInt(v).toString(36);
const inst13 = v => b36(v).padStart(13, '0');

/** Ben's device `inst` with save phase `phase` (ARCHITECTURE §6.6 code): the record key/value and the file. */
function relayDevice(inst, phase = 6) {
  const fileId = `relayBen${inst.toString(16)}`;
  const payload = Buffer.concat([
    str(DOC), u64(inst), leb(1), leb(EPOCH),
    str(BEN), str('Ben Writer'), str(''), Buffer.from('p'), str('Linux PC'), Buffer.from([phase]),
    leb(0), Buffer.from([0]), leb(0), leb(0), leb(0), leb(0)
  ]);
  const file = Buffer.concat([Buffer.from('IHN6R'), Buffer.from([1, 0]), payload]);
  const d16 = crypto.createHash('sha256').update(file).digest().subarray(0, 12).toString('base64url');
  return { key: `ihn6_${inst13(inst)}`, value: `1|1|${b36(EPOCH)}|${fileId}|${d16}|c`, fileId, file };
}

/** 5.18.0 packed presence of Ben's device (`packPresenceIdentity` / `packPresenceRuntime`). */
function v5Device(phase = '') {
  // The phase is stamped a few seconds into the run, so 5.18.0 still shows it (6.5 s window) at capture time.
  return { p_benpc_q: `0|cbenpc|${BEN}|Ben Writer|p|Linux PC`, p_benpc_r: `${b36(EPOCH + 4000)}|${phase}|${phase ? b36(EPOCH + 4000) : ''}` };
}

/** B-Y5: when a device counts as present (and so when its avatar, and the start of its CSS animations, is drawn) is
 * measured on the receiver's clock; B-Y6: the main device is the scribe. Allowed inside the avatars (padded for the
 * badges and the glow) and the popup. */
const around = (selectors, pad = 24) => async t => {
  const boxes = (await Promise.all(selectors.map(s => t.page.locator(s).boundingBox()))).filter(Boolean);
  if (!boxes.length) return [];
  const x = Math.min(...boxes.map(b => b.x)) - pad, y = Math.min(...boxes.map(b => b.y)) - pad;
  return [{ x, y, width: Math.max(...boxes.map(b => b.x + b.width)) + pad - x, height: Math.max(...boxes.map(b => b.y + b.height)) + pad - y }];
};
const avatarsRule = { tags: ['B-Y5'], expected: around(['#presence-avatars']) };

/** Seeds the scene shared with Ben and his device present, the way each side reads presence. */
function remote({ inst = 0xfff0_0000_0000_0100n, phase = 6, v5phase = '' } = {}) {
  return (mock, t) => {
    addAccounts(mock);
    const file = addScene(mock, SCENE, { permissions: [{ type: 'user', role: 'writer', emailAddress: BEN }] });
    if (t.side !== 'b') return Object.assign(file.properties, v5Device(v5phase));
    const folder = mock.drive.addFile({ id: 'benSyncFolder', name: 'Inhouse Notes sync', mimeType: FOLDER, owner: BEN, appProperties: { ihn6Kind: 'sync-folder' }, permissions: [{ type: 'anyone', role: 'reader' }] });
    const d = relayDevice(inst, phase);
    mock.drive.addFile({ id: d.fileId, name: `ihn6-${DOC}-${inst13(inst)}.bin`, mimeType: 'application/octet-stream', owner: BEN, parents: [folder.id], data: d.file, appProperties: { ihn6Kind: 'relay', ihn6Doc: DOC, ihn6Inst: inst13(inst) } });
    file.properties[d.key] = d.value;
  };
}

async function openWithRemote(t) {
  await t.open({ account: ANA, scene: SCENE });
  await t.advance(2500);
}

// Another editor present, the handshake under way: `p2p-connecting` on both (v6: our installation id is the
// smaller one, so we offer at once).
parity('presence/connecting', { drive: remote(), ...avatarsRule }, async t => {
  await openWithRemote(t);
});

// A device present through the relay only (v6: the other installation offers first; we wait 8 s): no P2P class.
parity('presence/relay-only', { drive: remote({ inst: 0x100n }), ...avatarsRule }, async t => {
  await openWithRemote(t);
});

// A remote save ring while the other device uploads.
parity('presence/save-ring', { drive: remote({ phase: 3, v5phase: 'u' }), ...avatarsRule }, async t => {
  await openWithRemote(t);
});

// The activity popup: this device first, then the other editor's device.
parity('presence/popup', {
  drive: remote(),
  tags: ['B-Y5', 'B-Y6'],
  expected: around(['#presence-avatars', '#presence-popup'])
}, async t => {
  await openWithRemote(t);
  await t.click('#presence-avatars');
  await t.advance(300);
});
