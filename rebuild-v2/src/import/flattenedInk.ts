import { simplifyPoints } from '../core/geometry';

export interface PixelBuffer {
  width: number;
  height: number;
  data: Uint8ClampedArray;
}

export interface RecoveredInkStroke {
  color: string;
  width: number;
  points: Float32Array;
}

export interface FlattenedInkExtraction {
  cleaned: Uint8ClampedArray;
  strokes: RecoveredInkStroke[];
  extractedPixels: number;
}

interface Component {
  pixels: number[];
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

const HUE_BINS = 36;
const MAX_COLOR_COVERAGE = 0.08;
const MIN_CORE_PIXELS = 18;
const MAX_STROKES = 4000;

/**
 * Recover pen-like coloured ink from a flattened raster page. The detector is
 * intentionally conservative: grayscale pages, photos and solid colour blocks
 * are left untouched.
 */
export function extractFlattenedInk(
  image: PixelBuffer,
  pageWidth: number,
  pageHeight: number
): FlattenedInkExtraction | null {
  const { width, height, data } = image;
  if (width < 8 || height < 8 || data.length !== width * height * 4) return null;

  const hueHistogram = new Float64Array(HUE_BINS);
  let corePixels = 0;
  for (let i = 0; i < width * height; i++) {
    const offset = i * 4;
    const r = data[offset]!;
    const g = data[offset + 1]!;
    const b = data[offset + 2]!;
    const a = data[offset + 3]!;
    const max = Math.max(r, g, b);
    const min = Math.min(r, g, b);
    const chroma = max - min;
    if (a < 128 || max < 55 || chroma < 48 || chroma / Math.max(1, max) < 0.25) continue;
    const hue = rgbHue(r, g, b);
    const bin = Math.min(HUE_BINS - 1, Math.floor(hue / (360 / HUE_BINS)));
    hueHistogram[bin] = hueHistogram[bin]! + chroma;
    corePixels++;
  }
  if (corePixels < MIN_CORE_PIXELS) return null;

  let dominantBin = 0;
  for (let i = 1; i < hueHistogram.length; i++) {
    if (hueHistogram[i]! > hueHistogram[dominantBin]!) dominantBin = i;
  }
  const dominantHue = (dominantBin + 0.5) * (360 / HUE_BINS);
  const candidateMask = new Uint8Array(width * height);
  let candidateCount = 0;
  for (let i = 0; i < candidateMask.length; i++) {
    const offset = i * 4;
    const r = data[offset]!;
    const g = data[offset + 1]!;
    const b = data[offset + 2]!;
    const a = data[offset + 3]!;
    const max = Math.max(r, g, b);
    const min = Math.min(r, g, b);
    const chroma = max - min;
    if (
      a >= 96 &&
      max >= 42 &&
      chroma >= 14 &&
      chroma / Math.max(1, max) >= 0.07 &&
      hueDistance(rgbHue(r, g, b), dominantHue) <= 28
    ) {
      candidateMask[i] = 1;
      candidateCount++;
    }
  }

  const coverage = candidateCount / candidateMask.length;
  if (candidateCount < MIN_CORE_PIXELS || coverage > MAX_COLOR_COVERAGE) return null;

  const scaleX = width / Math.max(1, pageWidth);
  const scaleY = height / Math.max(1, pageHeight);
  const minComponentArea = Math.max(4, Math.round(2 * scaleX * scaleY));
  const keptMask = new Uint8Array(candidateMask.length);
  const components = findComponents(candidateMask, width, height);
  let extractedPixels = 0;
  for (const component of components) {
    const boxArea = (component.x1 - component.x0 + 1) * (component.y1 - component.y0 + 1);
    const fillRatio = component.pixels.length / Math.max(1, boxArea);
    if (component.pixels.length < minComponentArea) continue;
    if (fillRatio > 0.46 && boxArea > 36 * scaleX * scaleY) continue;
    if (!hasLightNeutralSurroundings(component, candidateMask, data, width, height)) continue;
    for (const index of component.pixels) keptMask[index] = 1;
    extractedPixels += component.pixels.length;
  }
  if (extractedPixels < MIN_CORE_PIXELS) return null;

  const modelWidth = Math.max(1, Math.round(pageWidth));
  const modelHeight = Math.max(1, Math.round(pageHeight));
  const modelMask = downsampleMask(keptMask, width, height, modelWidth, modelHeight);
  const color = averageColor(data, keptMask);
  const strokes = vectorizeMask(modelMask, modelWidth, modelHeight, color);
  if (strokes.length === 0 || strokes.length > MAX_STROKES) return null;

  const cleaned = new Uint8ClampedArray(data);
  removeInkFromRaster(cleaned, keptMask, width, height);
  return { cleaned, strokes, extractedPixels };
}

function rgbHue(r: number, g: number, b: number): number {
  const rn = r / 255;
  const gn = g / 255;
  const bn = b / 255;
  const max = Math.max(rn, gn, bn);
  const min = Math.min(rn, gn, bn);
  const delta = max - min;
  if (delta === 0) return 0;
  let hue: number;
  if (max === rn) hue = ((gn - bn) / delta) % 6;
  else if (max === gn) hue = (bn - rn) / delta + 2;
  else hue = (rn - gn) / delta + 4;
  return (hue * 60 + 360) % 360;
}

function hueDistance(a: number, b: number): number {
  const distance = Math.abs(a - b) % 360;
  return Math.min(distance, 360 - distance);
}

function findComponents(mask: Uint8Array, width: number, height: number): Component[] {
  const visited = new Uint8Array(mask.length);
  const queue = new Int32Array(mask.length);
  const result: Component[] = [];
  for (let start = 0; start < mask.length; start++) {
    if (!mask[start] || visited[start]) continue;
    let read = 0;
    let write = 0;
    queue[write++] = start;
    visited[start] = 1;
    const pixels: number[] = [];
    let x0 = width;
    let y0 = height;
    let x1 = 0;
    let y1 = 0;
    while (read < write) {
      const index = queue[read++]!;
      pixels.push(index);
      const x = index % width;
      const y = Math.floor(index / width);
      x0 = Math.min(x0, x);
      y0 = Math.min(y0, y);
      x1 = Math.max(x1, x);
      y1 = Math.max(y1, y);
      for (let dy = -1; dy <= 1; dy++) {
        for (let dx = -1; dx <= 1; dx++) {
          if (dx === 0 && dy === 0) continue;
          const nx = x + dx;
          const ny = y + dy;
          if (nx < 0 || ny < 0 || nx >= width || ny >= height) continue;
          const next = ny * width + nx;
          if (!mask[next] || visited[next]) continue;
          visited[next] = 1;
          queue[write++] = next;
        }
      }
    }
    result.push({ pixels, x0, y0, x1, y1 });
  }
  return result;
}

function hasLightNeutralSurroundings(
  component: Component,
  mask: Uint8Array,
  data: Uint8ClampedArray,
  width: number,
  height: number
): boolean {
  const pad = 3;
  let samples = 0;
  let acceptable = 0;
  const x0 = Math.max(0, component.x0 - pad);
  const y0 = Math.max(0, component.y0 - pad);
  const x1 = Math.min(width - 1, component.x1 + pad);
  const y1 = Math.min(height - 1, component.y1 + pad);
  for (let y = y0; y <= y1; y++) {
    for (let x = x0; x <= x1; x++) {
      if (x > x0 && x < x1 && y > y0 && y < y1) continue;
      const index = y * width + x;
      if (mask[index]) continue;
      const offset = index * 4;
      const r = data[offset]!;
      const g = data[offset + 1]!;
      const b = data[offset + 2]!;
      const max = Math.max(r, g, b);
      const min = Math.min(r, g, b);
      samples++;
      if (min >= 176 && max - min <= 28) acceptable++;
    }
  }
  return samples === 0 || acceptable / samples >= 0.72;
}

function downsampleMask(
  source: Uint8Array,
  sourceWidth: number,
  sourceHeight: number,
  targetWidth: number,
  targetHeight: number
): Uint8Array {
  const target = new Uint8Array(targetWidth * targetHeight);
  for (let y = 0; y < sourceHeight; y++) {
    const ty = Math.min(targetHeight - 1, Math.floor((y * targetHeight) / sourceHeight));
    for (let x = 0; x < sourceWidth; x++) {
      if (!source[y * sourceWidth + x]) continue;
      const tx = Math.min(targetWidth - 1, Math.floor((x * targetWidth) / sourceWidth));
      target[ty * targetWidth + tx] = 1;
    }
  }
  return target;
}

function averageColor(data: Uint8ClampedArray, mask: Uint8Array): string {
  let r = 0;
  let g = 0;
  let b = 0;
  let weight = 0;
  for (let i = 0; i < mask.length; i++) {
    if (!mask[i]) continue;
    const offset = i * 4;
    const pr = data[offset]!;
    const pg = data[offset + 1]!;
    const pb = data[offset + 2]!;
    const chroma = Math.max(pr, pg, pb) - Math.min(pr, pg, pb);
    const w = Math.max(1, chroma);
    r += pr * w;
    g += pg * w;
    b += pb * w;
    weight += w;
  }
  const hex = (value: number) => Math.max(0, Math.min(255, Math.round(value / Math.max(1, weight))))
    .toString(16)
    .padStart(2, '0');
  return `#${hex(r)}${hex(g)}${hex(b)}`;
}

function vectorizeMask(mask: Uint8Array, width: number, height: number, color: string): RecoveredInkStroke[] {
  const strokes: RecoveredInkStroke[] = [];
  for (const component of findComponents(mask, width, height)) {
    const cropWidth = component.x1 - component.x0 + 3;
    const cropHeight = component.y1 - component.y0 + 3;
    const crop = new Uint8Array(cropWidth * cropHeight);
    for (const index of component.pixels) {
      const x = index % width;
      const y = Math.floor(index / width);
      crop[(y - component.y0 + 1) * cropWidth + (x - component.x0 + 1)] = 1;
    }
    const skeleton = thin(crop, cropWidth, cropHeight);
    let skeletonPixels = 0;
    for (const value of skeleton) skeletonPixels += value;
    if (skeletonPixels === 0) continue;
    const strokeWidth = Math.max(0.8, Math.min(8, component.pixels.length / skeletonPixels));
    for (const path of traceSkeleton(skeleton, cropWidth, cropHeight)) {
      if (path.length === 0) continue;
      const packed = new Float32Array(Math.max(2, path.length) * 3);
      const points = path.length === 1 ? [path[0]!, path[0]!] : path;
      for (let i = 0; i < points.length; i++) {
        const index = points[i]!;
        packed[i * 3] = (index % cropWidth) + component.x0 - 0.5;
        packed[i * 3 + 1] = Math.floor(index / cropWidth) + component.y0 - 0.5;
        packed[i * 3 + 2] = 0.5;
      }
      strokes.push({ color, width: strokeWidth, points: simplifyPoints(packed, 0.55) });
    }
  }
  return strokes;
}

function thin(input: Uint8Array, width: number, height: number): Uint8Array {
  const pixels = new Uint8Array(input);
  const remove = new Uint8Array(input.length);
  for (let iteration = 0; iteration < 80; iteration++) {
    let changed = false;
    for (let phase = 0; phase < 2; phase++) {
      remove.fill(0);
      for (let y = 1; y < height - 1; y++) {
        for (let x = 1; x < width - 1; x++) {
          const index = y * width + x;
          if (!pixels[index]) continue;
          const p2 = pixels[index - width]!;
          const p3 = pixels[index - width + 1]!;
          const p4 = pixels[index + 1]!;
          const p5 = pixels[index + width + 1]!;
          const p6 = pixels[index + width]!;
          const p7 = pixels[index + width - 1]!;
          const p8 = pixels[index - 1]!;
          const p9 = pixels[index - width - 1]!;
          const neighbours = p2 + p3 + p4 + p5 + p6 + p7 + p8 + p9;
          if (neighbours < 2 || neighbours > 6) continue;
          const transitions = Number(!p2 && p3) + Number(!p3 && p4) + Number(!p4 && p5) +
            Number(!p5 && p6) + Number(!p6 && p7) + Number(!p7 && p8) + Number(!p8 && p9) + Number(!p9 && p2);
          if (transitions !== 1) continue;
          const first = phase === 0 ? p2 * p4 * p6 : p2 * p4 * p8;
          const second = phase === 0 ? p4 * p6 * p8 : p2 * p6 * p8;
          if (first || second) continue;
          remove[index] = 1;
        }
      }
      for (let i = 0; i < remove.length; i++) {
        if (!remove[i]) continue;
        pixels[i] = 0;
        changed = true;
      }
    }
    if (!changed) break;
  }
  return pixels;
}

function traceSkeleton(mask: Uint8Array, width: number, height: number): number[][] {
  const paths: number[][] = [];
  const visitedEdges = new Set<string>();
  const pixels: number[] = [];
  for (let i = 0; i < mask.length; i++) if (mask[i]) pixels.push(i);
  const edgeKey = (a: number, b: number) => a < b ? `${a}:${b}` : `${b}:${a}`;
  const neighbours = (index: number): number[] => {
    const x = index % width;
    const y = Math.floor(index / width);
    const values: number[] = [];
    for (let dy = -1; dy <= 1; dy++) {
      for (let dx = -1; dx <= 1; dx++) {
        if (dx === 0 && dy === 0) continue;
        const nx = x + dx;
        const ny = y + dy;
        if (nx < 0 || ny < 0 || nx >= width || ny >= height) continue;
        const next = ny * width + nx;
        if (!mask[next]) continue;
        if (dx !== 0 && dy !== 0) {
          const horizontal = y * width + nx;
          const vertical = ny * width + x;
          if (mask[horizontal] || mask[vertical]) continue;
        }
        values.push(next);
      }
    }
    return values;
  };
  const walk = (start: number, next: number): number[] => {
    const path = [start];
    let previous = start;
    let current = next;
    visitedEdges.add(edgeKey(start, next));
    for (let guard = 0; guard < mask.length; guard++) {
      path.push(current);
      const options = neighbours(current);
      if (options.length !== 2) break;
      const candidate = options[0] === previous ? options[1]! : options[0]!;
      const key = edgeKey(current, candidate);
      if (visitedEdges.has(key)) break;
      visitedEdges.add(key);
      previous = current;
      current = candidate;
    }
    return path;
  };

  for (const pixel of pixels) {
    const adjacent = neighbours(pixel);
    if (adjacent.length === 2) continue;
    if (adjacent.length === 0) {
      paths.push([pixel]);
      continue;
    }
    for (const next of adjacent) {
      if (!visitedEdges.has(edgeKey(pixel, next))) paths.push(walk(pixel, next));
    }
  }
  for (const pixel of pixels) {
    for (const next of neighbours(pixel)) {
      if (!visitedEdges.has(edgeKey(pixel, next))) paths.push(walk(pixel, next));
    }
  }
  return paths;
}

function removeInkFromRaster(
  data: Uint8ClampedArray,
  mask: Uint8Array,
  width: number,
  height: number
): void {
  for (let index = 0; index < mask.length; index++) {
    if (!mask[index]) continue;
    const x = index % width;
    const y = Math.floor(index / width);
    let r = 0;
    let g = 0;
    let b = 0;
    let samples = 0;
    for (let radius = 2; radius <= 7 && samples < 4; radius++) {
      for (let dy = -radius; dy <= radius; dy++) {
        for (let dx = -radius; dx <= radius; dx++) {
          if (Math.abs(dx) !== radius && Math.abs(dy) !== radius) continue;
          const nx = x + dx;
          const ny = y + dy;
          if (nx < 0 || ny < 0 || nx >= width || ny >= height) continue;
          const neighbour = ny * width + nx;
          if (mask[neighbour]) continue;
          const offset = neighbour * 4;
          const nr = data[offset]!;
          const ng = data[offset + 1]!;
          const nb = data[offset + 2]!;
          if (Math.max(nr, ng, nb) - Math.min(nr, ng, nb) > 28) continue;
          r += nr;
          g += ng;
          b += nb;
          samples++;
        }
      }
    }
    const offset = index * 4;
    data[offset] = samples ? Math.round(r / samples) : 255;
    data[offset + 1] = samples ? Math.round(g / samples) : 255;
    data[offset + 2] = samples ? Math.round(b / samples) : 255;
  }
}
