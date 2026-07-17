// Decoded-image cache with LRU eviction (replaces the legacy unbounded
// imageCache/imageCacheUsage pair).

const MAX_ENTRIES = 48;

const cache = new Map<string, HTMLImageElement>();

export function getCachedImage(src: string, onload?: () => void): HTMLImageElement {
  let img = cache.get(src);
  if (img) {
    // Refresh LRU position.
    cache.delete(src);
    cache.set(src, img);
    if (!img.complete && onload) img.addEventListener('load', onload, { once: true });
    return img;
  }
  img = new Image();
  if (onload) img.addEventListener('load', onload, { once: true });
  img.src = src;
  cache.set(src, img);
  while (cache.size > MAX_ENTRIES) {
    const oldest = cache.keys().next().value;
    if (oldest === undefined) break;
    cache.delete(oldest);
  }
  return img;
}

export function dropCachedImage(src: string): void {
  cache.delete(src);
}
