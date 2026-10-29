import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import test from 'node:test';

const require = createRequire(import.meta.url);
const { fetchWithDeadline, settleWithTimeout } = require('../runtime-core-v5.js');

test('fetchWithDeadline aborts a stalled request and identifies a timeout', async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = (_url, options) => new Promise((_resolve, reject) => {
    options.signal.addEventListener('abort', () => {
      const error = new Error('aborted');
      error.name = 'AbortError';
      reject(error);
    }, { once: true });
  });
  try {
    await assert.rejects(fetchWithDeadline('/slow', {}, 1000), error => error.name === 'TimeoutError');
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test('fetchWithDeadline forwards caller cancellation without relabeling it', async () => {
  const originalFetch = globalThis.fetch;
  const controller = new AbortController();
  globalThis.fetch = (_url, options) => new Promise((_resolve, reject) => {
    options.signal.addEventListener('abort', () => {
      const error = new Error('aborted');
      error.name = 'AbortError';
      reject(error);
    }, { once: true });
  });
  try {
    const pending = fetchWithDeadline('/cancelled', { signal: controller.signal }, 5000);
    controller.abort();
    await assert.rejects(pending, error => error.name === 'AbortError');
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test('settleWithTimeout returns the fallback for timeout and rejection', async () => {
  assert.equal(await settleWithTimeout(new Promise(() => {}), 5, 'late'), 'late');
  assert.equal(await settleWithTimeout(Promise.reject(new Error('failed')), 50, 'failed-safe'), 'failed-safe');
});
