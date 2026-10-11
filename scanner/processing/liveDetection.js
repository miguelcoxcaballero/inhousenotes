(() => {
  'use strict';

  const SP = (window.ScannerPro = window.ScannerPro || {});
  const clamp = (value, low, high) => Math.max(low, Math.min(high, value));
  const cross = (a, b, c) => (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
  const distance = (a, b) => Math.hypot(a.x - b.x, a.y - b.y);
  const area = quad => Math.abs(quad.reduce((sum, point, i) => {
    const next = quad[(i + 1) % quad.length];
    return sum + point.x * next.y - next.x * point.y;
  }, 0)) / 2;
  const mix = (a, b, t) => ({ x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t });
  const project = (quad, u, v) => mix(mix(quad[0], quad[1], u), mix(quad[3], quad[2], u), v);
  const rotate = (quad, count) => quad.map((_, i) => quad[(i + count) % 4]);
  const rectangle = (quad, x0, y0, x1, y1) => [
    project(quad, x0, y0), project(quad, x1, y0),
    project(quad, x1, y1), project(quad, x0, y1)
  ];
  const topLeftFirst = quad => {
    let start = 0;
    for (let i = 1; i < quad.length; i += 1) {
      if (quad[i].x + quad[i].y < quad[start].x + quad[start].y) start = i;
    }
    return rotate(quad, start);
  };

  function hull(points) {
    if (points.length < 4) return [];
    points.sort((a, b) => a.x - b.x || a.y - b.y);
    const lower = [], upper = [];
    for (const point of points) {
      while (lower.length > 1 && cross(lower[lower.length - 2], lower[lower.length - 1], point) <= 0) lower.pop();
      lower.push(point);
    }
    for (let i = points.length - 1; i >= 0; i -= 1) {
      const point = points[i];
      while (upper.length > 1 && cross(upper[upper.length - 2], upper[upper.length - 1], point) <= 0) upper.pop();
      upper.push(point);
    }
    return lower.slice(0, -1).concat(upper.slice(0, -1));
  }

  function corners(points) {
    const result = hull(points);
    // Preserve the four actual extrema of the contour, rather than imposing
    // an axis-aligned box that would drift during handheld perspective changes.
    while (result.length > 4) {
      let smallest = Infinity, remove = 0;
      for (let i = 0; i < result.length; i += 1) {
        const triangle = Math.abs(cross(result[(i + result.length - 1) % result.length], result[i], result[(i + 1) % result.length]));
        if (triangle < smallest) { smallest = triangle; remove = i; }
      }
      result.splice(remove, 1);
    }
    if (result.length !== 4) return null;
    return topLeftFirst(result);
  }

  function plausible(quad, width, height) {
    if (!quad || quad.some(p => !Number.isFinite(p.x) || !Number.isFinite(p.y)
      || p.x < 2 || p.y < 2 || p.x > width - 3 || p.y > height - 3)) return false;
    const fraction = area(quad) / (width * height);
    if (fraction < .12 || fraction > .93) return false;
    const lengths = quad.map((p, i) => distance(p, quad[(i + 1) % 4]));
    if (Math.min(...lengths) < Math.min(width, height) * .18) return false;
    for (let i = 0; i < 4; i += 1) {
      const sine = cross(quad[i], quad[(i + 1) % 4], quad[(i + 2) % 4]) / (lengths[i] * lengths[(i + 1) % 4]);
      if (sine < .38) return false;
    }
    if (Math.min(lengths[0], lengths[2]) / Math.max(lengths[0], lengths[2]) < .4
      || Math.min(lengths[1], lengths[3]) / Math.max(lengths[1], lengths[3]) < .4) return false;
    const ratio = (lengths[0] + lengths[2]) / (lengths[1] + lengths[3]);
    return ratio > .38 && ratio < 2.6;
  }

  function components(mask, width, height, minimum, original = mask) {
    const visited = new Uint8Array(mask.length);
    const queue = new Int32Array(mask.length);
    const groups = [];
    for (let seed = 0; seed < mask.length; seed += 1) {
      if (!mask[seed] || visited[seed]) continue;
      let read = 0, write = 1, count = 0;
      queue[0] = seed; visited[seed] = 1;
      const points = [];
      while (read < write) {
        const at = queue[read++], x = at % width, y = Math.floor(at / width);
        count += 1;
        if (original[at] && (x === 0 || y === 0 || x === width - 1 || y === height - 1
          || !original[at - 1] || !original[at + 1] || !original[at - width] || !original[at + width])) points.push({ x, y });
        if (x > 0 && mask[at - 1] && !visited[at - 1]) { visited[at - 1] = 1; queue[write++] = at - 1; }
        if (x < width - 1 && mask[at + 1] && !visited[at + 1]) { visited[at + 1] = 1; queue[write++] = at + 1; }
        if (y > 0 && mask[at - width] && !visited[at - width]) { visited[at - width] = 1; queue[write++] = at - width; }
        if (y < height - 1 && mask[at + width] && !visited[at + width]) { visited[at + width] = 1; queue[write++] = at + width; }
      }
      if (count >= minimum && points.length >= 4) groups.push({ count, points });
    }
    return groups.sort((a, b) => b.count - a.count).slice(0, 5);
  }

  function dilate(mask, width, height) {
    const result = new Uint8Array(mask.length);
    for (let y = 1; y < height - 1; y += 1) {
      for (let x = 1; x < width - 1; x += 1) {
        const at = y * width + x;
        if (!mask[at]) continue;
        for (let dy = -1; dy <= 1; dy += 1) {
          result[at + dy * width - 1] = result[at + dy * width] = result[at + dy * width + 1] = 1;
        }
      }
    }
    return result;
  }

  function analyse(imageData) {
    const { width, height, data } = imageData;
    const length = width * height;
    const yellow = new Uint8Array(length), neutral = new Uint8Array(length);
    const luminance = new Float32Array(length), chroma = new Float32Array(length);
    const histogram = new Uint32Array(256);
    let yellowCount = 0;
    for (let i = 0; i < length; i += 1) {
      const offset = i * 4, r = data[offset], g = data[offset + 1], b = data[offset + 2];
      const maximum = Math.max(r, g, b), minimum = Math.min(r, g, b);
      const saturation = maximum ? (maximum - minimum) / maximum : 0;
      const light = r * .2126 + g * .7152 + b * .0722;
      luminance[i] = light;
      histogram[Math.round(light)] += 1;
      chroma[i] = (r + g) * .5 - b - Math.abs(r - g) * .34;
      neutral[i] = saturation < .28 ? 1 : 0;
      // A broad hue gate retains the thin, antialiased printed rails. The
      // later two-sided ridge test rejects yellow desks and filled objects.
      if (maximum > 65 && g >= r * .70 && r >= g * .82 && b < g * .94
        && chroma[i] > 10 && saturation > .055) { yellow[i] = 1; yellowCount += 1; }
    }
    let cumulative = 0, bright = 255;
    for (let i = 0; i < 256; i += 1) {
      cumulative += histogram[i];
      if (cumulative >= length * .84) { bright = i; break; }
    }
    return { width, height, yellow, neutral, luminance, chroma, yellowCount, paperThreshold: Math.max(95, bright - 38) };
  }

  const sample = (map, state, point) => map[
    clamp(Math.round(point.y), 0, state.height - 1) * state.width + clamp(Math.round(point.x), 0, state.width - 1)
  ];

  function ridge(state, point, normal, radius) {
    let best = 0;
    const gap = Math.max(3, radius * .8);
    for (let offset = -radius; offset <= radius; offset += 1) {
      const p = { x: point.x + normal.x * offset, y: point.y + normal.y * offset };
      if (!sample(state.yellow, state, p)) continue;
      const before = { x: p.x - normal.x * gap, y: p.y - normal.y * gap };
      const after = { x: p.x + normal.x * gap, y: p.y + normal.y * gap };
      const contrast = sample(state.chroma, state, p)
        - (sample(state.chroma, state, before) + sample(state.chroma, state, after)) * .5;
      if (contrast > best) best = contrast;
    }
    return best;
  }

  function edgeEvidence(state, quad, radius) {
    return quad.map((point, side) => {
      const end = quad[(side + 1) % 4];
      const length = distance(point, end);
      const normal = { x: -(end.y - point.y) / length, y: (end.x - point.x) / length };
      let hits = 0;
      for (let i = 0; i < 32; i += 1) {
        if (ridge(state, mix(point, end, .04 + i / 31 * .92), normal, radius) > 5) hits += 1;
      }
      return hits / 32;
    });
  }

  function paperEvidence(state, quad) {
    let brightness = 0, neutrals = 0, yellow = 0, count = 0, laplacian = 0, laplacianSquared = 0;
    for (let row = 0; row < 32; row += 1) {
      for (let column = 0; column < 24; column += 1) {
        const point = project(quad, .08 + column / 23 * .84, .07 + row / 31 * .86);
        const x = clamp(Math.round(point.x), 1, state.width - 2), y = clamp(Math.round(point.y), 1, state.height - 2);
        const at = y * state.width + x, value = state.luminance[at];
        brightness += value;
        neutrals += state.neutral[at];
        yellow += state.yellow[at];
        const lap = 4 * value - state.luminance[at - 1] - state.luminance[at + 1]
          - state.luminance[at - state.width] - state.luminance[at + state.width];
        laplacian += lap; laplacianSquared += lap * lap;
        count += 1;
      }
    }
    return {
      brightness: brightness / count,
      neutral: neutrals / count,
      yellow: yellow / count,
      sharpness: Math.max(0, laplacianSquared / count - (laplacian / count) ** 2)
    };
  }

  function calibration(state, frame) {
    let best = { support: 0, markers: [], rotation: 0 };
    const radius = Math.max(2, Math.min(state.width, state.height) * .007);
    for (let orientation = 0; orientation < 4; orientation += 1) {
      const oriented = rotate(frame, orientation);
      const boxes = [[.5 / 18, 8.5 / 18], [9.5 / 18, 17.5 / 18]].map(([x0, x1]) =>
        rectangle(oriented, x0, 26 / 27, x1, 26.5 / 27));
      const supports = boxes.map(box => {
        // The two long sides are observable even when the half-centimetre
        // vertical subdivisions become subpixel in a video preview.
        const edges = edgeEvidence(state, box, radius);
        return (edges[0] + edges[2]) * .5;
      });
      const support = Math.min(...supports);
      if (support > best.support) best = {
        support, rotation: orientation,
        markers: support >= .5 ? boxes : []
      };
    }
    return best;
  }

  function paperBoundary(state, quad) {
    const gap = Math.max(3, Math.min(state.width, state.height) * .013);
    let contrast = 0, hits = 0, total = 0;
    for (let side = 0; side < 4; side += 1) {
      const a = quad[side], b = quad[(side + 1) % 4], length = distance(a, b);
      const normal = { x: -(b.y - a.y) / length, y: (b.x - a.x) / length };
      for (let i = 1; i < 16; i += 1) {
        const p = mix(a, b, i / 16);
        const inside = sample(state.luminance, state, { x: p.x + normal.x * gap, y: p.y + normal.y * gap });
        const outside = sample(state.luminance, state, { x: p.x - normal.x * gap, y: p.y - normal.y * gap });
        contrast += Math.max(0, inside - outside);
        hits += inside - outside > 12 ? 1 : 0;
        total += 1;
      }
    }
    return { contrast: contrast / total, support: hits / total };
  }

  function detect(imageData) {
    if (!imageData || !Number.isInteger(imageData.width) || !Number.isInteger(imageData.height)
      || imageData.width < 40 || imageData.height < 40
      || !imageData.data || imageData.data.length !== imageData.width * imageData.height * 4) return null;
    const state = analyse(imageData), { width, height } = state;
    // A sheet can bow several pixels between its corners. Search around the
    // chord, but still require a narrow colour ridge at every accepted sample.
    const radius = Math.max(2, Math.min(width, height) * .035);
    const candidates = [];
    if (state.yellowCount > 60) {
      const groups = components(dilate(state.yellow, width, height), width, height, 55, state.yellow);
      for (const group of groups) candidates.push({ quad: corners(group.points), source: 'yellow' });
      // Disconnected rails can still describe one frame. This candidate must
      // pass the same independent four-side and interior checks as a contour.
      if (groups.length > 1) candidates.push({ quad: corners(groups.flatMap(group => group.points)), source: 'yellow' });
    }
    const paperMask = new Uint8Array(width * height);
    for (const threshold of [state.paperThreshold, Math.min(250, state.paperThreshold + 26)]) {
      for (let i = 0; i < paperMask.length; i += 1) paperMask[i] = state.neutral[i] && state.luminance[i] >= threshold ? 1 : 0;
      for (const group of components(paperMask, width, height, width * height * .12)) {
        const quad = corners(group.points);
        if (!plausible(quad, width, height)) continue;
        candidates.push({ quad, source: 'paper' });
        // A visible paper edge supplies an independent seed for faint/broken
        // yellow frames. Try both page orientations; the calibration row decides.
        for (let orientation = 0; orientation < 2; orientation += 1) {
          const paper = rotate(quad, orientation);
          const frame = rectangle(paper, 1.5 / 21, 1.43 / 29.7, 19.5 / 21, 28.43 / 29.7);
          candidates.push({ quad: frame, source: 'yellow', paper });
        }
      }
    }
    let best = null;
    for (const candidate of candidates) {
      const { quad, source } = candidate;
      if (!plausible(quad, width, height)) continue;
      const quality = paperEvidence(state, quad);
      if (quality.neutral < .64 || quality.yellow > .2 || quality.brightness < 75) continue;
      const edges = edgeEvidence(state, quad, radius);
      const edgeSupport = edges.reduce((sum, value) => sum + value, 0) / 4;
      let confidence, markers = [], markerSupport = 0, pageQuad = candidate.paper || quad;
      if (source === 'yellow') {
        if (Math.min(...edges) < .44 || edgeSupport < .68 || edges.filter(value => value >= .72).length < 3) continue;
        const calibrationResult = calibration(state, quad);
        markers = calibrationResult.markers;
        markerSupport = calibrationResult.support;
        const oriented = rotate(quad, calibrationResult.rotation);
        // Use a visible sheet perimeter where available. Otherwise extrapolate
        // the known printed A4 inset for capture, keeping the overlay on the ink.
        if (!candidate.paper) pageQuad = rectangle(oriented, -1.5 / 18, -1.43 / 27, 19.5 / 18, 28.27 / 27);
        confidence = clamp(.56 + edgeSupport * .25 + markerSupport * .13 + quality.neutral * .04, 0, .98);
        const aspect = (distance(quad[0], quad[1]) + distance(quad[2], quad[3]))
          / (distance(quad[1], quad[2]) + distance(quad[3], quad[0]));
        const aspectError = Math.min(Math.abs(Math.log(aspect / (18 / 27))), Math.abs(Math.log(aspect / (27 / 18))));
        // Perspective may foreshorten A4 into a square, but a generic square
        // yellow object without the printed strip is not capture evidence.
        if (markerSupport < .5) confidence -= clamp((aspectError - .15) * .5, 0, .18);
      } else {
        const boundary = paperBoundary(state, quad);
        if (boundary.support < .65 || boundary.contrast < 15) continue;
        confidence = clamp(.38 + boundary.support * .21 + quality.neutral * .09, 0, .69);
      }
      const coverage = area(quad) / (width * height);
      const score = confidence + Math.min(.04, coverage * .06);
      if (best && best.score >= score) continue;
      const normalize = points => points.map(p => ({ x: clamp(p.x / width, 0, 1), y: clamp(p.y / height, 0, 1) }));
      best = {
        score, quad: normalize(topLeftFirst(quad)), pageQuad: normalize(topLeftFirst(pageQuad)),
        markers: markers.map(normalize), confidence, source,
        sharpness: quality.sharpness, brightness: quality.brightness,
        coverage, edgeSupport, markerSupport
      };
    }
    if (best) delete best.score;
    return best;
  }

  // Intentionally synchronous and independent of OpenCV/full-resolution scan
  // processing. Feed a <=480px ImageData at a throttled preview cadence.
  SP.LiveDetection = { detect };
})();
