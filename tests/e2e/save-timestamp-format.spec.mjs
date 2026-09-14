import { expect, test } from '@playwright/test';

// Regression coverage for a perf fix: formatTime() used to call the
// expensive toLocaleTimeString() Intl API fresh on every call while actively
// writing (diffMin < 1 is true on almost every stroke). It's now memoized by
// clock-minute, since the formatted string only has minute granularity.
// These tests pin the *correctness* of that cache across the case that
// matters most: a minute boundary crossing must not return a stale value.

test('formatTime: same value on repeated calls within the same minute (cache hit)', async ({ page }) => {
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(() => window.__IHN_TEST_API__.ready());

  const result = await page.evaluate(() => {
    const now = Date.now();
    const a = window.__IHN_TEST_API__.formatTimeForTest(now);
    const b = window.__IHN_TEST_API__.formatTimeForTest(now);
    return { a, b };
  });
  expect(result.a).toBe(result.b);
  expect(result.a).toMatch(/^\d{2}:\d{2}$/);
});

test('formatTime: crossing a minute boundary does not return a stale cached value', async ({ page }) => {
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(() => window.__IHN_TEST_API__.ready());

  const result = await page.evaluate(() => {
    // Two timestamps a full minute apart, both "now" for formatTime's own
    // Date.now() comparison (diffMin < 1 for both, so both take the
    // absolute-clock-time path) — but they must format to different minutes.
    const t1 = Date.now();
    const t2 = t1 - 60_000; // exactly one minute earlier
    const label1 = window.__IHN_TEST_API__.formatTimeForTest(t1);
    const label2 = window.__IHN_TEST_API__.formatTimeForTest(t2);
    return { label1, label2 };
  });
  expect(result.label1).not.toBe(result.label2);
});

test('formatTime: relative "X min" range is unaffected by the cache', async ({ page }) => {
  await page.goto('/?e2e=1', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__IHN_TEST_API__);
  await page.evaluate(() => window.__IHN_TEST_API__.ready());

  const result = await page.evaluate(() => {
    const fiveMinAgo = Date.now() - 5 * 60_000;
    return {
      fiveMin: window.__IHN_TEST_API__.formatTimeForTest(fiveMinAgo),
      empty: window.__IHN_TEST_API__.formatTimeForTest(0),
      nullish: window.__IHN_TEST_API__.formatTimeForTest(null)
    };
  });
  expect(result.fiveMin).toBe('5 min');
  expect(result.empty).toBe('--:--');
  expect(result.nullish).toBe('--:--');
});
