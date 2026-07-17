// Collaborative merge between the local document and a remote snapshot,
// optionally informed by the last synced base (true three-way).
//
// Rules (port of the legacy §4.21c semantics, by id instead of index):
// - Pages: union by id, minus pages tombstoned on either side.
//   Order follows remote for shared pages; local-only pages keep their
//   position relative to their local predecessor.
// - Strokes/images per page: union by id, minus ids tombstoned on either
//   side. Order follows remote, local-only elements appended in local order.
// - Same id changed on both sides (transforms, background, meta): with a
//   base, the side that actually changed wins; if both changed, local wins
//   (the user is looking at it and their op is already queued for upload).
// - Tombstones: union.

import type { SerialDoc, SerialPage, SerialStroke } from './serial';
import type { Img } from '../core/model';

export interface MergeResult {
  merged: SerialDoc;
  /** Remote had content/ordering local was missing. */
  changedFromLocal: boolean;
  /** Local has content remote was missing → an upload is still needed. */
  changedFromRemote: boolean;
}

function strokeSig(s: SerialStroke): string {
  return `${s.tool}|${s.color}|${s.width}|${Array.from(s.points).join(',')}`;
}

function imageSig(img: Img): string {
  return `${img.x}|${img.y}|${img.width}|${img.height}|${img.rotation}|${img.src}`;
}

function pageMetaSig(p: SerialPage): string {
  return `${p.width}x${p.height}|${JSON.stringify(p.background)}|${JSON.stringify(p.sidePanel)}`;
}

/** Pick winner for an element present on both sides. */
function pickBoth<T>(local: T, remote: T, baseSig: string | null, sig: (v: T) => string): T {
  const localSig = sig(local);
  const remoteSig = sig(remote);
  if (localSig === remoteSig) return local;
  if (baseSig !== null) {
    if (localSig === baseSig) return remote; // only remote changed
    return local; // only local changed, or both → local wins
  }
  return local;
}

function mergeOrder(
  remoteOrder: string[],
  localOrder: string[],
  keep: (id: string) => boolean
): string[] {
  const remoteSet = new Set(remoteOrder);
  const out = remoteOrder.filter(keep);
  // Insert local-only ids after their nearest local predecessor that
  // survived into the merged order (or at the front).
  for (let i = 0; i < localOrder.length; i++) {
    const id = localOrder[i]!;
    if (remoteSet.has(id) || !keep(id) || out.includes(id)) continue;
    let insertAt = 0;
    for (let j = i - 1; j >= 0; j--) {
      const prev = localOrder[j]!;
      const at = out.indexOf(prev);
      if (at >= 0) {
        insertAt = at + 1;
        break;
      }
    }
    out.splice(insertAt, 0, id);
  }
  return out;
}

function mergePage(local: SerialPage, remote: SerialPage, base: SerialPage | null): SerialPage {
  const tombstones = new Set([...local.tombstones, ...remote.tombstones]);
  const keep = (id: string) => !tombstones.has(id);

  const localStrokes = new Map(local.strokes.map((s) => [s.id, s]));
  const remoteStrokes = new Map(remote.strokes.map((s) => [s.id, s]));
  const baseStrokes = base ? new Map(base.strokes.map((s) => [s.id, s])) : null;

  const strokeOrder = mergeOrder(
    remote.strokes.map((s) => s.id),
    local.strokes.map((s) => s.id),
    keep
  );
  const strokes: SerialStroke[] = [];
  for (const id of strokeOrder) {
    const localStroke = localStrokes.get(id);
    const remoteStroke = remoteStrokes.get(id);
    if (localStroke && remoteStroke) {
      const baseEntry = baseStrokes?.get(id);
      strokes.push(pickBoth(localStroke, remoteStroke, baseEntry ? strokeSig(baseEntry) : null, strokeSig));
    } else {
      strokes.push((localStroke ?? remoteStroke)!);
    }
  }

  const localImages = new Map(local.images.map((img) => [img.id, img]));
  const remoteImages = new Map(remote.images.map((img) => [img.id, img]));
  const baseImages = base ? new Map(base.images.map((img) => [img.id, img])) : null;

  const imageOrder = mergeOrder(
    remote.images.map((img) => img.id),
    local.images.map((img) => img.id),
    keep
  );
  const images: Img[] = [];
  for (const id of imageOrder) {
    const localImage = localImages.get(id);
    const remoteImage = remoteImages.get(id);
    if (localImage && remoteImage) {
      const baseEntry = baseImages?.get(id);
      images.push(pickBoth(localImage, remoteImage, baseEntry ? imageSig(baseEntry) : null, imageSig));
    } else {
      images.push((localImage ?? remoteImage)!);
    }
  }

  const meta = pickBoth(local, remote, base ? pageMetaSig(base) : null, pageMetaSig);

  return {
    id: local.id,
    width: meta.width,
    height: meta.height,
    background: meta.background,
    strokes,
    images,
    tombstones: [...tombstones],
    sidePanel: meta.sidePanel
  };
}

function pageContentSig(p: SerialPage): string {
  return `${pageMetaSig(p)}|${p.strokes.map(strokeSig).join(',')}|${p.images.map(imageSig).join(',')}|${[...p.tombstones].sort().join(',')}`;
}

export function mergeDocs(local: SerialDoc, remote: SerialDoc, base: SerialDoc | null = null): MergeResult {
  const pageTombstones = new Set([...(local.pageTombstones ?? []), ...(remote.pageTombstones ?? [])]);
  const keepPage = (id: string) => !pageTombstones.has(id);

  const localPages = new Map(local.pages.map((p) => [p.id, p]));
  const remotePages = new Map(remote.pages.map((p) => [p.id, p]));
  const basePages = base ? new Map(base.pages.map((p) => [p.id, p])) : null;

  const pageOrder = mergeOrder(
    remote.pages.map((p) => p.id),
    local.pages.map((p) => p.id),
    keepPage
  );

  const pages: SerialPage[] = [];
  for (const id of pageOrder) {
    const localPage = localPages.get(id);
    const remotePage = remotePages.get(id);
    if (localPage && remotePage) {
      pages.push(mergePage(localPage, remotePage, basePages?.get(id) ?? null));
    } else {
      pages.push((localPage ?? remotePage)!);
    }
  }

  const merged: SerialDoc = {
    id: local.id,
    rev: local.rev,
    pageOrder: pages.map((p) => p.id),
    pages,
    pageTombstones: [...pageTombstones],
    meta: { ...remote.meta, ...((base && JSON.stringify(local.meta) !== JSON.stringify(base.meta)) || !base ? local.meta : {}) }
  };

  const sigOf = (d: SerialDoc) => [
    d.pages.map((p) => `${p.id}:${pageContentSig(p)}`).join(';'),
    d.pageOrder.join(','),
    [...(d.pageTombstones ?? [])].sort().join(','),
    JSON.stringify(d.meta)
  ].join('|');
  const mergedSig = sigOf(merged);
  return {
    merged,
    changedFromLocal: mergedSig !== sigOf(local),
    changedFromRemote: mergedSig !== sigOf(remote)
  };
}
