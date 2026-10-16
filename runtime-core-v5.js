(function attachInhouseRuntimeCore(root) {
    'use strict';

    async function fetchWithDeadline(url, options = {}, timeoutMs = 30000, consumeResponse = null) {
        const controller = new AbortController();
        const sourceSignal = options.signal || null;
        if (sourceSignal?.aborted) throw abortError(sourceSignal);
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

    function abortError(signal) {
        return signal?.reason || new DOMException('Aborted', 'AbortError');
    }

    function delay(ms, signal) {
        return new Promise((resolve, reject) => {
            if (signal?.aborted) { reject(abortError(signal)); return; }
            const finish = error => {
                clearTimeout(timer);
                signal?.removeEventListener('abort', abort);
                if (error) reject(error); else resolve();
            };
            const abort = () => finish(abortError(signal));
            const timer = setTimeout(() => finish(), ms);
            signal?.addEventListener('abort', abort, { once: true });
        });
    }

    async function runWithRetries(operation, options = {}) {
        if (typeof operation !== 'function') throw new TypeError('Retry operation must be a function');
        const attempts = Number.isFinite(options.attempts) ? Math.max(1, Math.min(8, Math.floor(options.attempts))) : 3;
        const baseDelayMs = Number.isFinite(options.baseDelayMs) ? Math.max(0, Math.min(5000, options.baseDelayMs)) : 80;
        for (let attempt = 0; attempt < attempts; attempt++) {
            if (options.signal?.aborted) throw abortError(options.signal);
            try { return await operation(attempt); }
            catch (error) {
                if (options.signal?.aborted) throw abortError(options.signal);
                if (error?.name === 'AbortError' || attempt + 1 === attempts
                    || options.shouldRetry?.(error, attempt) === false) throw error;
                await delay(Math.min(5000, baseDelayMs * 2 ** attempt), options.signal);
            }
        }
    }

    // A running snapshot is immutable. Requests arriving during it belong to
    // the next batch, and their promises cannot resolve with the older save.
    class CoalescingQueue {
        constructor(execute, merge = (_, next) => next) {
            this.execute = execute;
            this.merge = merge;
            this.pending = new Map();
            this.running = false;
        }
        request(key, input) {
            const promise = new Promise((resolve, reject) => {
                let batch = this.pending.get(key);
                if (batch) batch.input = this.merge(batch.input, input);
                else { batch = { input, waiters: [] }; this.pending.set(key, batch); }
                batch.waiters.push({ resolve, reject });
            });
            if (!this.running) void this.drain();
            return promise;
        }
        async drain() {
            this.running = true;
            try {
                while (this.pending.size) {
                    const key = this.pending.keys().next().value;
                    const batch = this.pending.get(key);
                    this.pending.delete(key);
                    try {
                        const result = await this.execute(batch.input, key);
                        for (const waiter of batch.waiters) waiter.resolve(result);
                    } catch (error) {
                        for (const waiter of batch.waiters) waiter.reject(error);
                    }
                }
            } finally { this.running = false; }
        }
    }

    class WorkerRpcClient {
        constructor(createWorker, { timeoutMs = 90000, maxPending = 64 } = {}) {
            this.createWorker = createWorker;
            this.timeoutMs = timeoutMs;
            this.maxPending = maxPending;
            this.worker = null;
            this.active = null;
            this.queue = [];
            this.epoch = 0;
            this.sequence = 0;
        }
        assertEpoch(epoch) {
            if (epoch !== this.epoch) throw Object.assign(new Error('Document processor session expired'), { name: 'InvalidStateError' });
        }
        run(type, data, options = {}) {
            if (options.signal?.aborted) return Promise.reject(abortError(options.signal));
            if (this.queue.length >= this.maxPending) return Promise.reject(new Error('Document processor queue is full'));
            return new Promise((resolve, reject) => {
                const request = { id: ++this.sequence, type, data, options, resolve, reject };
                request.abort = () => {
                    const error = abortError(options.signal);
                    if (this.active === request) this.reset(error);
                    else {
                        this.queue = this.queue.filter(item => item !== request);
                        this.settle(request, error);
                    }
                };
                options.signal?.addEventListener('abort', request.abort, { once: true });
                this.queue.push(request);
                this.pump();
            });
        }
        settle(request, error, value) {
            clearTimeout(request.timer);
            request.options.signal?.removeEventListener('abort', request.abort);
            if (error) request.reject(error); else request.resolve(value);
        }
        reset(error = new Error('Document processor stopped')) {
            this.worker?.terminate();
            this.worker = null;
            this.epoch++;
            const requests = this.active ? [this.active, ...this.queue] : this.queue;
            this.active = null;
            this.queue = [];
            for (const request of requests) this.settle(request, error);
        }
        pump() {
            if (this.active || !this.queue.length) return;
            const request = this.active = this.queue.shift();
            try {
                if (!this.worker) {
                    const worker = this.worker = this.createWorker();
                    worker.onmessage = ({ data }) => {
                        if (this.worker !== worker || data?.id !== this.active?.id) return;
                        const current = this.active;
                        this.active = null;
                        const error = data.error ? Object.assign(new Error(data.error), { name: data.errorName || 'Error' }) : null;
                        this.settle(current, error, data.value);
                        this.pump();
                    };
                    worker.onerror = event => { if (this.worker === worker) this.reset(new Error(event.message || 'Document worker failed')); };
                    worker.onmessageerror = () => { if (this.worker === worker) this.reset(new Error('Document worker response unreadable')); };
                }
                request.timer = setTimeout(() => this.reset(Object.assign(new Error('Document processing timed out'), { name: 'TimeoutError' })), request.options.timeoutMs ?? this.timeoutMs);
                this.worker.postMessage({ id: request.id, type: request.type, data: request.data }, request.options.transfer || []);
            } catch (error) {
                this.active = null;
                this.settle(request, error);
                this.pump();
            }
        }
    }

    const api = Object.freeze({ fetchWithDeadline, settleWithTimeout, runWithRetries, CoalescingQueue, WorkerRpcClient });
    root.InhouseRuntimeCore = api;
    if (typeof module !== 'undefined' && module.exports) module.exports = api;
})(typeof globalThis !== 'undefined' ? globalThis : window);
