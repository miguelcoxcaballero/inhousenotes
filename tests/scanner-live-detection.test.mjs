import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import { test } from 'node:test';

const context = vm.createContext({ window: {} });
vm.runInContext(fs.readFileSync(new URL('../scanner/processing/liveDetection.js', import.meta.url), 'utf8'), context);
const detect = context.window.ScannerPro.LiveDetection.detect;

// Render in page coordinates, independently of the detector's hull/edge code.
// Solving the bilinear inverse makes every rail and small bottom box undergo
// exactly the same perspective, including pages rotated 90 and 180 degrees.
function frame({ quad = [[100, 25], [348, 34], [375, 455], [73, 447]], yellow = true,
  markers = true, paper = true, background = [68, 78, 83], ink = [240, 219, 76], brightness = 1 } = {}) {
  const width = 480, height = 480, data = new Uint8ClampedArray(width * height * 4);
  const [a, b, c, d] = quad;
  const ex = b[0] - a[0], ey = b[1] - a[1], fx = d[0] - a[0], fy = d[1] - a[1];
  const gx = a[0] - b[0] + c[0] - d[0], gy = a[1] - b[1] + c[1] - d[1];
  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      let u = .5, v = .5;
      for (let i = 0; i < 5; i += 1) {
        const dx = a[0] + ex * u + fx * v + gx * u * v - x;
        const dy = a[1] + ey * u + fy * v + gy * u * v - y;
        const ux = ex + gx * v, uy = ey + gy * v, vx = fx + gx * u, vy = fy + gy * u;
        const determinant = ux * vy - uy * vx;
        u -= (dx * vy - dy * vx) / determinant;
        v -= (dy * ux - dx * uy) / determinant;
      }
      let colour = background;
      if (paper && u >= 0 && u <= 1 && v >= 0 && v <= 1) {
        colour = [244, 244, 240];
        const cmX = u * 21, cmY = v * 29.7;
        const line = .065;
        if (yellow && (((Math.abs(cmX - 1.5) < line || Math.abs(cmX - 19.5) < line)
          && cmY > 1.43 - line && cmY < 28.43 + line)
          || ((Math.abs(cmY - 1.43) < line || Math.abs(cmY - 28.43) < line)
            && cmX > 1.5 - line && cmX < 19.5 + line))) colour = ink;
        if (yellow && markers && ((cmX >= 2 && cmX <= 10) || (cmX >= 11 && cmX <= 19))
          && (Math.abs(cmY - 27.43) < line || Math.abs(cmY - 27.93) < line
            || (cmY >= 27.43 && cmY <= 27.93 && Math.abs(cmX * 2 - Math.round(cmX * 2)) < .08))) colour = ink;
        if (cmX > 3 && cmX < 17 && cmY > 4 && cmY < 24 && cmY % 1.7 < .07) colour = [28, 44, 78];
      }
      const offset = (y * width + x) * 4;
      data[offset] = colour[0] * brightness;
      data[offset + 1] = colour[1] * brightness;
      data[offset + 2] = colour[2] * brightness;
      data[offset + 3] = 255;
    }
  }
  return { width, height, data };
}

for (const [name, quad] of [
  ['portrait', [[100, 25], [348, 34], [375, 455], [73, 447]]],
  ['perspective', [[178, 28], [320, 71], [414, 445], [38, 448]]],
  ['clockwise', [[442, 101], [435, 348], [25, 375], [33, 73]]],
  ['upside down', [[375, 455], [73, 447], [100, 25], [348, 34]]]
]) test(`finds yellow frame and observed bottom boxes: ${name}`, () => {
  const result = detect(frame({ quad }));
  assert.ok(result);
  assert.equal(result.source, 'yellow');
  assert.ok(result.confidence > .86, JSON.stringify(result));
  assert.equal(result.markers.length, 2);
  assert.ok(result.markerSupport >= .65);
  assert.ok(result.sharpness > 45);
  assert.ok(result.quad.every(point => point.x >= 0 && point.x <= 1 && point.y >= 0 && point.y <= 1));
  const trueFrame = [[1.5 / 21, 1.43 / 29.7], [19.5 / 21, 1.43 / 29.7], [19.5 / 21, 28.43 / 29.7], [1.5 / 21, 28.43 / 29.7]]
    .map(([u, v]) => ({
      x: ((quad[0][0] * (1 - u) + quad[1][0] * u) * (1 - v) + (quad[3][0] * (1 - u) + quad[2][0] * u) * v) / 480,
      y: ((quad[0][1] * (1 - u) + quad[1][1] * u) * (1 - v) + (quad[3][1] * (1 - u) + quad[2][1] * u) * v) / 480
    }));
  for (const point of result.quad) assert.ok(Math.min(...trueFrame.map(expected => Math.hypot(point.x - expected.x, point.y - expected.y))) < .018);
});

test('uses paper perimeter without claiming absent yellow markers', () => {
  const result = detect(frame({ yellow: false }));
  assert.ok(result);
  assert.equal(result.source, 'paper');
  assert.ok(result.confidence < .78);
  assert.equal(result.markers.length, 0);
});

test('does not draw predicted boxes when the calibration strip is absent', () => {
  const result = detect(frame({ markers: false }));
  assert.equal(result.source, 'yellow');
  assert.equal(result.markers.length, 0);
  assert.ok(result.markerSupport < .5);
});

test('does not promote a square yellow rectangle to automatic A4 capture', () => {
  const result = detect(frame({ quad: [[80, 80], [400, 80], [400, 400], [80, 400]], markers: false }));
  assert.ok(!result || result.confidence < .78);
});

test('finds a frame against a white desk and in dim light', () => {
  for (const options of [{ background: [242, 241, 237] }, { brightness: .58 }]) {
    const result = detect(frame(options));
    assert.equal(result?.source, 'yellow');
    assert.ok(result.confidence > .86);
  }
});

test('rejects uniform views and yellow surfaces', () => {
  for (const background of [[255, 255, 255], [240, 218, 75], [24, 24, 24]]) assert.equal(detect(frame({ paper: false, background })), null);
  assert.equal(detect(frame({ yellow: false, background: [243, 243, 239] })), null);
});

test('rejects clipped sheets and invalid image data', () => {
  assert.equal(detect(frame({ quad: [[-80, -50], [500, -50], [550, 540], [-100, 540]] })), null);
  assert.equal(detect(null), null);
  assert.equal(detect({ width: 480, height: 480, data: [] }), null);
});
