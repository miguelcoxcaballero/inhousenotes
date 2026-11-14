// IndexedDB transaction executor (family 3.2 = 0x20..0x2f), Web Locks (3.9 = 0x80..0x8f) and the read-only dumper
// of the 5.x databases. Wire formats: rust/crates/ihn-app/src/store/{mod.rs (ops), tx.rs (operations), sv.rs
// (keys and values)}. This file makes no decisions: Rust says what to open, read, write, lock and for how long to
// wait; results go back with host.io(req, status, 0, bytes).
const FX = 3 << 8;
const ST = { OK: 0, ERROR: -1, BLOCKED: -2, TIMEOUT: -3, UNAVAILABLE: -4, CLOSED: -5, QUOTA: -6, CONSTRAINT: -7, MISSING: 1 };
const ERRORS = { QuotaExceededError: ST.QUOTA, ConstraintError: ST.CONSTRAINT, InvalidStateError: ST.CLOSED };
// Tag 11 kinds, in the order of `sv::TYPED` (0 = ArrayBuffer, 11 = DataView).
const VIEWS = [Int8Array, Uint8ClampedArray, Int16Array, Uint16Array, Int32Array, Uint32Array, Float32Array, Float64Array, BigInt64Array, BigUint64Array, DataView];
const encoder = new TextEncoder();

// Structured value → bytes. Blobs stay in JS: Rust gets a handle (`held` lists them, to drop on failure).
class Enc {
  constructor(host) { this.host = host; this.b = new Uint8Array(1024); this.v = new DataView(this.b.buffer); this.n = 0; this.held = []; }
  need(k) {
    if (this.n + k <= this.b.length) return;
    const b = new Uint8Array(Math.max(this.b.length * 2, this.n + k));
    b.set(this.b.subarray(0, this.n));
    this.b = b;
    this.v = new DataView(b.buffer);
  }
  u8(x) { this.need(1); this.b[this.n++] = x; return this; }
  u32(x) { this.need(4); this.v.setUint32(this.n, x, true); this.n += 4; return this; }
  f64(x) { this.need(8); this.v.setFloat64(this.n, x, true); this.n += 8; return this; }
  raw(bytes) { this.need(bytes.length); this.b.set(bytes, this.n); this.n += bytes.length; return this; }
  bin(bytes) { return this.u32(bytes.length).raw(bytes); }
  str(s) {
    this.need(4 + s.length * 3);
    const { written } = encoder.encodeInto(s, this.b.subarray(this.n + 4));
    this.v.setUint32(this.n, written, true);
    this.n += 4 + written;
    return this;
  }
  val(x, depth = 0) {
    if (x === undefined) return this.u8(0);
    if (x === null) return this.u8(1);
    if (typeof x === 'boolean') return this.u8(x ? 3 : 2);
    if (typeof x === 'number') return this.u8(4).f64(x);
    if (typeof x === 'string') return this.u8(5).str(x);
    if (typeof x === 'bigint') return this.u8(14).str(String(x));
    if (typeof x !== 'object') return this.u8(0);
    if (depth > 90) throw new Error('Stored value is nested too deeply');
    if (x instanceof Uint8Array) return this.u8(6).bin(x);
    if (Array.isArray(x)) { this.u8(7).u32(x.length); for (let i = 0; i < x.length; i++) this.val(x[i], depth + 1); return this; }
    if (x instanceof Blob) { const id = this.host.hold(x); this.held.push(id); return this.u8(9).u32(id).f64(x.size).str(x.type); }
    if (x instanceof Date) return this.u8(10).f64(x.getTime());
    if (x instanceof ArrayBuffer) return this.u8(11).u8(0).bin(new Uint8Array(x));
    if (ArrayBuffer.isView(x)) return this.u8(11).u8(VIEWS.indexOf(x.constructor) + 1).bin(new Uint8Array(x.buffer, x.byteOffset, x.byteLength));
    if (x instanceof Map) { this.u8(12).u32(x.size); for (const [k, v] of x) this.val(k, depth + 1).val(v, depth + 1); return this; }
    if (x instanceof Set) { this.u8(13).u32(x.size); for (const v of x) this.val(v, depth + 1); return this; }
    const keys = Object.keys(x);
    this.u8(8).u32(keys.length);
    for (const key of keys) this.str(key).val(x[key], depth + 1);
    return this;
  }
  done() { return this.b.subarray(0, this.n); }
}

const isObject = x => x !== null && typeof x === 'object' && Object.getPrototypeOf(x) === Object.prototype;
// Own data properties only, so a key such as `__proto__` is just a key.
const setField = (o, key, value) => Object.defineProperty(o, key, { value, writable: true, enumerable: true, configurable: true });
const getField = (o, key) => (Object.hasOwn(o, key) ? o[key] : undefined);
const setPath = (record, path, value) => {
  let o = record;
  for (let i = 0; i < path.length - 1; i++) {
    let next = getField(o, path[i]);
    if (!isObject(next)) setField(o, path[i], next = {});
    o = next;
  }
  const last = path[path.length - 1];
  if (value === undefined) delete o[last]; else setField(o, last, value);
};
const getPath = (record, path) => {
  let o = record;
  for (const name of path) { if (!isObject(o)) return undefined; o = getField(o, name); }
  return o;
};

export default function install(host) {
  // Bytes of an out-buffer record → structured value. Everything is copied: the callbacks run after wasm returned.
  const val = r => {
    switch (r.u8()) {
      case 0: return undefined;
      case 1: return null;
      case 2: return false;
      case 3: return true;
      case 4: return r.f64();
      case 5: return r.str();
      case 6: return r.copy();
      case 7: { const n = r.u32(), items = new Array(n); for (let i = 0; i < n; i++) items[i] = val(r); return items; }
      case 8: { const n = r.u32(), o = {}; for (let i = 0; i < n; i++) { const key = r.str(); setField(o, key, val(r)); } return o; }
      case 9: { const blob = host.obj(r.u32()); r.f64(); r.str(); return blob; }
      case 10: return new Date(r.f64());
      default: throw new Error('ihn: unknown stored value tag');
    }
  };
  const path = r => { const n = r.u32(), names = []; for (let i = 0; i < n; i++) names.push(r.str()); return names; };
  const range = r => {
    switch (r.u8()) {
      case 0: return null;
      case 1: return IDBKeyRange.only(val(r));
      case 2: { const lower = val(r), upper = val(r), flags = r.u8(); return IDBKeyRange.bound(lower, upper, !!(flags & 1), !!(flags & 2)); }
      case 3: { const key = val(r); return IDBKeyRange.lowerBound(key, !!r.u8()); }
      default: { const key = val(r); return IDBKeyRange.upperBound(key, !!r.u8()); }
    }
  };
  const readOp = r => {
    const op = { t: r.u8(), s: r.u8() };
    switch (op.t) {
      case 1: op.k = val(r); break;
      case 2: op.m = r.u8(); op.k = val(r); op.v = val(r); break;
      case 3: case 5: op.r = range(r); break;
      case 4: op.r = range(r); op.n = r.u32(); op.f = r.u8(); break;
      case 6: { op.k = val(r); op.f = r.u8(); op.p = []; for (let n = r.u32(); n > 0; n--) { const names = path(r); op.p.push([names, val(r)]); } break; }
      case 7: op.k = val(r); op.p = path(r); op.d = r.f64(); op.f = r.u8(); break;
      default: throw new Error('ihn: unknown store operation');
    }
    return op;
  };
  const fail = (req, error, held = []) => {
    for (const id of held) host.objs.delete(id);
    host.io(req, ERRORS[error?.name] || ST.ERROR, 0, encoder.encode(String(error?.message || error || 'IndexedDB failed')));
  };
  const reply = (req, value) => host.io(req, ST.OK, 0, new Enc(host).val(value).done());

  // ---- 3.2 IndexedDB: connections and transactions ----
  const dbs = new Map(); // handle → { idb, stores }
  // One operation; `done(result)` runs when its last request succeeded.
  const step = (store, op, done) => {
    const then = (request, next) => { request.onsuccess = () => next(request.result); };
    switch (op.t) {
      case 1: then(store.get(op.k), done); break;
      case 2:
        if (op.m === 2) then(store.get(op.k), old => { if (old === undefined) then(store.put(op.v, op.k), () => done(old)); else done(old); });
        else if (op.m === 3) then(store.get(op.k), old => {
          // Fill: the stored object keeps its fields and gets the ones it lacks; the result is how it was before.
          if (old === undefined) return then(store.put(op.v, op.k), () => done(old));
          if (!isObject(old) || !isObject(op.v)) return done(old);
          const missing = Object.keys(op.v).filter(key => !Object.hasOwn(old, key));
          if (!missing.length) return done(old);
          const before = {};
          for (const key of Object.keys(old)) setField(before, key, old[key]);
          for (const key of missing) setField(old, key, op.v[key]);
          then(store.put(old, op.k), () => done(before));
        });
        else then(op.m === 1 ? store.add(op.v, op.k) : store.put(op.v, op.k), () => done());
        break;
      case 3: then(op.r ? store.delete(op.r) : store.clear(), () => done()); break;
      case 4: {
        const limit = op.n || undefined;
        then(store.getAllKeys(op.r, limit), keys => {
          if (op.f & 1) done(keys);
          else then(store.getAll(op.r, limit), values => done(keys.flatMap((key, i) => [key, values[i]])));
        });
        break;
      }
      case 5: then(store.count(op.r ?? undefined), done); break;
      case 6: then(store.get(op.k), old => {
        if (old === undefined && op.f & 1) return done(false);
        const record = isObject(old) ? old : {};
        for (const [names, value] of op.p) {
          // Flag 2 (max): a number never replaces a greater (or equal) number.
          if (op.f & 2 && typeof value === 'number') { const now = getPath(record, names); if (typeof now === 'number' && now >= value) continue; }
          setPath(record, names, value);
        }
        then(store.put(record, op.k), () => done(old !== undefined));
      }); break;
      default: then(store.get(op.k), old => {
        let record, sum;
        if (op.p.length) {
          record = isObject(old) ? old : {};
          let parent = record;
          for (const name of op.p.slice(0, -1)) parent = isObject(getField(parent, name)) ? parent[name] : {};
          sum = (Number(getField(parent, op.p[op.p.length - 1])) || 0) + op.d;
          setPath(record, op.p, sum);
        } else {
          record = sum = (typeof old === 'number' ? old : 0) + op.d;
        }
        then(op.f & 1 && sum <= 0 ? store.delete(op.k) : store.put(record, op.k), () => done(sum));
      });
    }
  };
  // The operations run strictly one after the other (each is issued when the previous one finished), so a
  // read-modify-write sees everything before it in the same transaction.
  // Operations of FX 0x26 pieces waiting for their transaction record.
  const parts = [];
  const transact = (req, db, flags, ops) => {
    let tx, failure = null;
    const results = new Array(ops.length);
    try {
      if (!db) throw new DOMException('The database connection is closed', 'InvalidStateError');
      const names = [...new Set(ops.map(op => db.stores[op.s]))];
      tx = db.idb.transaction(names, flags & 1 ? 'readwrite' : 'readonly', flags & 2 ? { durability: 'strict' } : undefined);
    } catch (error) { return fail(req, error); }
    tx.oncomplete = () => {
      const out = new Enc(host);
      try { for (const result of results) out.val(result); } catch (error) { return fail(req, error, out.held); }
      host.io(req, ST.OK, 0, out.done());
    };
    tx.onerror = event => { failure = failure || event.target.error; };
    tx.onabort = () => fail(req, failure || tx.error);
    const run = i => {
      if (i === ops.length) return;
      try {
        step(tx.objectStore(db.stores[ops[i].s]), ops[i], result => { results[i] = result; run(i + 1); });
      } catch (error) {
        failure = error;
        try { tx.abort(); } catch (_) { /* already aborting */ }
      }
    };
    run(0);
  };
  // Opens without creating or upgrading: null when the database does not exist. A connection that arrives after
  // the timeout is closed at once, so it can never block a 5.x tab.
  const openExisting = async name => {
    if (indexedDB.databases && !(await indexedDB.databases()).some(db => db.name === name)) return null;
    return new Promise((resolve, reject) => {
      const request = indexedDB.open(name);
      let late = false;
      const timer = setTimeout(() => { late = true; reject(new Error(`Timed out opening ${name}`)); }, 5000);
      request.onupgradeneeded = () => request.transaction.abort();
      request.onsuccess = () => { clearTimeout(timer); if (late) request.result.close(); else resolve(request.result); };
      request.onerror = event => {
        event.preventDefault();
        clearTimeout(timer);
        if (request.error?.name === 'AbortError') resolve(null); else reject(request.error);
      };
    });
  };
  const walk = (store, keys, each) => new Promise((resolve, reject) => {
    const request = store.openCursor(keys);
    request.onsuccess = () => { const cursor = request.result; if (!cursor || each(cursor) === false) resolve(); else cursor.continue(); };
    request.onerror = () => reject(request.error);
  });
  // Runs `work(db)` on a read-only connection to an existing 5.x database and closes it.
  const withExisting = (req, name, work, held = []) => {
    (async () => {
      const db = await openExisting(name);
      if (!db) return host.io(req, ST.MISSING);
      db.onversionchange = () => db.close(); // a 5.x tab upgrading wins; the read fails and is retried by Rust
      try { await work(db); } finally { db.close(); }
    })().catch(error => fail(req, error, held));
  };

  // ---- 3.9 Web Locks ----
  const locks = new Map(); // req → { abort, release }

  return {
    [FX | 0x20]: r => {
      const req = r.u32(), id = r.u32(), name = r.str(), version = r.u32(), blockedMs = r.f64(), timeoutMs = r.f64(), stores = [];
      for (let n = r.u32(); n > 0; n--) stores.push(r.str());
      let settled = false, timer = 0, blocked = 0;
      const finish = (status, message, idb) => {
        if (settled) { idb?.close(); return; } // a late success after Rust gave up: do not keep the connection
        settled = true;
        clearTimeout(timer);
        clearTimeout(blocked);
        if (idb) {
          const drop = () => { if (dbs.get(id)?.idb === idb) dbs.delete(id); };
          idb.onversionchange = () => { idb.close(); drop(); };
          idb.onclose = drop;
          dbs.get(id)?.idb.close();
          dbs.set(id, { idb, stores });
        }
        host.io(req, status, 0, message ? encoder.encode(message) : null);
      };
      let request;
      try { request = indexedDB.open(name, version); } catch (error) { return finish(ST.UNAVAILABLE, String(error?.message || error)); }
      request.onupgradeneeded = () => {
        for (const store of stores) if (!request.result.objectStoreNames.contains(store)) request.result.createObjectStore(store);
      };
      request.onblocked = () => { blocked = blocked || setTimeout(() => finish(ST.BLOCKED, 'blocked by another window'), blockedMs); };
      request.onsuccess = () => finish(ST.OK, '', request.result);
      request.onerror = event => { event.preventDefault(); finish(ST.ERROR, String(request.error?.message || 'open failed')); };
      timer = setTimeout(() => finish(ST.TIMEOUT, 'open timed out'), timeoutMs);
    },
    [FX | 0x21]: r => { const id = r.u32(); dbs.get(id)?.idb.close(); dbs.delete(id); },
    [FX | 0x22]: r => {
      const req = r.u32(), db = dbs.get(r.u32()), flags = r.u8(), ops = flags & 4 ? parts.splice(0) : [];
      for (let n = r.u32(); n > 0; n--) ops.push(readOp(r));
      transact(req, db, flags, ops);
    },
    // The first operations of the next transaction (flag 4): a large one arrives in pieces, so that wasm memory
    // never holds it twice.
    [FX | 0x26]: r => { for (let n = r.u32(); n > 0; n--) parts.push(readOp(r)); },
    [FX | 0x23]: r => {
      const req = r.u32(), name = r.str(), blockedMs = r.f64();
      let settled = false, blocked = 0;
      const finish = (status, error) => {
        if (settled) return;
        settled = true;
        clearTimeout(blocked);
        if (error) fail(req, error); else host.io(req, status);
      };
      let request;
      try { request = indexedDB.deleteDatabase(name); } catch (error) { return finish(0, error); }
      request.onsuccess = () => finish(ST.OK);
      request.onerror = event => { event.preventDefault(); finish(0, request.error || 'delete failed'); };
      request.onblocked = () => { blocked = blocked || setTimeout(() => finish(ST.BLOCKED), blockedMs); };
    },
    // 5.x info: per store the record count, the greatest numeric key and the greatest of the named numeric fields.
    [FX | 0x24]: r => {
      const req = r.u32(), name = r.str(), fields = r.str().split(',').filter(Boolean);
      withExisting(req, name, async db => {
        const stores = [];
        for (const store of [...db.objectStoreNames].sort()) {
          let count = 0, maxKey = 0, stamp = 0;
          await walk(db.transaction(store).objectStore(store), null, cursor => {
            count++;
            if (typeof cursor.key === 'number' && cursor.key > maxKey) maxKey = cursor.key;
            const value = cursor.value;
            if (value !== null && typeof value === 'object') {
              for (const field of fields) { const x = value[field]; if (typeof x === 'number' && x > stamp) stamp = x; }
            }
          });
          stores.push({ name: store, count, maxKey, stamp });
        }
        reply(req, { version: db.version, stores });
      });
    },
    // 5.x records of one store after a key: [done, key, value, key, value…].
    [FX | 0x25]: r => {
      const req = r.u32(), name = r.str(), store = r.str(), after = val(r), limit = r.u32(), maxBytes = r.f64();
      const out = new Enc(host).u8(7).u32(0).u8(3);
      withExisting(req, name, async db => {
        if (!db.objectStoreNames.contains(store)) return host.io(req, ST.MISSING);
        let n = 0;
        await walk(db.transaction(store).objectStore(store), after === undefined ? null : IDBKeyRange.lowerBound(after, true), cursor => {
          if (n && ((limit && n >= limit) || out.n >= maxBytes)) { out.b[5] = 2; return false; }
          out.val(cursor.key).val(cursor.value);
          n++;
        });
        out.v.setUint32(1, 1 + 2 * n, true);
        host.io(req, ST.OK, 0, out.done());
      }, out.held);
    },
    [FX | 0x80]: r => {
      const req = r.u32(), name = r.str(), flags = r.u8();
      if (!navigator.locks?.request) return host.io(req, ST.UNAVAILABLE);
      const entry = { abort: new AbortController(), release: null };
      const options = { mode: flags & 2 ? 'shared' : 'exclusive' };
      if (flags & 1) options.ifAvailable = true; else options.signal = entry.abort.signal;
      locks.set(req, entry);
      navigator.locks.request(name, options, lock => {
        if (locks.get(req) !== entry) return undefined; // released while it waited
        if (!lock) { locks.delete(req); host.io(req, 0); return undefined; }
        // Held until lock_release — which Rust may send from inside the completion, so `release` exists first.
        const held = new Promise(resolve => { entry.release = resolve; });
        host.io(req, 1);
        return held;
      }).catch(error => { if (locks.get(req) === entry) { locks.delete(req); fail(req, error); } });
    },
    [FX | 0x81]: r => {
      const req = r.u32(), entry = locks.get(req);
      if (!entry) return;
      locks.delete(req);
      if (entry.release) entry.release(); else entry.abort.abort();
    },
    [FX | 0x82]: r => {
      const req = r.u32(), prefix = r.str();
      if (!navigator.locks?.query) return host.io(req, ST.UNAVAILABLE);
      const pick = list => (list || []).map(lock => lock.name).filter(name => name.startsWith(prefix)).sort();
      navigator.locks.query().then(state => reply(req, { held: pick(state.held), pending: pick(state.pending) }), error => fail(req, error));
    }
  };
}
