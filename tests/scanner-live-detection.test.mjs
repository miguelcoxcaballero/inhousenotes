import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import { test } from 'node:test';

const context = vm.createContext({ window: {}, setTimeout });
vm.runInContext(fs.readFileSync(new URL('../scanner/processing/liveDetection.js', import.meta.url), 'utf8'), context);
const detect = context.window.ScannerPro.LiveDetection.detect;
const normalizeIllumination = context.window.ScannerPro.LiveDetection.normalizeIllumination;
const normalizeIlluminationAsync = context.window.ScannerPro.LiveDetection.normalizeIlluminationAsync;

// Render in page coordinates, independently of the detector's hull/edge code.
// Solving the bilinear inverse makes every rail and small bottom box undergo
// exactly the same perspective, including pages rotated 90 and 180 degrees.
function frame({ quad = [[100, 25], [348, 34], [375, 455], [73, 447]], yellow = true,
  markers = true, paper = true, background = [68, 78, 83], ink = [240, 219, 76], brightness = 1,
  bowX = 0, bowY = 0, brokenRail = false, brokenCorner = false, railJitter = 0, railPhase = 0, colourMarkers = false } = {}) {
  const width = 480, height = 480, data = new Uint8ClampedArray(width * height * 4);
  const [a, b, c, d] = quad;
  const ex = b[0] - a[0], ey = b[1] - a[1], fx = d[0] - a[0], fy = d[1] - a[1];
  const gx = a[0] - b[0] + c[0] - d[0], gy = a[1] - b[1] + c[1] - d[1];
  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      let u = .5, v = .5;
      for (let i = 0; i < 5; i += 1) {
        const dx = a[0] + ex * u + fx * v + gx * u * v + bowX * Math.sin(Math.PI * v) - x;
        const dy = a[1] + ey * u + fy * v + gy * u * v + bowY * Math.sin(Math.PI * u) - y;
        const ux = ex + gx * v, uy = ey + gy * v + bowY * Math.PI * Math.cos(Math.PI * u);
        const vx = fx + gx * u + bowX * Math.PI * Math.cos(Math.PI * v), vy = fy + gy * u;
        const determinant = ux * vy - uy * vx;
        u -= (dx * vy - dy * vx) / determinant;
        v -= (dy * ux - dx * uy) / determinant;
      }
      let colour = background;
      if (paper && u >= 0 && u <= 1 && v >= 0 && v <= 1) {
        colour = [244, 244, 240];
        const cmX = u * 21, cmY = v * 29.7;
        const line = .065;
        const railX = cmX + (v > .09 && v < .91 ? railJitter * Math.sin(cmY * 4 + railPhase) : 0);
        const railY = cmY + (u > .13 && u < .87 ? railJitter * Math.sin(cmX * 6 + railPhase) : 0);
        if (yellow && (((Math.abs(railX - 1.5) < line || Math.abs(railX - 19.5) < line)
          && cmY > 1.43 - line && cmY < 28.43 + line)
          || ((Math.abs(railY - 1.43) < line || Math.abs(railY - 28.43) < line)
            && cmX > 1.5 - line && cmX < 19.5 + line))) colour = ink;
        if (yellow && markers && ((cmX >= 2 && cmX <= 10) || (cmX >= 11 && cmX <= 19))
          && (Math.abs(cmY - 27.43) < line || Math.abs(cmY - 27.93) < line
            || (cmY >= 27.43 && cmY <= 27.93 && Math.abs(cmX * 2 - Math.round(cmX * 2)) < .08))) colour = ink;
        if (colourMarkers) {
          const colours = Array.isArray(colourMarkers) ? colourMarkers : [[255, 0, 0], [0, 0, 0], [0, 0, 255], [110, 255, 18]];
          for (let marker = 0; marker < 4; marker += 1) {
            if (Math.hypot(cmX - (10.125 + marker * .25), cmY - 27.68) < .125) colour = colours[marker];
          }
        }
        if (brokenRail && cmX > 19 && cmY > 11 && cmY < 18) colour = [244, 244, 240];
        if (brokenCorner && cmX > 18 && cmY < 3.5) colour = [244, 244, 240];
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


test('traces observed curved rails rather than drawing the corner chords', () => {
  const quad = [[100, 25], [348, 34], [375, 455], [73, 447]];
  for (const [bowX, bowY] of [[25, 0], [-24, 18], [0, -22]]) {
    const result = detect(frame({ quad, bowX, bowY }));
    assert.equal(result?.source, 'yellow');
    assert.ok(result.confidence >= .78);
    assert.equal(result.frameEdges.length, 4);
    const expected = [];
    for (let side = 0; side < 4; side += 1) {
      for (let step = 0; step <= 120; step += 1) {
        const t = step / 120;
        const u = side === 0 ? (1.5 + 18 * t) / 21 : side === 1 ? 19.5 / 21 : side === 2 ? (19.5 - 18 * t) / 21 : 1.5 / 21;
        const v = side === 1 ? (1.43 + 27 * t) / 29.7 : side === 2 ? 28.43 / 29.7 : side === 3 ? (28.43 - 27 * t) / 29.7 : 1.43 / 29.7;
        expected.push({
          x: ((quad[0][0] * (1 - u) + quad[1][0] * u) * (1 - v) + (quad[3][0] * (1 - u) + quad[2][0] * u) * v + bowX * Math.sin(Math.PI * v)) / 480,
          y: ((quad[0][1] * (1 - u) + quad[1][1] * u) * (1 - v) + (quad[3][1] * (1 - u) + quad[2][1] * u) * v + bowY * Math.sin(Math.PI * u)) / 480
        });
      }
    }
    const errors = result.contour.map(point => Math.min(...expected.map(reference => Math.hypot(point.x - reference.x, point.y - reference.y))));
    assert.ok(Math.max(...errors) < .015, `curve maximum error ${Math.max(...errors)}`);
    assert.ok(errors.reduce((sum, value) => sum + value, 0) / errors.length < .005);
    result.frameEdges.forEach((edge, side) => {
      assert.deepEqual(edge[0], result.quad[side]);
      assert.deepEqual(edge.at(-1), result.quad[(side + 1) % 4]);
    });
    const bends = result.frameEdges.map(edge => {
      const a = edge[0], b = edge.at(-1), p = edge[Math.floor(edge.length / 2)];
      return Math.abs((b.x - a.x) * (a.y - p.y) - (a.x - p.x) * (b.y - a.y)) / Math.hypot(b.x - a.x, b.y - a.y);
    });
    assert.ok(Math.max(...bends) > .02, 'the overlay must actually bend with the ink');
  }
});

test('keeps measured rails through a partial reflection and validates every new frame', () => {
  const previous = detect(frame());
  const current = detect(frame({ brokenRail: true }), previous);
  assert.equal(current?.source, 'yellow');
  assert.ok(current.frameEdgeSupport.some(support => support < .95));
  assert.ok(current.frameEdgeSupport.filter(support => support > .7).length >= 3);
  assert.equal(detect(frame({ paper: false, background: [244, 244, 240] }), previous), null);
});

test('does not confuse the edge of a white sheet on a warm desk with yellow printed ink', () => {
  for (const background of [[149, 113, 81], [184, 156, 113], [240, 218, 75]]) {
    const result = detect(frame({ yellow: false, background }));
    assert.ok(!result || result.source === 'paper', JSON.stringify(result && { source: result.source, confidence: result.confidence }));
  }
});


test('a previous seed follows a moving curved page without freezing its corners', () => {
  const quad = [[100, 25], [348, 34], [375, 455], [73, 447]];
  const options = { quad, bowX: 22, bowY: 12, background: [244, 244, 240] };
  const previous = detect(frame(options));
  assert.equal(previous?.source, 'yellow');
  const moved = quad.map(([x, y]) => [x + 10, y + 7]);
  const current = detect(frame({ ...options, quad: moved, brokenCorner: true, brokenRail: true }), previous);
  assert.equal(current?.source, 'yellow');
  assert.equal(current.tracking, true, 'exercise recovery from the previous frame while the corner is occluded');
  const average = points => points.reduce((sum, point) => ({ x: sum.x + point.x / points.length, y: sum.y + point.y / points.length }), { x: 0, y: 0 });
  const before = average(previous.quad), after = average(current.quad);
  assert.ok(Math.abs(after.x - before.x - 10 / 480) < .008, `horizontal motion ${after.x - before.x}`);
  assert.ok(Math.abs(after.y - before.y - 7 / 480) < .008, `vertical motion ${after.y - before.y}`);
  assert.ok(Math.max(...current.quad.map((point, index) => Math.hypot(point.x - previous.quad[index].x, point.y - previous.quad[index].y))) > .018);
  current.frameEdges.forEach((edge, side) => {
    assert.deepEqual(edge[0], current.quad[side]);
    assert.deepEqual(edge.at(-1), current.quad[(side + 1) % 4]);
  });
  assert.equal(detect(frame({ paper: false, background: [244, 244, 240] }), current), null);
});

test('small changes in noisy ink do not turn a bowed edge into many little folds', () => {
  for (const railPhase of [0, 2.2]) {
    const result = detect(frame({ bowX: 22, bowY: 12, railJitter: .14, railPhase }));
    assert.equal(result?.source, 'yellow');
    assert.ok(result.confidence >= .78);
    const changes = result.frameEdges.flatMap(edge => edge.slice(1, -1).map((point, index) =>
      Math.hypot(edge[index].x - 2 * point.x + edge[index + 2].x,
        edge[index].y - 2 * point.y + edge[index + 2].y) * 480));
    // The source has a smooth 22px bow plus ~2px sampling/ink perturbations.
    // Those perturbations must not become sharp visible bends in the overlay.
    assert.ok(Math.max(...changes) < .65, `local bend ${Math.max(...changes)}px`);
    assert.ok(changes.reduce((sum, value) => sum + value, 0) / changes.length < .2);
    const bow = result.frameEdges.map(edge => {
      const a = edge[0], b = edge.at(-1), center = edge[Math.floor(edge.length / 2)];
      return Math.abs((b.x - a.x) * (a.y - center.y) - (a.x - center.x) * (b.y - a.y)) / Math.hypot(b.x - a.x, b.y - a.y) * 480;
    });
    assert.ok(Math.max(...bow) > 12, 'keep the real broad paper curl');
  }
});

function illuminated(image, gains, { gradient = false, shadow = false } = {}) {
  const data = new Uint8ClampedArray(image.data.length);
  for (let y = 0; y < image.height; y += 1) {
    for (let x = 0; x < image.width; x += 1) {
      const at = (y * image.width + x) * 4;
      const shade = shadow ? .62 + .38 * x / image.width : 1;
      for (let channel = 0; channel < 3; channel += 1) {
        const gain = gains[channel] + (gradient ? (1 - gains[channel]) * .35 * y / image.height : 0);
        data[at + channel] = image.data[at + channel] * gain * shade;
      }
      data[at + 3] = 255;
    }
  }
  return { ...image, data };
}

test('measures faint yellow rails relative to the paper under cool, warm and mixed illumination', () => {
  const image = frame({ ink: [225, 218, 174], bowX: 16, bowY: -11 });
  const reference = detect(image);
  for (const [name, gains, options] of [
    ['blue camera white balance', [.54, .75, 1], {}],
    ['warm lamp', [1, .78, .52], {}],
    ['green cast', [.72, 1, .76], {}],
    ['magenta cast', [1, .72, .94], {}],
    ['cool gradient and shadow', [.54, .75, 1], { gradient: true, shadow: true }]
  ]) {
    const input = illuminated(image, gains, options), original = input.data.slice();
    const result = detect(input);
    assert.equal(result?.source, 'yellow', name);
    assert.ok(result.confidence > .88, `${name}: confidence ${result.confidence}`);
    assert.equal(result.markers.length, 2, name);
    assert.ok(result.markerSupport >= .65, `${name}: markers ${result.markerSupport}`);
    const movement = Math.max(...result.quad.map((point, index) => Math.hypot(point.x - reference.quad[index].x, point.y - reference.quad[index].y)));
    assert.ok(movement < .009, `${name}: illumination moved the corners by ${movement}`);
    assert.deepEqual(input.data, original, 'illumination analysis must not alter the captured photo');
  }
});

test('illumination normalization preserves neutral data and rejects coloured desks and neutral printing', () => {
  const neutral = frame();
  assert.equal(normalizeIllumination(neutral), neutral, 'ordinary neutral scenes retain the exact established geometry input');
  for (const gains of [[.54, .75, 1], [1, .78, .52]]) {
    const unverified = detect(illuminated(frame({ markers: false }), gains));
    assert.equal(unverified?.source, 'yellow', 'a measurable outline can remain visible');
    assert.ok(unverified.confidence <= .76, 'cast-coloured material needs the calibration strips before auto capture');
    assert.ok(unverified.illumination);
    for (const options of [
      { yellow: false },
      { ink: [175, 175, 175] },
      { paper: false, background: [240, 218, 75] },
      { paper: false, background: [114, 177, 230] }
    ]) {
      const result = detect(illuminated(frame(options), gains));
      assert.ok(!result || result.source === 'paper', JSON.stringify({ gains, options, result }));
      assert.ok(!result || result.confidence < .78);
    }
  }
});

test('full-resolution illumination normalization yields, cancels and matches detector pixels exactly', async () => {
  const original = illuminated(frame(), [.54, .75, 1]), snapshot = original.data.slice();
  const synchronous = normalizeIllumination(original);
  let yielded = false;
  setTimeout(() => { yielded = true; }, 0);
  const asynchronous = await normalizeIlluminationAsync(original);
  assert.ok(yielded, 'processing must yield to pending UI work');
  assert.deepEqual(asynchronous.data, synchronous.data);
  assert.deepEqual(asynchronous.illumination, synchronous.illumination);
  assert.deepEqual(original.data, snapshot);
  const controller = new AbortController();
  const cancelled = normalizeIlluminationAsync(original, { signal: controller.signal });
  setTimeout(() => controller.abort(), 0);
  await assert.rejects(cancelled, error => error.name === 'AbortError');
  assert.deepEqual(original.data, snapshot, 'cancelling must retain the original camera photo');
  const neutral = frame();
  assert.equal(await normalizeIlluminationAsync(neutral), neutral);
  await assert.rejects(normalizeIlluminationAsync(neutral, { signal: controller.signal }), error => error.name === 'AbortError');
});

test('a larger pastel desk cannot recolour a smaller correctly exposed white sheet', () => {
  const base = [[100, 25], [348, 34], [375, 455], [73, 447]];
  for (const scale of [.9, .7]) {
    const quad = base.map(([x, y]) => [240 + (x - 240) * scale, 240 + (y - 240) * scale]);
    for (const background of [[114, 177, 230], [180, 210, 245], [220, 180, 230]]) {
      const source = frame({ quad, background }), detection = detect(source);
      assert.equal(normalizeIllumination(source), source, 'the brighter neutral page must outweigh the larger coloured desk');
      assert.equal(detection?.source, 'yellow', JSON.stringify({ scale, background }));
      assert.ok(detection.confidence > .9, JSON.stringify({ scale, background, confidence: detection.confidence }));
      assert.equal(detection.markers.length, 2);
    }
  }
});

test('rectified-page illumination uses the physical interior rather than synthetic white margins', async () => {
  const source = illuminated(frame({ background: [244, 244, 240] }), [.54, .75, 1]);
  for (let y = 0; y < source.height; y += 1) {
    for (let x = 0; x < source.width; x += 1) {
      if (x >= source.width * .1 && x <= source.width * .9 && y >= source.height * .06 && y <= source.height * .92) continue;
      const at = (y * source.width + x) * 4;
      source.data[at] = source.data[at + 1] = source.data[at + 2] = 255;
    }
  }
  assert.equal(normalizeIllumination(source), source, 'scene-wide neutral material remains a valid live reference');
  const options = { region: { x0: .12, y0: .08, x1: .88, y1: .90 } };
  const corrected = normalizeIllumination(source, options);
  assert.ok(corrected.illumination, 'the known page interior provides its own illuminant');
  const at = (Math.round(source.height * .45) * source.width + Math.round(source.width * .5)) * 4;
  assert.ok(Math.abs(corrected.data[at] - corrected.data[at + 2]) < 6, 'the page is neutral despite the artificial white border');
  assert.deepEqual((await normalizeIlluminationAsync(source, options)).data, corrected.data);
});

function locallyIlluminatedPage(gains, { offset = 0, colourMarkers = true, yellow = true } = {}) {
  const source = frame({ colourMarkers, yellow, ink: [225, 218, 174], bowX: 16, bowY: -11 });
  for (let at = 0; at < source.data.length; at += 4) {
    const desk = source.data[at] === 68 && source.data[at + 1] === 78 && source.data[at + 2] === 83;
    for (let channel = 0; channel < 3; channel += 1) source.data[at + channel] = desk ? 235
      : source.data[at + channel] * gains[channel] + offset;
  }
  return source;
}

test('four ordered colour circles select the page illuminant independently of a brighter neutral desk', () => {
  for (const [name, gains] of [['cool', [.42, .65, 1]], ['warm', [1, .62, .32]],
    ['green', [.4, 1, .55]], ['magenta', [1, .45, .86]]]) {
    const source = locallyIlluminatedPage(gains), snapshot = source.data.slice();
    const refs = context.window.ScannerPro.LiveDetection.findColourReferences(source);
    assert.ok(refs, name);
    assert.equal(refs.points.length, 4);
    assert.ok(refs.paperSupport >= .67, name);
    assert.ok(refs.points.every(point => point.y > 400 && point.x > 210 && point.x < 240), name);
    const normalized = normalizeIllumination(source);
    assert.equal(normalized.illumination?.reference, 'colour-markers', name);
    assert.equal(normalized.luminanceData, source.data, 'focus and exposure retain original camera intensities');
    const result = detect(source);
    assert.equal(result?.source, 'yellow', name);
    assert.ok(result.confidence > .86, `${name}: ${result.confidence}`);
    assert.ok(result.markerSupport >= .65, `${name}: bottom boxes ${result.markerSupport}`);
    assert.deepEqual(source.data, snapshot, 'the raw photograph must remain unchanged');
  }
});

test('verified colour references retain the frame in low exposure without manufacturing capture quality', () => {
  for (const gains of [[.28, .28, .28], [.33, .24, .16]]) {
    const source = locallyIlluminatedPage(gains), result = detect(source);
    assert.equal(result?.source, 'yellow');
    assert.ok(result.brightness < 75, `original low brightness ${result.brightness}`);
    assert.ok(result.confidence <= .76, 'better colour evidence cannot authorize a dim automatic capture');
    assert.ok(result.frameEdgeSupport.filter(support => support >= .7).length >= 3);
  }
});

test('bright paper with a veiling reflection keeps independently measured rails', () => {
  const source = locallyIlluminatedPage([.9, .9, .9], { offset: 55 });
  const result = detect(source);
  assert.equal(result?.source, 'yellow');
  assert.ok(result.frameEdgeSupport.filter(support => support >= .7).length >= 3);
  assert.ok(result.confidence > .78);
});

test('missing, reordered or occluded colour circles cannot become an illuminant reference', () => {
  const red = [255, 0, 0], black = [0, 0, 0], blue = [0, 0, 255], green = [110, 255, 18], white = [244, 244, 240];
  for (const colourMarkers of [false, [red, black, green, blue], [red, white, blue, green], [red, black, white, green]]) {
    assert.equal(context.window.ScannerPro.LiveDetection.findColourReferences(frame({ colourMarkers })), null);
  }
  const noRails = locallyIlluminatedPage([.42, .65, 1], { yellow: false });
  assert.ok(context.window.ScannerPro.LiveDetection.findColourReferences(noRails));
  const result = detect(noRails);
  assert.ok(!result || result.source === 'paper');
  assert.ok(!result || result.confidence < .78, 'four colours alone are not a document frame');
});

test('a small ordered row of laptop icons at the image edge is not page calibration', () => {
  const width = 360, height = 480, data = new Uint8ClampedArray(width * height * 4);
  for (let at = 0; at < data.length; at += 4) { data[at] = 124; data[at + 1] = 90; data[at + 2] = 104; data[at + 3] = 255; }
  const colours = [[255, 0, 0], [0, 0, 0], [0, 0, 255], [110, 255, 18]];
  for (let marker = 0; marker < 4; marker += 1) {
    const cx = 355 - marker * 1.4, cy = 95 + marker * 3.8;
    for (let y = 90; y < 112; y += 1) for (let x = 344; x < 360; x += 1) {
      if (Math.hypot(x - cx, y - cy) > 1.7) continue;
      const at = (y * width + x) * 4;
      for (let channel = 0; channel < 3; channel += 1) data[at + channel] = colours[marker][channel];
    }
  }
  assert.equal(context.window.ScannerPro.LiveDetection.findColourReferences({ width, height, data }), null,
    'red-to-green orientation would place the actual sheet beyond the right image edge');
  assert.equal(detect({ width, height, data }), null);
});

test('identifies the printed layout when the rails are grey or shifted away from yellow', () => {
  for (const ink of [[211,214,215],[224,219,229],[212,228,228],[238,239,238]]) {
    for(const bowX of [0,18]) {
      const photo=frame({ink,colourMarkers:true,bowX});
      const result=detect(photo);
      assert.equal(result?.source,'yellow',JSON.stringify({ink,bowX,result}));
      assert.equal(result.detectionMode,'layout');
      assert.ok(result.confidence>.86,JSON.stringify({ink,bowX,confidence:result.confidence}));
      assert.equal(result.markers.length,2);
      assert.ok(result.markerSupport>=.65);
      assert.ok(Math.min(...result.frameEdgeSupport)>.6);
      const moved=detect(frame({ink,colourMarkers:true,bowX,quad:[[105,29],[353,38],[380,459],[78,451]]}),result);
      assert.equal(moved?.detectionMode,'layout');
      assert.ok(moved.confidence>.86);
      assert.equal(detect(frame({yellow:false,colourMarkers:true}),result)?.source,'paper');
    }
  }
});

test('layout recovery requires the measured box strips and matching four-circle footer', () => {
  for (const options of [
    {colourMarkers:false},{markers:false},{yellow:false},
    {colourMarkers:[[0,0,255],[0,0,0],[255,0,0],[110,255,18]]}
  ]) {
    const result=detect(frame({ink:[215,215,215],colourMarkers:true,...options}));
    assert.ok(!result||result.source==='paper'||result.confidence<.78,JSON.stringify(result));
  }
});
