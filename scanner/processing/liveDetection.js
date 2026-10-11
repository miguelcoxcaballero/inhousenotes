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
      const ink = sample(state.chroma, state, p);
      const contrast = ink - (sample(state.chroma, state, before) + sample(state.chroma, state, after)) * .5;
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

  // Trace the printed ink, allowing a smooth curved surface instead of fitting
  // all samples to a straight chord. Dynamic programming prevents a single
  // yellow object, a grid intersection or a reflection from pulling a rail.
  function traceEdge(state, start, end, radius, samples = 33, guide = null, outer = false, anchorCorners = true) {
    const span = distance(start, end);
    const normal = { x: -(end.y - start.y) / span, y: (end.x - start.x) / span };
    const reach = Math.max(2, Math.ceil(radius)), bands = reach * 2 + 1;
    const evidence = new Float32Array(samples * bands);
    const narrow = new Uint8Array(samples * bands);
    const back = new Int16Array(samples * bands);
    let previous = new Float32Array(bands);
    const gap = 3;
    for (let row = 0; row < samples; row += 1) {
      const center = guide ? guide[row] : mix(start, end, row / (samples - 1));
      const scores = new Float32Array(bands);
      for (let band = 0; band < bands; band += 1) {
        const offset = band - reach;
        const point = { x: center.x + normal.x * offset, y: center.y + normal.y * offset };
        let contrast = 0;
        if (sample(state.yellow, state, point)) {
          const ink = sample(state.chroma, state, point);
          for (const separation of [gap, gap + 3]) {
            const before = sample(state.chroma, state, { x: point.x - normal.x * separation, y: point.y - normal.y * separation });
            const after = sample(state.chroma, state, { x: point.x + normal.x * separation, y: point.y + normal.y * separation });
            contrast = Math.max(contrast, ink - (before + after) * .5);
            if (separation === 6 && Math.min(ink - before, ink - after) > 1) narrow[row * bands + band] = 1;
          }
        }
        evidence[row * bands + band] = Math.max(0, contrast);
        const signal = Math.min(3, Math.max(0, contrast) / 12);
        // Use the chord as a weak prior, not as a constraint on bent paper.
        const observation = signal - Math.abs(offset) / reach * .16 - (outer ? offset * .015 : 0);
        let best = 0, parent = band;
        if (row) {
          best = -Infinity;
          for (let old = Math.max(0, band - 4); old <= Math.min(bands - 1, band + 4); old += 1) {
            const value = previous[old] - Math.abs(old - band) * .14;
            if (value > best) { best = value; parent = old; }
          }
        }
        scores[band] = observation + best;
        back[row * bands + band] = parent;
      }
      previous = scores;
    }
    let band = 0;
    for (let i = 1; i < bands; i += 1) if (previous[i] > previous[band]) band = i;
    const offsets = new Float32Array(samples), strengths = new Float32Array(samples);
    let narrowHits = 0;
    for (let row = samples - 1; row >= 0; row -= 1) {
      if (row > 0 && row < samples - 1 && evidence[row * bands + band] > 5) narrowHits += narrow[row * bands + band];
      offsets[row] = band - reach;
      strengths[row] = evidence[row * bands + band];
      band = back[row * bands + band];
    }
    // Fill short gaps from observations on both sides. No unseen corner or
    // frame is accepted: detection still requires independent rail coverage.
    const points = [];
    let hits = 0;
    for (let row = 0; row < samples; row += 1) {
      const lo = Math.max(0, row - 1), hi = Math.min(samples - 1, row + 1);
      let offset = 0;
      for (let index = lo; index <= hi; index += 1) offset += offsets[index];
      offset /= hi - lo + 1;
      if (outer && anchorCorners) offset *= Math.min(1, row / 2, (samples - 1 - row) / 2);
      const center = guide ? guide[row] : mix(start, end, row / (samples - 1));
      points.push({ x: center.x + normal.x * offset, y: center.y + normal.y * offset });
      if (row > 0 && row < samples - 1 && strengths[row] > 5) hits += 1;
    }
    if (outer && anchorCorners) { points[0] = start; points[points.length - 1] = end; }
    return {
      points, normal, offsets, strengths,
      support: hits / (samples - 2), narrowSupport: narrowHits / (samples - 2),
      cornerOffset: (Math.abs(offsets[0]) + Math.abs(offsets[1])
        + Math.abs(offsets[samples - 2]) + Math.abs(offsets[samples - 1])) / 4
    };
  }

  function trackedCorners(quad, traces, radius) {
    // The previous contour is only a search guide. Fit the movement of the
    // CURRENT ink near each end, then intersect the two adjacent displacement
    // constraints. Retaining the old corners would hide motion from capture.
    const endpoint = (trace, end) => {
      const { offsets, strengths } = trace;
      const from = end ? offsets.length - 10 : 2, to = end ? offsets.length - 3 : 9;
      let count = 0, sx = 0, sy = 0, sxx = 0, sxy = 0;
      for (let index = from; index <= to; index += 1) {
        if (strengths[index] <= 5) continue;
        const x = end ? index - offsets.length + 1 : index, y = offsets[index];
        count += 1; sx += x; sy += y; sxx += x * x; sxy += x * y;
      }
      const denominator = count * sxx - sx * sx;
      return count >= 3 && denominator > 0 ? (sy * sxx - sx * sxy) / denominator : null;
    };
    const result = [];
    for (let index = 0; index < 4; index += 1) {
      const incoming = traces[(index + 3) % 4], outgoing = traces[index];
      const first = endpoint(incoming, true), second = endpoint(outgoing, false);
      if (first === null || second === null) return null;
      const a = incoming.normal, b = outgoing.normal, determinant = a.x * b.y - a.y * b.x;
      if (Math.abs(determinant) < .2) return null;
      const dx = (first * b.y - a.y * second) / determinant;
      const dy = (a.x * second - first * b.x) / determinant;
      if (Math.hypot(dx, dy) > radius * 1.5) return null;
      result.push({ x: quad[index].x + dx, y: quad[index].y + dy });
    }
    return result;
  }

  const along = (points, t) => {
    const position = clamp(t, 0, 1) * (points.length - 1), index = Math.floor(position);
    return mix(points[index], points[Math.min(points.length - 1, index + 1)], position - index);
  };

  function surfacePoint(quad, edges, u, v) {
    if (!edges) return project(quad, u, v);
    const top = along(edges[0], u), right = along(edges[1], v);
    const bottom = along(edges[2], 1 - u), left = along(edges[3], 1 - v);
    const base = project(quad, u, v);
    return {
      x: top.x * (1 - v) + bottom.x * v + left.x * (1 - u) + right.x * u - base.x,
      y: top.y * (1 - v) + bottom.y * v + left.y * (1 - u) + right.y * u - base.y
    };
  }

  function calibration(state, frame, frameEdges) {
    let best = { support: 0, markers: [], markerEdges: [], rotation: 0 };
    const radius = Math.max(2, Math.min(state.width, state.height) * .009);
    for (let orientation = 0; orientation < 4; orientation += 1) {
      const oriented = rotate(frame, orientation);
      const edges = frameEdges && rotate(frameEdges, orientation);
      const measured = [[.5 / 18, 8.5 / 18], [9.5 / 18, 17.5 / 18]].map(([x0, x1]) => {
        const top = [surfacePoint(oriented, edges, x0, 26 / 27), surfacePoint(oriented, edges, x1, 26 / 27)];
        const bottom = [surfacePoint(oriented, edges, x0, 26.5 / 27), surfacePoint(oriented, edges, x1, 26.5 / 27)];
        const height = distance(mix(top[0], top[1], .5), mix(bottom[0], bottom[1], .5));
        const reach = Math.min(radius, Math.max(1, height * .38));
        const topGuide = Array.from({ length: 17 }, (_, index) => surfacePoint(oriented, edges, x0 + (x1 - x0) * index / 16, 26 / 27));
        const bottomGuide = Array.from({ length: 17 }, (_, index) => surfacePoint(oriented, edges, x0 + (x1 - x0) * index / 16, 26.5 / 27));
        const first = traceEdge(state, top[0], top[1], reach, 17, topGuide);
        const second = traceEdge(state, bottom[0], bottom[1], reach, 17, bottomGuide);
        const separation = distance(along(first.points, .5), along(second.points, .5));
        const expectedSeparation = distance(mix(top[0], top[1], .5), mix(bottom[0], bottom[1], .5));
        // One rail cannot count twice as the two edges of a calibration strip.
        const distinct = separation >= Math.max(1.3, expectedSeparation * .45);
        return {
          support: distinct ? Math.min(first.support, second.support) : 0,
          box: [first.points[0], first.points.at(-1), second.points.at(-1), second.points[0]],
          edges: [first.points, [first.points.at(-1), second.points.at(-1)], second.points.slice().reverse(), [second.points[0], first.points[0]]]
        };
      });
      const support = Math.min(...measured.map(item => item.support));
      if (support > best.support) best = {
        support, rotation: orientation,
        markers: support >= .5 ? measured.map(item => item.box) : [],
        markerEdges: support >= .5 ? measured.map(item => item.edges) : []
      };
    }
    return best;
  }

  function railSharpness(state, edges) {
    let sum = 0, squares = 0, count = 0;
    for (const edge of edges || []) {
      for (let i = 2; i < edge.length - 2; i += 1) {
        const point = edge[i];
        const x = clamp(Math.round(point.x), 2, state.width - 3);
        const y = clamp(Math.round(point.y), 2, state.height - 3);
        // Focus on the observed printed border as well as the sparse interior.
        // Blank/dotted paper must not be judged blurrier just for lacking text.
        for (let dy = -1; dy <= 1; dy += 1) {
          for (let dx = -1; dx <= 1; dx += 1) {
            const at = (y + dy) * state.width + x + dx;
            const lap = 4 * state.luminance[at] - state.luminance[at - 1] - state.luminance[at + 1]
              - state.luminance[at - state.width] - state.luminance[at + state.width];
            sum += lap; squares += lap * lap; count += 1;
          }
        }
      }
    }
    return count ? Math.max(0, squares / count - (sum / count) ** 2) : 0;
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

  function detect(imageData, previous = null) {
    if (!imageData || !Number.isInteger(imageData.width) || !Number.isInteger(imageData.height)
      || imageData.width < 40 || imageData.height < 40
      || !imageData.data || imageData.data.length !== imageData.width * imageData.height * 4) return null;
    const state = analyse(imageData), { width, height } = state;
    // A sheet can bow several pixels between its corners. Search around the
    // chord, but still require a narrow colour ridge at every accepted sample.
    const radius = Math.max(3, Math.min(width, height) * .065);
    const candidates = [];
    if (previous?.source === 'yellow' && previous.quad?.length === 4) {
      const quad = previous.quad.map(point => ({ x: point.x * width, y: point.y * height }));
      if (plausible(quad, width, height)) {
        const guides = previous.frameEdges?.length === 4 && previous.frameEdges.every(edge => edge.length === 33
          && edge.every(point => Number.isFinite(point.x) && Number.isFinite(point.y)))
          ? previous.frameEdges.map(edge => edge.map(point => ({ x: point.x * width, y: point.y * height }))) : null;
        candidates.push({ quad, guides, source: 'yellow', tracking: true });
      }
    }
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
      let quad = candidate.quad;
      const source = candidate.source;
      if (!plausible(quad, width, height)) continue;
      const quality = paperEvidence(state, quad);
      if (quality.neutral < .64 || quality.yellow > .2 || quality.brightness < 75) continue;
      let frameEdges = null, frameEdgeSupport = [], markerEdges = [];
      let edgeSupport = 0, geometryPenalty = 0;
      let confidence, markers = [], markerSupport = 0, pageQuad = candidate.paper || quad;
      if (source === 'yellow') {
        // A quick wide search rejects irrelevant contours before the more
        // expensive continuous trace. Missing ink is allowed on one side.
        const preliminary = edgeEvidence(state, quad, radius);
        if (preliminary.filter(value => value >= .4).length < 3 || Math.max(...preliminary) < .7) continue;
        let traces = quad.map((point, side) => traceEdge(state, point, quad[(side + 1) % 4], radius, 33,
          candidate.guides?.[side] || null, true, !candidate.tracking));
        if (candidate.tracking) {
          const currentQuad = trackedCorners(quad, traces, radius);
          if (!plausible(currentQuad, width, height)) continue;
          quad = currentQuad;
          // Resample against the measured corners so the curves and calibration
          // coordinates use the current surface, rather than old endpoints.
          traces = quad.map((point, side) => traceEdge(state, point, quad[(side + 1) % 4], radius, 33, null, true));
        }
        if (traces.filter(trace => trace.narrowSupport >= .4).length < 3) continue;
        frameEdges = traces.map(trace => trace.points);
        geometryPenalty = traces.reduce((sum, trace) => sum + trace.cornerOffset, 0) / (4 * Math.min(width, height));
        if (geometryPenalty > .025) continue;
        frameEdgeSupport = traces.map(trace => trace.support);
        edgeSupport = frameEdgeSupport.reduce((sum, value) => sum + value, 0) / 4;
        if (edgeSupport < (candidate.tracking ? .5 : .59)
          || frameEdgeSupport.filter(value => value >= .5).length < 3) continue;
        const calibrationResult = calibration(state, quad, frameEdges);
        markers = calibrationResult.markers;
        markerEdges = calibrationResult.markerEdges;
        markerSupport = calibrationResult.support;
        const oriented = rotate(quad, calibrationResult.rotation);
        // Use a visible sheet perimeter where available. Otherwise extrapolate
        // the known printed A4 inset for capture, keeping the overlay on the ink.
        if (!candidate.paper) pageQuad = rectangle(oriented, -1.5 / 18, -1.43 / 27, 19.5 / 18, 28.27 / 27);
        confidence = clamp(.49 + edgeSupport * .31 + markerSupport * .13 + quality.neutral * .05, 0, .98);
        if (Math.min(...frameEdgeSupport) < .2 && markerSupport < .5) confidence = Math.min(confidence, .76);
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
      const score = confidence + Math.min(.04, coverage * .06) - geometryPenalty * 2;
      if (best && best.score >= score) continue;
      const normalize = points => points.map(p => ({ x: clamp(p.x / width, 0, 1), y: clamp(p.y / height, 0, 1) }));
      const orderedQuad = topLeftFirst(quad);
      const start = quad.indexOf(orderedQuad[0]);
      if (frameEdges) frameEdges = rotate(frameEdges, start);
      if (frameEdgeSupport.length) frameEdgeSupport = rotate(frameEdgeSupport, start);
      best = {
        score, quad: normalize(orderedQuad), pageQuad: normalize(topLeftFirst(pageQuad)),
        markers: markers.map(normalize), markerEdges: markerEdges.map(edges => edges.map(normalize)), confidence, source,
        frameEdges: frameEdges ? frameEdges.map(normalize) : null,
        contour: frameEdges ? normalize(frameEdges.flat()) : normalize(quad),
        frameEdgeSupport, tracking: !!candidate.tracking,
        sharpness: quality.sharpness, edgeSharpness: railSharpness(state, frameEdges), brightness: quality.brightness,
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
