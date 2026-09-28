import { expect, test } from '@playwright/test';

test('real WebRTC connects new devices without edits, including a third account', async ({ browser }) => {
  const contexts = [];
  const comments = new Map();
  const properties = { ihn_live_key_v1: Buffer.alloc(32, 73).toString('base64url') };
  const appProperties = {};
  let nextId = 0;
  async function replica(email) {
    const context = await browser.newContext();
    contexts.push(context);
    await context.route('https://www.googleapis.com/**', async route => {
      // Exercise non-zero signalling latency instead of instant mocked Drive.
      await new Promise(resolve => setTimeout(resolve, 100));
      const request = route.request();
      const url = new URL(request.url());
      const method = request.method();
      const body = request.postDataJSON() || {};
      let value = {};
      const match = url.pathname.match(/\/comments(?:\/([^/]+))?(?:\/replies(?:\/([^/]+))?)?/);
      if (match) {
        const id = match[1];
        if (url.pathname.includes('/replies') && method === 'POST') {
          const reply = { id: `r${++nextId}`, content: body.content, createdTime: new Date().toISOString() };
          comments.get(id)?.replies.push(reply);
          value = reply;
        } else if (method === 'POST') {
          value = { id: `c${++nextId}`, content: body.content, replies: [], createdTime: new Date().toISOString() };
          comments.set(value.id, value);
        } else if (method === 'PATCH') {
          value = comments.get(id) || { id, replies: [] };
          Object.assign(value, body); comments.set(id, value);
        } else if (method === 'DELETE') comments.delete(id);
        else value = id ? comments.get(id) || {} : { comments: [...comments.values()] };
      } else {
        if (method === 'PATCH') { Object.assign(properties, body.properties); Object.assign(appProperties, body.appProperties); }
        value = { id: 'e2e-live-handshake', properties, appProperties, capabilities: { canEdit: true, canModifyContent: true } };
      }
      await route.fulfill({ status: 200, headers: { 'Access-Control-Allow-Origin': '*', 'Content-Type': 'application/json' }, body: JSON.stringify(value) });
    });
    const page = await context.newPage();
    await page.goto('http://127.0.0.1:4173/?e2e=1');
    await page.waitForFunction(() => window.__IHN_TEST_API__);
    await page.evaluate(async email => {
      await window.__IHN_TEST_API__.resetLocalDocument(1, 'Connection test');
      await window.__IHN_TEST_API__.startLiveConnectionForTest(email);
    }, email);
    return page;
  }
  try {
    const first = await replica('first@example.com');
    const second = await replica('second@example.com');
    await expect.poll(() => first.evaluate(() => window.__IHN_TEST_API__.liveConnectionOverviewForTest().openPeerCount), { timeout: 8000, intervals: [50] }).toBe(1);
    await expect.poll(() => second.evaluate(() => window.__IHN_TEST_API__.liveConnectionOverviewForTest().openPeerCount), { timeout: 8000, intervals: [50] }).toBe(1);
    for (const page of [first, second]) {
      await expect.poll(() => page.evaluate(() => window.__IHN_TEST_API__.liveConnectionOverviewForTest().peers[0]?.interactive), { timeout: 4000, intervals: [25] }).toBe('open');
    }
    const ping = await first.evaluate(() => window.__IHN_TEST_API__.congestBulkAndPingForTest());
    await expect.poll(() => first.evaluate(() => window.__IHN_TEST_API__.liveConnectionOverviewForTest().peers[0]?.lastPongEchoAt), { timeout: 1000, intervals: [25] }).toBeGreaterThanOrEqual(ping);
    const pageId = await second.evaluate(() => window.__IHN_TEST_API__.snapshot().pages[0].pageId);
    const strokeStart = Date.now();
    await first.evaluate(pageId => window.__IHN_TEST_API__.publishPreviewForTest('under-load', pageId, 2), pageId);
    await expect.poll(() => second.evaluate(() => window.__IHN_TEST_API__.previewPointCountForTest('under-load')), { timeout: 700, intervals: [15] }).toBe(2);
    await first.evaluate(pageId => window.__IHN_TEST_API__.publishPreviewForTest('under-load', pageId, 4), pageId);
    await expect.poll(() => second.evaluate(() => window.__IHN_TEST_API__.previewPointCountForTest('under-load')), { timeout: 700, intervals: [15] }).toBe(4);
    console.log('Two live preview batches with delayed bulk queue (ms):', Date.now() - strokeStart);
    const third = await replica('third@example.com');
    for (const page of [first, second, third]) {
      await expect.poll(() => page.evaluate(() => window.__IHN_TEST_API__.liveConnectionOverviewForTest().openPeerCount), { timeout: 8000, intervals: [50] }).toBe(2);
    }
  } catch (error) {
    for (const context of contexts) for (const page of context.pages()) {
      console.log('Handshake diagnostics:', await page.evaluate(() => window.__IHN_TEST_API__?.liveConnectionOverviewForTest()).catch(() => null));
    }
    throw error;
  } finally { await Promise.all(contexts.map(context => context.close())); }
});
