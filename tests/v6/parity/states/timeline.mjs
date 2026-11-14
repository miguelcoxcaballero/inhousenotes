// The Timeline panel (TESTING §3.3 row "timeline", BUILD-PLAN W5.3) on the `versions-30` fixture (30 entries six
// minutes apart, every third a milestone, plus the save entry): the grouped list with the current entry selected,
// an older entry's preview with its diff highlight, collapsed cluster and month, the `Original` entry and the restore
// slider. Both apps list the same imported 5.x history.
import { parity } from '../run.mjs';
import { ANA } from '../../mock/fixtures.mjs';

const doc = t => t.open({ account: ANA, scene: 'versions-30' });

async function openTimeline(t) {
  await t.click('#btn-edit-pages', { force: true });
  await t.advance(600);
  await t.click('#btn-open-timeline', { force: true });
  await t.settle();
  await t.advance(600);
}

parity('timeline/list', {}, async t => {
  await doc(t);
  await openTimeline(t);
});

parity('timeline/older', {}, async t => {
  await doc(t);
  await openTimeline(t);
  await t.click(':nth-match(.timeline-entry, 21)');
  await t.advance(400);
});

parity('timeline/collapsed', {}, async t => {
  await doc(t);
  await openTimeline(t);
  await t.click('.timeline-cluster-header');
  await t.advance(300);
  await t.capture('cluster');
  await t.click('.timeline-group-header');
  await t.advance(300);
  await t.capture('month');
});

parity('timeline/original', {}, async t => {
  await doc(t);
  await openTimeline(t);
  await t.click('.timeline-entry.is-original');
  await t.advance(400);
});

parity('timeline/restore-slider', {}, async t => {
  await doc(t);
  await openTimeline(t);
  await t.click(':nth-match(.timeline-entry, 21)');
  await t.advance(300);
  await t.click('#btn-revert-to-version');
  await t.advance(400);
});
