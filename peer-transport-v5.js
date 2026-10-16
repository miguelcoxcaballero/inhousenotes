/* Version-one wire framing, independent of signalling, editor state and merging. */
(function (root) {
    'use strict';
    const CHUNK = 16000;
    const MAX_CHARS = 50000000;

    function waitForCapacity(channel, { highWater = 64000, timeoutMs = 15000, signal } = {}) {
        return new Promise((resolve, reject) => {
            let timer;
            const finish = error => {
                clearTimeout(timer);
                channel.removeEventListener('bufferedamountlow', drained);
                channel.removeEventListener('close', closed);
                channel.removeEventListener('error', closed);
                signal?.removeEventListener('abort', aborted);
                if (error) reject(error); else resolve();
            };
            const closed = () => finish(new Error('Peer channel closed'));
            const aborted = () => finish(signal.reason || new DOMException('Aborted', 'AbortError'));
            const drained = () => {
                if (channel.readyState !== 'open') closed();
                else if (channel.bufferedAmount < highWater) finish();
            };
            if (signal?.aborted) { aborted(); return; }
            channel.bufferedAmountLowThreshold = Math.floor(highWater / 2);
            channel.addEventListener('bufferedamountlow', drained);
            channel.addEventListener('close', closed, { once: true });
            channel.addEventListener('error', closed, { once: true });
            signal?.addEventListener('abort', aborted, { once: true });
            timer = setTimeout(() => finish(new Error('Peer channel remained congested')), timeoutMs);
            drained();
        });
    }

    async function send(channel, payload, { assertCurrent = () => {}, signal, maxChars = MAX_CHARS } = {}) {
        assertCurrent();
        if (channel?.readyState !== 'open') return false;
        const raw = JSON.stringify(payload);
        if (!raw || raw.length > maxChars) return false;
        const id = root.crypto.randomUUID();
        const total = Math.ceil(raw.length / CHUNK);
        const write = async message => {
            await waitForCapacity(channel, { signal });
            assertCurrent();
            channel.send(JSON.stringify(message));
        };
        await write({ t: 'start', id, total, chars: raw.length });
        for (let index = 0; index < total; index++) {
            await write({ t: 'chunk', id, index, data: raw.slice(index * CHUNK, (index + 1) * CHUNK) });
        }
        return true;
    }

    class Receiver {
        constructor({ maxChars = MAX_CHARS, maxAssemblies = 16, maxPerPeer = 4, ttlMs = 60000, now = Date.now } = {}) {
            Object.assign(this, { maxChars, maxAssemblies, maxPerPeer, ttlMs, now });
            this.assemblies = new Map();
            this.reserved = 0;
            this.timer = null;
        }
        remove(key) {
            const assembly = this.assemblies.get(key);
            if (!assembly) return;
            this.reserved -= assembly.chars;
            this.assemblies.delete(key);
            if (!this.assemblies.size) { clearTimeout(this.timer); this.timer = null; }
        }
        prune() {
            for (const [key, assembly] of this.assemblies) if (assembly.expires <= this.now()) this.remove(key);
        }
        arm() {
            if (this.timer || !this.assemblies.size) return;
            this.timer = setTimeout(() => { this.timer = null; this.prune(); this.arm(); }, this.ttlMs);
            this.timer.unref?.();
        }
        clear() { for (const key of this.assemblies.keys()) this.remove(key); }
        dropPeer(peer) { for (const [key, value] of this.assemblies) if (value.peer === peer) this.remove(key); }
        accept(peer, message) {
            this.prune();
            if (!message || typeof message.id !== 'string' || !message.id || message.id.length > 128) return null;
            const key = JSON.stringify([peer, message.id]);
            if (message.t === 'start') {
                const { total, chars } = message;
                if (!Number.isSafeInteger(total) || !Number.isSafeInteger(chars)
                    || total < 1 || total > 5000 || chars < total || chars > total * CHUNK || chars > this.maxChars) return null;
                // Repeated starts never reset a partially received transfer.
                if (this.assemblies.has(key)) return null;
                const peerCount = [...this.assemblies.values()].filter(value => value.peer === peer).length;
                if (this.assemblies.size >= this.maxAssemblies || peerCount >= this.maxPerPeer || this.reserved + chars > this.maxChars) return null;
                this.assemblies.set(key, { peer, chars, total, parts: new Map(), size: 0, expires: this.now() + this.ttlMs });
                this.reserved += chars;
                this.arm();
                return null;
            }
            if (message.t !== 'chunk') return null;
            const assembly = this.assemblies.get(key);
            if (!assembly) return null;
            const { index, data } = message;
            if (!Number.isSafeInteger(index) || index < 0 || index >= assembly.total || typeof data !== 'string' || !data.length || data.length > CHUNK) {
                this.remove(key); return null;
            }
            if (assembly.parts.has(index)) {
                if (assembly.parts.get(index) !== data) this.remove(key);
                return null;
            }
            assembly.size += data.length;
            if (assembly.size > assembly.chars) { this.remove(key); return null; }
            assembly.parts.set(index, data);
            if (assembly.parts.size !== assembly.total) return null;
            this.remove(key);
            if (assembly.size !== assembly.chars) return null;
            const raw = Array.from({ length: assembly.total }, (_, i) => assembly.parts.get(i)).join('');
            try { return JSON.parse(raw); } catch (_) { return null; }
        }
    }
    const api = Object.freeze({ send, waitForCapacity, Receiver });
    root.InhousePeerTransport = api;
    if (typeof module !== 'undefined' && module.exports) module.exports = api;
})(globalThis);
