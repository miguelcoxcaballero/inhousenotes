import { describe, expect, it } from 'vitest';
import { extractFlattenedInk } from './flattenedInk';

function whiteImage(width: number, height: number): Uint8ClampedArray {
  const data = new Uint8ClampedArray(width * height * 4);
  for (let i = 0; i < width * height; i++) {
    data[i * 4] = 255;
    data[i * 4 + 1] = 255;
    data[i * 4 + 2] = 255;
    data[i * 4 + 3] = 255;
  }
  return data;
}

function pixel(data: Uint8ClampedArray, width: number, x: number, y: number, color: [number, number, number]): void {
  const offset = (y * width + x) * 4;
  data[offset] = color[0];
  data[offset + 1] = color[1];
  data[offset + 2] = color[2];
}

describe('flattened coloured ink recovery', () => {
  it('separates blue handwriting while preserving grayscale content', () => {
    const width = 72;
    const height = 48;
    const data = whiteImage(width, height);
    for (let y = 5; y < 12; y++) {
      for (let x = 6; x < 24; x++) pixel(data, width, x, y, [28, 28, 28]);
    }
    for (let x = 28; x < 62; x++) {
      const y = 19 + Math.round((x - 28) * 0.45);
      for (let dy = -1; dy <= 1; dy++) pixel(data, width, x, y + dy, [24, 70, 245]);
    }

    const recovered = extractFlattenedInk({ width, height, data }, width, height);

    expect(recovered).not.toBeNull();
    expect(recovered!.strokes.length).toBeGreaterThan(0);
    expect(recovered!.strokes.length).toBeLessThan(30);
    expect(recovered!.strokes.every((stroke) => stroke.color.startsWith('#'))).toBe(true);
    expect(recovered!.strokes.every((stroke) => [...stroke.points].every(Number.isFinite))).toBe(true);
    const blackOffset = (7 * width + 8) * 4;
    expect([...recovered!.cleaned.slice(blackOffset, blackOffset + 3)]).toEqual([28, 28, 28]);
    const inkOffset = (24 * width + 39) * 4;
    expect(recovered!.cleaned[inkOffset]).toBeGreaterThan(220);
    expect(recovered!.cleaned[inkOffset + 1]).toBeGreaterThan(220);
    expect(recovered!.cleaned[inkOffset + 2]).toBeGreaterThan(220);
  });

  it('does not alter grayscale-only pages', () => {
    const width = 40;
    const height = 30;
    const data = whiteImage(width, height);
    for (let x = 4; x < 35; x++) pixel(data, width, x, 14, [20, 20, 20]);
    expect(extractFlattenedInk({ width, height, data }, width, height)).toBeNull();
  });

  it('rejects large colour regions such as photos or filled graphics', () => {
    const width = 60;
    const height = 40;
    const data = whiteImage(width, height);
    for (let y = 5; y < 30; y++) {
      for (let x = 8; x < 45; x++) pixel(data, width, x, y, [20, 80, 230]);
    }
    expect(extractFlattenedInk({ width, height, data }, width, height)).toBeNull();
  });
});
