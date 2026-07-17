import { docKeyRange, req, txDone, STORE_ASSETS } from './idb';

export interface BinaryAsset {
  id: string;
  name: string;
  mimeType: string;
  bytes: Uint8Array;
  createdAt: number;
}

interface AssetRecord extends BinaryAsset {
  docId: string;
}

export async function saveAssets(
  db: IDBDatabase,
  docId: string,
  assets: Iterable<BinaryAsset>
): Promise<void> {
  const records = [...assets].map((asset): AssetRecord => ({ ...asset, docId }));
  if (records.length === 0) return;
  const tx = db.transaction(STORE_ASSETS, 'readwrite');
  const store = tx.objectStore(STORE_ASSETS);
  for (const record of records) store.put(record);
  await txDone(tx);
}

export async function loadAssets(db: IDBDatabase, docId: string): Promise<BinaryAsset[]> {
  const tx = db.transaction(STORE_ASSETS, 'readonly');
  const records = (await req(storeFor(tx).getAll(docKeyRange(docId)))) as AssetRecord[];
  return records.map(({ docId: _docId, ...asset }) => asset);
}

export async function deleteAssets(db: IDBDatabase, docId: string): Promise<void> {
  const tx = db.transaction(STORE_ASSETS, 'readwrite');
  storeFor(tx).delete(docKeyRange(docId));
  await txDone(tx);
}

function storeFor(tx: IDBTransaction): IDBObjectStore {
  return tx.objectStore(STORE_ASSETS);
}
