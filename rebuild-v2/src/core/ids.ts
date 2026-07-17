// Sortable unique ids: 10-char base36 timestamp + 12 chars of randomness.
// Lexicographic order ≈ creation order, which keeps IndexedDB ranges tidy.

const ALPHABET = '0123456789abcdefghijklmnopqrstuvwxyz';

let lastTs = 0;
let counter = 0;

function randomChars(n: number): string {
  const bytes = new Uint8Array(n);
  crypto.getRandomValues(bytes);
  let out = '';
  for (let i = 0; i < n; i++) out += ALPHABET[(bytes[i] as number) % 36];
  return out;
}

export function newId(): string {
  const now = Date.now();
  if (now === lastTs) {
    counter++;
  } else {
    lastTs = now;
    counter = 0;
  }
  const ts = now.toString(36).padStart(9, '0');
  const seq = counter.toString(36).padStart(2, '0');
  return ts + seq + randomChars(12);
}

export type DocId = string;
export type PageId = string;
export type StrokeId = string;
export type ImageId = string;
export type VersionId = string;
