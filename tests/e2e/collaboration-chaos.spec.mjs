import { expect, test } from '@playwright/test';

async function openReplica(context, actor, room) {
  const page = await context.newPage();
  await page.goto('/tests/e2e/fixtures/collaboration-chaos.html');
  await page.waitForFunction(() => window.__CHAOS__);
  await page.evaluate(({ actor: nextActor, room: nextRoom }) => {
    window.__CHAOS__.init(nextActor, nextRoom);
  }, { actor, room });
  return page;
}

async function snapshots(pages) {
  return Promise.all(pages.map(page => page.evaluate(() => window.__CHAOS__.snapshot())));
}

async function expectConvergence(pages) {
  await expect.poll(async () => {
    const values = await snapshots(pages);
    return new Set(values.map(value => value.hash)).size;
  }, { timeout: 5000, intervals: [20, 40, 80, 160] }).toBe(1);
}

test('three concurrent replicas converge for strokes, fields, page moves and deletes', async ({ context }) => {
  const room = `concurrent-${Date.now()}`;
  const pages = await Promise.all([
    openReplica(context, 'account-a:phone', room),
    openReplica(context, 'account-b:tablet', room),
    openReplica(context, 'account-c:web', room)
  ]);

  await Promise.all([
    pages[0].evaluate(() => {
      window.__CHAOS__.addStroke('page-a', 'stroke-a');
      window.__CHAOS__.movePage('page-c', 0);
    }),
    pages[1].evaluate(() => {
      window.__CHAOS__.addStroke('page-a', 'stroke-b');
      window.__CHAOS__.deletePage('page-b');
    }),
    pages[2].evaluate(() => window.__CHAOS__.rename('Shared title'))
  ]);

  await expectConvergence(pages);
  const [snapshot] = await snapshots(pages);
  expect(snapshot.order).toEqual(['page-c', 'page-a']);
  expect(snapshot.pages.flatMap(page => page.strokes.map(stroke => stroke.id)).sort())
    .toEqual(['stroke-a', 'stroke-b']);
  expect(snapshot.exportName).toBe('Shared title');
});

test('network changes reconnect immediately and a slow stale fallback cannot revert state', async ({ context }) => {
  const room = `network-${Date.now()}`;
  const pages = await Promise.all([
    openReplica(context, 'account-a:wifi', room),
    openReplica(context, 'account-b:data', room)
  ]);
  await expectConvergence(pages);
  await pages[0].evaluate(() => window.__CHAOS__.scheduleStaleFallback(900));
  await Promise.all(pages.map(page => page.evaluate(() => window.__CHAOS__.setOnline(false))));
  await Promise.all([
    pages[0].evaluate(() => {
      window.__CHAOS__.addStroke('page-a', 'offline-a');
      window.__CHAOS__.deletePage('page-c');
    }),
    pages[1].evaluate(() => {
      window.__CHAOS__.addStroke('page-a', 'offline-b');
      window.__CHAOS__.rename('Newest title');
    })
  ]);

  const reconnectStarted = Date.now();
  await Promise.all(pages.map(page => page.evaluate(() => window.__CHAOS__.setOnline(true))));
  await expectConvergence(pages);
  expect(Date.now() - reconnectStarted).toBeLessThan(1000);

  await pages[0].waitForTimeout(1100);
  await expectConvergence(pages);
  const values = await snapshots(pages);
  for (const value of values) {
    expect(value.order).not.toContain('page-c');
    expect(value.exportName).toBe('Newest title');
    expect(value.pages.flatMap(page => page.strokes.map(stroke => stroke.id)).sort())
      .toEqual(['offline-a', 'offline-b']);
    expect(value.received).toBeLessThan(30);
  }
});
