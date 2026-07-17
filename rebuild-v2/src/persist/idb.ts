// Minimal promise wrapper over IndexedDB + the v2 schema.
//
// Everything is keyed by identity ([docId, pageId], [docId, seq], …), never
// by array position — reordering pages touches a single doc record.

export const DB_NAME = 'inhouse-notes-v2';
export const DB_VERSION = 1;

export const STORE_DOCS = 'docs';
export const STORE_PAGES = 'pages';
export const STORE_OPLOG = 'oplog';
export const STORE_VERSIONS = 'versions';

export function req<T>(request: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

export function txDone(tx: IDBTransaction): Promise<void> {
  return new Promise((resolve, reject) => {
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
    tx.onabort = () => reject(tx.error ?? new Error('Transaction aborted'));
  });
}

let dbPromise: Promise<IDBDatabase> | null = null;

export function openDb(factory: IDBFactory = indexedDB): Promise<IDBDatabase> {
  if (dbPromise) return dbPromise;
  dbPromise = new Promise((resolve, reject) => {
    const request = factory.open(DB_NAME, DB_VERSION);
    request.onupgradeneeded = () => {
      const db = request.result;
      if (!db.objectStoreNames.contains(STORE_DOCS)) {
        db.createObjectStore(STORE_DOCS, { keyPath: 'id' });
      }
      if (!db.objectStoreNames.contains(STORE_PAGES)) {
        db.createObjectStore(STORE_PAGES, { keyPath: ['docId', 'id'] });
      }
      if (!db.objectStoreNames.contains(STORE_OPLOG)) {
        db.createObjectStore(STORE_OPLOG, { keyPath: ['docId', 'seq'] });
      }
      if (!db.objectStoreNames.contains(STORE_VERSIONS)) {
        db.createObjectStore(STORE_VERSIONS, { keyPath: ['docId', 'id'] });
      }
    };
    request.onsuccess = () => {
      const db = request.result;
      db.onversionchange = () => {
        try {
          db.close();
        } catch {
          /* ignore */
        }
        dbPromise = null;
      };
      resolve(db);
    };
    request.onerror = () => {
      dbPromise = null;
      reject(request.error);
    };
  });
  return dbPromise;
}

/** Test/teardown hook: forget the cached connection. */
export function resetDbCache(): void {
  dbPromise = null;
}

/** All keys for a doc inside a composite-key store ([docId, x]). */
export function docKeyRange(docId: string): IDBKeyRange {
  return IDBKeyRange.bound([docId], [docId, []], false, false);
}
