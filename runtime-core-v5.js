(function attachInhouseRuntimeCore(root) {
    'use strict';

    async function fetchWithDeadline(url, options = {}, timeoutMs = 30000, consumeResponse = null) {
        const controller = new AbortController();
        const sourceSignal = options.signal || null;
        const safeTimeoutMs = Number.isFinite(timeoutMs) ? Math.max(1000, timeoutMs) : 30000;
        let timedOut = false;
        const timeoutId = setTimeout(() => {
            timedOut = true;
            controller.abort();
        }, safeTimeoutMs);
        let sourceAbortHandler = null;
        if (sourceSignal) {
            if (sourceSignal.aborted) controller.abort();
            else {
                sourceAbortHandler = () => controller.abort();
                sourceSignal.addEventListener('abort', sourceAbortHandler, { once: true });
            }
        }
        try {
            const response = await fetch(url, { ...options, signal: controller.signal });
            return typeof consumeResponse === 'function'
                ? await consumeResponse(response)
                : response;
        } catch (error) {
            if (error?.name === 'AbortError' && timedOut) {
                const timeoutError = new Error('Network request timed out');
                timeoutError.name = 'TimeoutError';
                throw timeoutError;
            }
            throw error;
        } finally {
            clearTimeout(timeoutId);
            if (sourceSignal && sourceAbortHandler) {
                sourceSignal.removeEventListener('abort', sourceAbortHandler);
            }
        }
    }

    async function settleWithTimeout(promise, timeoutMs, fallback = null) {
        const safeTimeoutMs = Number.isFinite(timeoutMs) ? Math.max(0, timeoutMs) : 0;
        let timeoutId = null;
        try {
            return await Promise.race([
                Promise.resolve(promise),
                new Promise(resolve => {
                    timeoutId = setTimeout(() => resolve(fallback), safeTimeoutMs);
                })
            ]);
        } catch (error) {
            return fallback;
        } finally {
            if (timeoutId !== null) clearTimeout(timeoutId);
        }
    }

    async function runWithRetries(operation, options = {}) {
        if (typeof operation !== 'function') throw new TypeError('Retry operation must be a function');
        const attempts = Math.max(1, Math.min(8, Number(options.attempts) || 3));
        const baseDelayMs = Math.max(0, Math.min(5000, Number(options.baseDelayMs) || 80));
        let lastError;
        for (let attempt = 0; attempt < attempts; attempt += 1) {
            try {
                return await operation(attempt);
            } catch (error) {
                lastError = error;
                if (attempt + 1 >= attempts || options.signal?.aborted) break;
                const delay = Math.min(5000, baseDelayMs * (2 ** attempt));
                await new Promise((resolve, reject) => {
                    const timeoutId = setTimeout(resolve, delay);
                    if (!options.signal) return;
                    options.signal.addEventListener('abort', () => {
                        clearTimeout(timeoutId);
                        reject(options.signal.reason || new DOMException('Aborted', 'AbortError'));
                    }, { once: true });
                });
            }
        }
        throw lastError;
    }

    const api = Object.freeze({ fetchWithDeadline, settleWithTimeout, runWithRetries });
    root.InhouseRuntimeCore = api;
    if (typeof module !== 'undefined' && module.exports) module.exports = api;
})(typeof globalThis !== 'undefined' ? globalThis : window);
