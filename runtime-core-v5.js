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

    // One host alarm for every paused document task. Rust owns eligibility,
    // deadlines, FIFO admission and session rejection; this bridge owns Promises
    // and AbortSignals. Older/offline WASM versions use the equivalent fallback.
    class DocumentWorkQueue {
        constructor(readState, options = {}) {
            this.readState = readState;
            this.makePlanner = options.makePlanner || (() => root.InhouseRust?.documentWorkQueue?.());
            this.setTimer = options.setTimer || ((fn, ms) => setTimeout(fn, ms));
            this.clearTimer = options.clearTimer || (id => clearTimeout(id));
            this.now = options.now || (() => performance.now());
            this.waiters = new Map(); this.plainSignals = new Set();
            this.planner = null; this.nextId = 0; this.timer = null; this.deadline = Infinity;
            this.wakes = 0; this.maxBatch = 0;
        }
        wait(options = {}) {
            const signal = options.abortSignal;
            if (signal?.aborted) return Promise.resolve(false);
            this.ensurePlanner();
            do { this.nextId = (this.nextId + 1) >>> 0; } while (!this.nextId || this.waiters.has(this.nextId));
            const id = this.nextId;
            const session = Number.isFinite(options.sessionToken) ? options.sessionToken : -1;
            const quietMs = Math.min(60000, Math.max(0, Number.isFinite(options.windowMs) ? options.windowMs : 1200));
            const empty = !this.waiters.size;
            const promise = new Promise(resolve => {
                const waiter = {id, session, quietMs, signal, resolve};
                this.waiters.set(id, waiter);
                if (this.planner && !this.planner.add(id, session, quietMs)) this.dropPlanner();
                if (signal?.addEventListener) {
                    waiter.onAbort = () => this.settle(id, false);
                    signal.addEventListener('abort', waiter.onAbort, {once:true});
                } else if (signal) this.plainSignals.add(waiter);
            });
            if (empty) this.pump();
            else this.wake();
            return promise;
        }
        ensurePlanner() {
            if (this.planner || this.waiters.size > 4096) return;
            this.planner = this.makePlanner() || null;
            if (!this.planner) return;
            for (const waiter of this.waiters.values()) {
                if (!this.planner.add(waiter.id, waiter.session, waiter.quietMs)) { this.dropPlanner(); break; }
            }
        }
        dropPlanner() { this.planner?.dispose(); this.planner = null; }
        settle(id, result) {
            const waiter = this.waiters.get(id);
            if (!waiter) return;
            this.waiters.delete(id); this.plainSignals.delete(waiter);
            this.planner?.cancel(id);
            if (waiter.onAbort) waiter.signal.removeEventListener('abort', waiter.onAbort);
            waiter.resolve(result);
            if (!this.waiters.size && this.timer !== null) {
                this.clearTimer(this.timer); this.timer = null; this.deadline = Infinity;
            }
        }
        fallback(state) {
            const ready = []; let delay = Infinity;
            const elapsed = Math.max(0, state.now - state.lastActivity);
            for (const waiter of this.waiters.values()) {
                const stale = waiter.session >= 0 && waiter.session !== state.session;
                const remaining = state.bypass ? 0 : Math.max(0, waiter.quietMs - elapsed);
                const busy = state.busy && !state.bypass;
                if ((stale || (!busy && !remaining)) && ready.length < 8) ready.push([waiter.id, !stale]);
                else delay = Math.min(delay, stale ? 1 : busy ? 100 : Math.max(1, remaining));
            }
            return {ready, delay:Number.isFinite(delay) ? delay : -1};
        }
        pump() {
            if (this.timer !== null) this.clearTimer(this.timer);
            this.timer = null; this.deadline = Infinity;
            if (!this.waiters.size) return;
            this.wakes++;
            this.ensurePlanner();
            for (const waiter of this.plainSignals) if (waiter.signal.aborted) this.settle(waiter.id, false);
            const state = this.readState();
            let batch;
            try { batch = this.planner?.poll(state); }
            catch (_) { this.dropPlanner(); }
            batch ||= this.fallback(state);
            this.maxBatch = Math.max(this.maxBatch, batch.ready.length);
            for (const [id, valid] of batch.ready) this.settle(id, valid);
            if (this.waiters.size) this.schedule(Math.min(batch.delay < 0 ? 1 : batch.delay,
                this.plainSignals.size ? 100 : Infinity));
        }
        schedule(delay) {
            const deadline = this.now() + delay;
            if (this.timer !== null && this.deadline <= deadline) return;
            if (this.timer !== null) this.clearTimer(this.timer);
            this.deadline = deadline;
            this.timer = this.setTimer(() => { this.timer = null; this.deadline = Infinity; this.pump(); }, delay);
        }
        wake() { if (this.waiters.size) this.schedule(0); }
        dispose() {
            for (const id of this.waiters.keys()) this.settle(id, false);
            this.dropPlanner();
        }
        get diagnostics() { return {backend:this.planner ? 'rust' : 'javascript', pending:this.waiters.size, wakes:this.wakes, maxBatch:this.maxBatch}; }
    }

    const api = Object.freeze({ fetchWithDeadline, settleWithTimeout, runWithRetries, CoalescingQueue, WorkerRpcClient, DocumentWorkQueue });
    root.InhouseRuntimeCore = api;
    if (typeof module !== 'undefined' && module.exports) module.exports = api;
})(typeof globalThis !== 'undefined' ? globalThis : window);
