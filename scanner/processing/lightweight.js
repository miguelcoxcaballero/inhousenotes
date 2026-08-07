(() => {
  "use strict";

  const SP = (window.ScannerPro = window.ScannerPro || {});
  const Light = (SP.Lightweight = {});
  const A4_RATIO = 21 / 29.7;
  // A 608 px first pass keeps even JPEG-softened rails measurable while
  // reducing the hot per-pixel work by roughly ten percent versus 640 px.
  // Difficult or strongly curled sheets are promoted to the 800 px pass.
  const ANALYSIS_FAST_MAX = 608;
  const ANALYSIS_MAX = 800;
  const OUTPUT_LONG_EDGE = 2546;

  const clamp = (value, min, max) => Math.max(min, Math.min(max, value));
  const yieldToBrowser = () => {
    if (globalThis.scheduler?.yield) return globalThis.scheduler.yield();
    return new Promise(resolve => setTimeout(resolve, 0));
  };

  function createScaledCanvas(source, maxDimension) {
    const scale = Math.min(1, maxDimension / Math.max(source.width, source.height));
    const canvas = document.createElement("canvas");
    canvas.width = Math.max(1, Math.round(source.width * scale));
    canvas.height = Math.max(1, Math.round(source.height * scale));
    const context = canvas.getContext("2d", { willReadFrequently: true });
    context.imageSmoothingEnabled = true;
    context.imageSmoothingQuality = "medium";
    context.drawImage(source, 0, 0, canvas.width, canvas.height);
    return { canvas, context, scale };
  }

  function isYellow(r, g, b) {
    const max = Math.max(r, g, b);
    const min = Math.min(r, g, b);
    if (max < 82 || max === min) return false;
    const saturation = (max - min) / max;
    const hue = max === r
      ? 60 * (((g - b) / (max - min)) % 6)
      : max === g
        ? 60 * (((b - r) / (max - min)) + 2)
        : 60 * (((r - g) / (max - min)) + 4);
    const normalizedHue = hue < 0 ? hue + 360 : hue;
    return normalizedHue >= 39
      && normalizedHue <= 68
      && saturation >= 0.16
      && g / Math.max(1, r) >= 0.70
      && b / Math.max(1, g) <= 0.84
      && (r + g) * 0.5 - b >= 20;
  }

  function yellowChroma(r, g, b) {
    return (r + g) * 0.5 - b - Math.abs(r - g) * 0.34;
  }

  function estimatePaperYellowBaseline(width, height, pixels, paperQuad) {
    if (!paperQuad?.every(Boolean)) return { chroma: 3, saturation: 0.025, luminance: 232 };
    const chromaSamples = [];
    const saturationSamples = [];
    const luminanceSamples = [];
    // The central area is deliberately sampled away from the printed frame
    // and calibration strip. Bright pixels survive handwriting and give us the
    // paper's actual warm/cool cast for this individual photo.
    for (let row = 0; row < 13; row += 1) {
      const v = 0.13 + row / 12 * 0.68;
      for (let column = 0; column < 13; column += 1) {
        const u = 0.17 + column / 12 * 0.66;
        const source = bilinear(paperQuad, u, v);
        const x = clamp(Math.round(source.x), 0, width - 1);
        const y = clamp(Math.round(source.y), 0, height - 1);
        const offset = (y * width + x) * 4;
        const r = pixels[offset], g = pixels[offset + 1], b = pixels[offset + 2];
        const maximum = Math.max(r, g, b);
        const minimum = Math.min(r, g, b);
        const luminance = r * 0.2126 + g * 0.7152 + b * 0.0722;
        const saturation = maximum ? (maximum - minimum) / maximum : 0;
        if (luminance < 92 || saturation > 0.42) continue;
        chromaSamples.push(yellowChroma(r, g, b));
        saturationSamples.push(saturation);
        luminanceSamples.push(luminance);
      }
    }
    if (chromaSamples.length < 18) return { chroma: 3, saturation: 0.025, luminance: 232 };
    chromaSamples.sort((a, b) => a - b);
    saturationSamples.sort((a, b) => a - b);
    luminanceSamples.sort((a, b) => a - b);
    return {
      chroma: chromaSamples[Math.floor(chromaSamples.length * 0.55)],
      saturation: saturationSamples[Math.floor(saturationSamples.length * 0.55)],
      luminance: luminanceSamples[Math.floor(luminanceSamples.length * 0.62)]
    };
  }

  function isFaintYellow(r, g, b, baseline) {
    const maximum = Math.max(r, g, b);
    const minimum = Math.min(r, g, b);
    if (maximum < 54 || maximum === minimum) return false;
    const saturation = (maximum - minimum) / maximum;
    const hue = maximum === r
      ? 60 * (((g - b) / (maximum - minimum)) % 6)
      : maximum === g
        ? 60 * (((b - r) / (maximum - minimum)) + 2)
        : 60 * (((r - g) / (maximum - minimum)) + 4);
    const normalizedHue = hue < 0 ? hue + 360 : hue;
    const opponent = (r + g) * 0.5 - b;
    const balance = Math.abs(r - g);
    return normalizedHue >= 27
      && normalizedHue <= 88
      && saturation >= Math.max(0.032, baseline.saturation - 0.012)
      && yellowChroma(r, g, b) >= baseline.chroma + 3.2
      && opponent >= 6
      && balance <= opponent * 1.45 + 9
      && g / Math.max(1, r) >= 0.58
      && b / Math.max(1, g) <= 0.97;
  }

  function isNeutralStencilTone(r, g, b, baseline) {
    const maximum = Math.max(r, g, b);
    const minimum = Math.min(r, g, b);
    const luminance = r * 0.2126 + g * 0.7152 + b * 0.0722;
    const saturation = maximum ? (maximum - minimum) / maximum : 0;
    // An overexposed yellow rail can retain almost no measurable hue. Only
    // admit a neutral candidate when it is still a little darker than the
    // sheet. Geometry and the two-sided ridge check below provide the actual
    // proof, so grey handwriting elsewhere never enters the stencil mask.
    return luminance >= 48
      && luminance <= baseline.luminance - 3.5
      && saturation <= Math.max(0.24, baseline.saturation + 0.12);
  }

  function stencilReferenceBandAxes(point) {
    if (!point || point.u < -0.08 || point.u > 1.08 || point.v < -0.08 || point.v > 1.08) return false;
    const frameU0 = 1.5 / 21;
    const frameU1 = 19.5 / 21;
    const frameV0 = 1.43 / 29.7;
    const frameV1 = 28.43 / 29.7;
    const stripV = 27.68 / 29.7;
    // Try all rotations because the calibration strip may be on any image
    // edge. The relaxed colour predicate is never used away from these narrow
    // geometric bands, so desks and neighbouring yellow sheets cannot flood
    // the mask.
    for (let rotation = 0; rotation < 4; rotation += 1) {
      const oriented = rotateNormalized(point, rotation);
      const onVerticalFrame = (Math.abs(oriented.u - frameU0) <= 0.052
          || Math.abs(oriented.u - frameU1) <= 0.052)
        && oriented.v >= frameV0 - 0.055 && oriented.v <= frameV1 + 0.055;
      const onHorizontalFrame = (Math.abs(oriented.v - frameV0) <= 0.045
          || Math.abs(oriented.v - frameV1) <= 0.045)
        && oriented.u >= frameU0 - 0.06 && oriented.u <= frameU1 + 0.06;
      const onCalibrationStrip = Math.abs(oriented.v - stripV) <= 0.048
        && oriented.u >= frameU0 - 0.06 && oriented.u <= frameU1 + 0.06;
      if (onVerticalFrame) return { rotation, axis: "u" };
      if (onHorizontalFrame || onCalibrationStrip) return { rotation, axis: "v" };
    }
    return null;
  }

  function sampleYellowChroma(width, height, pixels, paperQuad, normalized) {
    const source = bilinear(paperQuad, normalized.u, normalized.v);
    const x = clamp(Math.round(source.x), 0, width - 1);
    const y = clamp(Math.round(source.y), 0, height - 1);
    const offset = (y * width + x) * 4;
    return yellowChroma(pixels[offset], pixels[offset + 1], pixels[offset + 2]);
  }

  function sampleLuminance(width, height, pixels, paperQuad, normalized) {
    const source = bilinear(paperQuad, normalized.u, normalized.v);
    const x = clamp(Math.round(source.x), 0, width - 1);
    const y = clamp(Math.round(source.y), 0, height - 1);
    const offset = (y * width + x) * 4;
    return pixels[offset] * 0.2126 + pixels[offset + 1] * 0.7152 + pixels[offset + 2] * 0.0722;
  }

  function sampleColour(width, height, pixels, paperQuad, normalized) {
    const source = bilinear(paperQuad, normalized.u, normalized.v);
    const x = clamp(Math.round(source.x), 0, width - 1);
    const y = clamp(Math.round(source.y), 0, height - 1);
    const offset = (y * width + x) * 4;
    return [pixels[offset], pixels[offset + 1], pixels[offset + 2]];
  }

  function localStencilEvidence(point, r, g, b, width, height, pixels, paperQuad) {
    const band = stencilReferenceBandAxes(point);
    if (!band) return { accepted: false, chromatic: false, neutral: false, score: 0 };
    const oriented = rotateNormalized(point, band.rotation);
    const delta = 0.014;
    const before = { ...oriented, [band.axis]: oriented[band.axis] - delta };
    const after = { ...oriented, [band.axis]: oriented[band.axis] + delta };
    const first = sampleColour(width, height, pixels, paperQuad,
      unrotateNormalized(before, band.rotation));
    const second = sampleColour(width, height, pixels, paperQuad,
      unrotateNormalized(after, band.rotation));
    const firstChroma = yellowChroma(...first);
    const secondChroma = yellowChroma(...second);
    const centerChroma = yellowChroma(r, g, b);
    const firstLuminance = first[0] * 0.2126 + first[1] * 0.7152 + first[2] * 0.0722;
    const secondLuminance = second[0] * 0.2126 + second[1] * 0.7152 + second[2] * 0.0722;
    const centerLuminance = r * 0.2126 + g * 0.7152 + b * 0.0722;
    const maximum = Math.max(r, g, b);
    const minimum = Math.min(r, g, b);
    const saturation = maximum ? (maximum - minimum) / maximum : 0;
    const chromaMargin = centerChroma - Math.max(firstChroma, secondChroma);
    const averageChromaMargin = centerChroma - (firstChroma + secondChroma) * 0.5;
    const minimumDarkness = Math.min(firstLuminance, secondLuminance) - centerLuminance;
    const averageDarkness = (firstLuminance + secondLuminance) * 0.5 - centerLuminance;
    const balancedPaper = Math.abs(firstLuminance - secondLuminance)
      <= Math.max(28, centerLuminance * 0.2);
    const chromatic = chromaMargin >= 0.65 && averageChromaMargin >= 1.1;
    const neutral = saturation <= 0.24
      && centerLuminance >= 48
      && balancedPaper
      && minimumDarkness >= 2.5
      && averageDarkness >= 5.2;
    return {
      accepted: chromatic || neutral,
      chromatic,
      neutral,
      score: Math.max(0, chromaMargin) * 1.6 + Math.max(0, averageChromaMargin)
        + Math.max(0, minimumDarkness) * 0.34 + Math.max(0, averageDarkness) * 0.18
    };
  }

  function hasLocalStencilContrast(point, r, g, b, width, height, pixels, paperQuad) {
    return localStencilEvidence(point, r, g, b, width, height, pixels, paperQuad).accepted;
  }

  function neutralCalibrationSupport(width, height, pixels, paperQuad, baseline) {
    if (!paperQuad?.every(Boolean)) return 0;
    const rails = [27.43 / 29.7, 27.93 / 29.7];
    const ranges = [[2 / 21, 10 / 21], [11 / 21, 19 / 21]];
    let probes = 0;
    let hits = 0;
    for (const rail of rails) {
      for (const [start, end] of ranges) {
        for (let sample = 0; sample <= 28; sample += 1) {
          const u = start + (end - start) * (sample / 28);
          probes += 1;
          let bestScore = 0;
          for (let offsetStep = -10; offsetStep <= 10; offsetStep += 1) {
            const normalized = { u, v: rail + offsetStep * 0.0032 };
            const colour = sampleColour(width, height, pixels, paperQuad, normalized);
            if (!isNeutralStencilTone(...colour, baseline)) continue;
            const evidence = localStencilEvidence(
              normalized, ...colour, width, height, pixels, paperQuad
            );
            if (evidence.neutral) bestScore = Math.max(bestScore, evidence.score);
          }
          if (bestScore >= 3.2) hits += 1;
        }
      }
    }
    return hits / Math.max(1, probes);
  }

  function hasPaperOnBothSides(point, r, g, b, width, height, pixels, paperQuad) {
    const band = stencilReferenceBandAxes(point);
    if (!band) return false;
    const oriented = rotateNormalized(point, band.rotation);
    const delta = 0.014;
    const before = { ...oriented, [band.axis]: oriented[band.axis] - delta };
    const after = { ...oriented, [band.axis]: oriented[band.axis] + delta };
    const first = sampleLuminance(width, height, pixels, paperQuad,
      unrotateNormalized(before, band.rotation));
    const second = sampleLuminance(width, height, pixels, paperQuad,
      unrotateNormalized(after, band.rotation));
    const center = r * 0.2126 + g * 0.7152 + b * 0.0722;
    // A printed frame is inset into the sheet, therefore it has paper on both
    // sides. Only the relaxed trace mask needs this check: marker discovery
    // retains every colour candidate, while the geometric tracer discards a
    // one-light/one-dark paper silhouette that can look yellow under a warm
    // desk or shadow.
    return Math.abs(first - second) <= Math.max(24, center * 0.28);
  }

  function robustLine(points, independentKey, dependentKey) {
    if (points.length < 12) return null;
    let active = points;
    let line = null;
    for (let pass = 0; pass < 3; pass += 1) {
      let sx = 0, sy = 0, sxx = 0, sxy = 0;
      for (const point of active) {
        const x = point[independentKey];
        const y = point[dependentKey];
        sx += x;
        sy += y;
        sxx += x * x;
        sxy += x * y;
      }
      const count = active.length;
      const denominator = count * sxx - sx * sx;
      if (Math.abs(denominator) < 1e-6) return null;
      const a = (count * sxy - sx * sy) / denominator;
      const b = (sy - a * sx) / count;
      line = { a, b, count };
      if (pass === 2) break;
      const residuals = active.map(point => Math.abs(point[dependentKey] - (a * point[independentKey] + b)));
      const sorted = residuals.slice().sort((x, y) => x - y);
      const median = sorted[Math.floor(sorted.length / 2)] || 1;
      const threshold = Math.max(2.25, median * 2.8);
      const filtered = active.filter((point, index) => residuals[index] <= threshold);
      if (filtered.length < 12 || filtered.length === active.length) break;
      active = filtered;
    }
    return line;
  }

  function ransacLine(points, independentKey, dependentKey, minimumSpan, tolerance) {
    if (points.length < 14) return null;
    let seed = (points.length * 2654435761) >>> 0;
    const randomIndex = () => {
      seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
      return seed % points.length;
    };
    let bestInliers = null;
    let bestScore = 0;
    const hypotheses = Math.min(220, Math.max(80, points.length));
    for (let attempt = 0; attempt < hypotheses; attempt += 1) {
      const first = points[randomIndex()];
      const second = points[randomIndex()];
      const independentDelta = second[independentKey] - first[independentKey];
      if (Math.abs(independentDelta) < minimumSpan * 0.32) continue;
      const a = (second[dependentKey] - first[dependentKey]) / independentDelta;
      const b = first[dependentKey] - a * first[independentKey];
      const inliers = [];
      let minimum = Infinity;
      let maximum = -Infinity;
      for (const point of points) {
        if (Math.abs(point[dependentKey] - (a * point[independentKey] + b)) > tolerance) continue;
        inliers.push(point);
        minimum = Math.min(minimum, point[independentKey]);
        maximum = Math.max(maximum, point[independentKey]);
      }
      const span = maximum - minimum;
      if (span < minimumSpan) continue;
      const score = inliers.length * (1 + Math.min(1, span / (minimumSpan * 1.8)));
      if (score > bestScore) {
        bestScore = score;
        bestInliers = inliers;
      }
    }
    return bestInliers && bestInliers.length >= 12
      ? robustLine(bestInliers, independentKey, dependentKey)
      : null;
  }

  function lineIntersection(vertical, horizontal) {
    if (!vertical || !horizontal) return null;
    // vertical: x = a*y+b; horizontal: y = a*x+b
    const denominator = 1 - horizontal.a * vertical.a;
    if (Math.abs(denominator) < 1e-5) return null;
    const y = (horizontal.a * vertical.b + horizontal.b) / denominator;
    return { x: vertical.a * y + vertical.b, y };
  }

  function fallbackPageQuad(source) {
    const width = source.width;
    const height = source.height;
    const ratio = width / height;
    let left = 0, top = 0, right = width, bottom = height;
    if (ratio > A4_RATIO) {
      const targetWidth = height * A4_RATIO;
      left = (width - targetWidth) * 0.5;
      right = left + targetWidth;
    } else {
      const targetHeight = width / A4_RATIO;
      top = (height - targetHeight) * 0.5;
      bottom = top + targetHeight;
    }
    return [
      { x: left, y: top },
      { x: right, y: top },
      { x: right, y: bottom },
      { x: left, y: bottom }
    ];
  }

  function polygonArea(quad) {
    let area = 0;
    for (let index = 0; index < quad.length; index += 1) {
      const current = quad[index];
      const next = quad[(index + 1) % quad.length];
      area += current.x * next.y - next.x * current.y;
    }
    return Math.abs(area) * 0.5;
  }

  function isPlausibleQuad(quad, width, height, options = {}) {
    if (!Array.isArray(quad) || quad.length !== 4 || !quad.every(Boolean)) return false;
    const margin = options.allowOutside ? Math.max(width, height) * 0.08 : 2;
    if (quad.some(point => point.x < -margin || point.y < -margin
      || point.x > width + margin || point.y > height + margin)) return false;
    const areaRatio = polygonArea(quad) / Math.max(1, width * height);
    if (areaRatio < (options.minAreaRatio || 0.18) || areaRatio > 1.18) return false;
    const lengths = [
      Math.hypot(quad[1].x - quad[0].x, quad[1].y - quad[0].y),
      Math.hypot(quad[2].x - quad[1].x, quad[2].y - quad[1].y),
      Math.hypot(quad[3].x - quad[2].x, quad[3].y - quad[2].y),
      Math.hypot(quad[0].x - quad[3].x, quad[0].y - quad[3].y)
    ];
    if (Math.min(...lengths) < Math.min(width, height) * 0.18) return false;
    const horizontalRatio = Math.max(lengths[0], lengths[2]) / Math.max(1, Math.min(lengths[0], lengths[2]));
    const verticalRatio = Math.max(lengths[1], lengths[3]) / Math.max(1, Math.min(lengths[1], lengths[3]));
    const maxOppositeRatio = options.maxOppositeRatio || 1.58;
    return horizontalRatio <= maxOppositeRatio && verticalRatio <= maxOppositeRatio;
  }

  function convexHull(points) {
    if (points.length <= 4) return points.slice();
    const sorted = points.slice().sort((a, b) => a.x - b.x || a.y - b.y);
    const cross = (origin, a, b) => (a.x - origin.x) * (b.y - origin.y)
      - (a.y - origin.y) * (b.x - origin.x);
    const lower = [];
    for (const point of sorted) {
      while (lower.length >= 2 && cross(lower[lower.length - 2], lower[lower.length - 1], point) <= 0) {
        lower.pop();
      }
      lower.push(point);
    }
    const upper = [];
    for (let index = sorted.length - 1; index >= 0; index -= 1) {
      const point = sorted[index];
      while (upper.length >= 2 && cross(upper[upper.length - 2], upper[upper.length - 1], point) <= 0) {
        upper.pop();
      }
      upper.push(point);
    }
    lower.pop();
    upper.pop();
    return lower.concat(upper);
  }

  function cornerFromHull(hull, score) {
    const ranked = hull
      .map(point => ({ point, score: score(point) }))
      .sort((a, b) => a.score - b.score);
    const count = Math.max(1, Math.min(4, Math.ceil(ranked.length * 0.035)));
    let x = 0, y = 0, weightTotal = 0;
    for (let index = 0; index < count; index += 1) {
      const weight = count - index;
      x += ranked[index].point.x * weight;
      y += ranked[index].point.y * weight;
      weightTotal += weight;
    }
    return { x: x / weightTotal, y: y / weightTotal };
  }

  function detectPaperQuad(width, height, pixels) {
    const size = width * height;
    const candidate = new Uint8Array(size);
    for (let offset = 0, pixel = 0; offset < pixels.length; offset += 4, pixel += 1) {
      const r = pixels[offset], g = pixels[offset + 1], b = pixels[offset + 2];
      const max = Math.max(r, g, b);
      const min = Math.min(r, g, b);
      const luminance = r * 0.2126 + g * 0.7152 + b * 0.0722;
      const saturation = max ? (max - min) / max : 1;
      if (luminance >= 72 && saturation <= 0.27
        && g / Math.max(1, r) >= 0.76
        && b / Math.max(1, r) >= 0.67) {
        candidate[pixel] = 1;
      }
    }

    // Ink, the dotted template and the calibration strip can form a complete
    // dark barrier across the sheet. A tiny separable dilation reconnects the
    // paper on both sides without moving its outer boundary materially.
    const horizontal = new Uint8Array(size);
    const navigable = new Uint8Array(size);
    const bridgeRadius = 3;
    for (let y = 0; y < height; y += 1) {
      const row = y * width;
      for (let x = 0; x < width; x += 1) {
        let found = 0;
        for (let dx = -bridgeRadius; dx <= bridgeRadius; dx += 1) {
          const sx = x + dx;
          if (sx >= 0 && sx < width && candidate[row + sx]) { found = 1; break; }
        }
        horizontal[row + x] = found;
      }
    }
    for (let y = 0; y < height; y += 1) {
      for (let x = 0; x < width; x += 1) {
        let found = 0;
        for (let dy = -bridgeRadius; dy <= bridgeRadius; dy += 1) {
          const sy = y + dy;
          if (sy >= 0 && sy < height && horizontal[sy * width + x]) { found = 1; break; }
        }
        navigable[y * width + x] = found;
      }
    }

    const visited = new Uint8Array(size);
    const queue = new Int32Array(size);
    let best = [];
    const centerX = Math.floor(width * 0.5);
    const centerY = Math.floor(height * 0.5);
    const seeds = [];
    for (let radius = 0; radius <= 24; radius += 4) {
      for (let dy = -radius; dy <= radius; dy += Math.max(1, radius || 1)) {
        for (let dx = -radius; dx <= radius; dx += Math.max(1, radius || 1)) {
          const x = clamp(centerX + dx, 0, width - 1);
          const y = clamp(centerY + dy, 0, height - 1);
          const index = y * width + x;
          if (navigable[index]) seeds.push(index);
        }
      }
      if (seeds.length) break;
    }
    if (!seeds.length) return null;

    for (const seed of seeds.slice(0, 6)) {
      if (visited[seed]) continue;
      let head = 0, tail = 0;
      queue[tail++] = seed;
      visited[seed] = 1;
      const boundary = [];
      let componentSize = 0;
      while (head < tail) {
        const index = queue[head++];
        const x = index % width;
        const y = (index / width) | 0;
        componentSize += 1;
        let isBoundary = false;
        const neighbors = [index - 1, index + 1, index - width, index + width];
        for (let direction = 0; direction < 4; direction += 1) {
          if ((direction === 0 && x === 0) || (direction === 1 && x === width - 1)
            || (direction === 2 && y === 0) || (direction === 3 && y === height - 1)) {
            isBoundary = true;
            continue;
          }
          const nextIndex = neighbors[direction];
          if (!navigable[nextIndex]) {
            isBoundary = true;
            continue;
          }
          if (!visited[nextIndex]) {
            visited[nextIndex] = 1;
            queue[tail++] = nextIndex;
          }
        }
        if (isBoundary && (componentSize % 2 === 0)) boundary.push({ x, y });
      }
      if (componentSize > size * 0.08 && boundary.length > best.length) best = boundary;
    }
    if (best.length < 40) return null;
    const hull = convexHull(best);
    if (hull.length < 4) return null;
    const quad = [
      cornerFromHull(hull, point => point.x + point.y),
      cornerFromHull(hull, point => -point.x + point.y),
      cornerFromHull(hull, point => -point.x - point.y),
      cornerFromHull(hull, point => point.x - point.y)
    ];
    return isPlausibleQuad(quad, width, height, {
      minAreaRatio: 0.22,
      maxOppositeRatio: 2.25
    }) ? quad : null;
  }

  function bilinear(quad, u, v) {
    const topX = quad[0].x + (quad[1].x - quad[0].x) * u;
    const topY = quad[0].y + (quad[1].y - quad[0].y) * u;
    const bottomX = quad[3].x + (quad[2].x - quad[3].x) * u;
    const bottomY = quad[3].y + (quad[2].y - quad[3].y) * u;
    return {
      x: topX + (bottomX - topX) * v,
      y: topY + (bottomY - topY) * v
    };
  }

  function extrapolateStencil(stencilQuad) {
    const u0 = 1.5 / 21;
    const u1 = 19.5 / 21;
    const v0 = 1.43 / 29.7;
    const v1 = 28.43 / 29.7;
    const left = -u0 / (u1 - u0);
    const right = (1 - u0) / (u1 - u0);
    const top = -v0 / (v1 - v0);
    const bottom = (1 - v0) / (v1 - v0);
    return [
      bilinear(stencilQuad, left, top),
      bilinear(stencilQuad, right, top),
      bilinear(stencilQuad, right, bottom),
      bilinear(stencilQuad, left, bottom)
    ];
  }

  function quadFitsObservedImage(quad, width, height, marginRatio = 0.085) {
    if (!quad?.every(point => Number.isFinite(point?.x) && Number.isFinite(point?.y))) return false;
    const marginX = width * marginRatio;
    const marginY = height * marginRatio;
    return quad.every(point => point.x >= -marginX
      && point.x <= width + marginX
      && point.y >= -marginY
      && point.y <= height + marginY);
  }

  function invertBilinear(quad, point) {
    let u = 0.5;
    let v = 0.5;
    for (let iteration = 0; iteration < 7; iteration += 1) {
      const estimate = bilinear(quad, u, v);
      const errorX = point.x - estimate.x;
      const errorY = point.y - estimate.y;
      const duX = (quad[1].x - quad[0].x) * (1 - v) + (quad[2].x - quad[3].x) * v;
      const duY = (quad[1].y - quad[0].y) * (1 - v) + (quad[2].y - quad[3].y) * v;
      const dvX = (quad[3].x - quad[0].x) * (1 - u) + (quad[2].x - quad[1].x) * u;
      const dvY = (quad[3].y - quad[0].y) * (1 - u) + (quad[2].y - quad[1].y) * u;
      const determinant = duX * dvY - duY * dvX;
      if (Math.abs(determinant) < 1e-7) return null;
      u += (errorX * dvY - errorY * dvX) / determinant;
      v += (duX * errorY - duY * errorX) / determinant;
      if (Math.abs(errorX) + Math.abs(errorY) < 0.04) break;
    }
    return Number.isFinite(u) && Number.isFinite(v) ? { u, v } : null;
  }

  function traceFrameSide(points, axis, perpendicular, expected, sampleCount = 19, maxDeviation = 0.052, preserveEndpoints = false) {
    const candidateSets = new Array(sampleCount);
    const window = 0.72 / (sampleCount - 1);
    for (let sample = 0; sample < sampleCount; sample += 1) {
      const position = sample / (sampleCount - 1);
      const localPoints = points.filter(point => Math.abs(point[axis] - position) <= window
        && Math.abs(point[perpendicular] - expected) <= maxDeviation);
      const candidates = localPoints
        .map(point => point[perpendicular])
        .sort((a, b) => a - b);
      const clusters = [];
      for (const candidate of candidates) {
        const cluster = clusters[clusters.length - 1];
        if (!cluster || candidate - cluster[cluster.length - 1] > 0.009) clusters.push([candidate]);
        else cluster.push(candidate);
      }
      const choices = clusters
        .filter(cluster => cluster.length >= 2)
        .map(cluster => ({
          value: cluster[Math.floor(cluster.length / 2)],
          strength: Math.min(36, cluster.length),
          supported: true
        }));
      const localLine = guidedRansacLine(localPoints, axis, perpendicular, {
        minimumSpan: Math.max(0.022, window * 0.82),
        tolerance: 0.0044,
        maximumSlope: 0.58,
        slopePenaltyScale: 0.34,
        expected: null,
        anchorIndependent: position,
        anchorDependent: expected
      });
      if (localLine) {
        const value = localLine.a * position + localLine.b;
        if (Math.abs(value - expected) <= maxDeviation) {
          choices.push({
            value,
            strength: Math.min(48, (localLine.support || 12) * 1.35),
            supported: true
          });
        }
      }
      choices.sort((first, second) => first.value - second.value);
      const deduplicated = [];
      for (const choice of choices) {
        const previous = deduplicated[deduplicated.length - 1];
        if (!previous || choice.value - previous.value > 0.004) deduplicated.push(choice);
        else if (choice.strength > previous.strength) deduplicated[deduplicated.length - 1] = choice;
      }
      candidateSets[sample] = deduplicated.length
        ? deduplicated
        : [{ value: expected, strength: -12, supported: false }];
    }

    if (!preserveEndpoints) {
      candidateSets[0] = [{ value: expected, strength: 0, supported: false }];
      candidateSets[sampleCount - 1] = [{ value: expected, strength: 0, supported: false }];
    }

    // Select one continuous ridge across the whole side. Independent local
    // maxima jump between overlapping sheets; this dynamic path rewards the
    // same long, smooth border from corner to corner.
    const costs = candidateSets.map(choices => choices.map(() => Infinity));
    const parents = candidateSets.map(choices => choices.map(() => -1));
    candidateSets[0].forEach((choice, index) => {
      costs[0][index] = Math.abs(choice.value - expected) * 80 - choice.strength;
    });
    for (let sample = 1; sample < sampleCount; sample += 1) {
      candidateSets[sample].forEach((choice, choiceIndex) => {
        const nodeCost = Math.abs(choice.value - expected) * 80 - choice.strength;
        candidateSets[sample - 1].forEach((previous, previousIndex) => {
          const step = Math.abs(choice.value - previous.value);
          const transition = step * 520 + Math.max(0, step - 0.018) * 1600;
          const cost = costs[sample - 1][previousIndex] + transition + nodeCost;
          if (cost < costs[sample][choiceIndex]) {
            costs[sample][choiceIndex] = cost;
            parents[sample][choiceIndex] = previousIndex;
          }
        });
      });
    }
    let selectedIndex = costs[sampleCount - 1]
      .reduce((best, cost, index, all) => cost < all[best] ? index : best, 0);
    const values = new Array(sampleCount);
    const supported = new Array(sampleCount);
    for (let sample = sampleCount - 1; sample >= 0; sample -= 1) {
      const selected = candidateSets[sample][selectedIndex];
      values[sample] = selected.value;
      supported[sample] = selected.supported;
      selectedIndex = parents[sample][selectedIndex];
      if (sample && selectedIndex < 0) selectedIndex = 0;
    }
    const support = supported.filter(Boolean).length / sampleCount;
    if (support < 0.28) {
      return {
        values: values.map(() => expected),
        supported,
        support
      };
    }
    let smoothed = values.map((value, index) => {
      const neighborhood = values.slice(Math.max(0, index - 2), Math.min(values.length, index + 3)).slice().sort((a, b) => a - b);
      return neighborhood[Math.floor(neighborhood.length / 2)];
    });
    for (let pass = 0; pass < 5; pass += 1) {
      smoothed = smoothed.map((value, index) => {
        if (!preserveEndpoints && (!index || index === smoothed.length - 1)) return expected;
        if (!index) return value * 0.72 + smoothed[index + 1] * 0.28;
        if (index === smoothed.length - 1) return smoothed[index - 1] * 0.28 + value * 0.72;
        return smoothed[index - 1] * 0.22 + value * 0.56 + smoothed[index + 1] * 0.22;
      });
    }
    if (preserveEndpoints && smoothed.length >= 4) {
      // The perpendicular border also occupies the corner window, so its
      // dense pixels can bias the first cluster. Continue the selected ridge
      // from its two nearest interior samples instead of averaging the two
      // intersecting strokes.
      smoothed[0] = clamp(smoothed[1] * 2 - smoothed[2], expected - maxDeviation, expected + maxDeviation);
      const last = smoothed.length - 1;
      smoothed[last] = clamp(smoothed[last - 1] * 2 - smoothed[last - 2], expected - maxDeviation, expected + maxDeviation);
    }
    const maximumStep = preserveEndpoints ? 0.018 : 0.012;
    for (let index = 1; index < sampleCount; index += 1) {
      smoothed[index] = clamp(smoothed[index], smoothed[index - 1] - maximumStep, smoothed[index - 1] + maximumStep);
    }
    for (let index = sampleCount - 2; index >= 0; index -= 1) {
      smoothed[index] = clamp(smoothed[index], smoothed[index + 1] - maximumStep, smoothed[index + 1] + maximumStep);
    }
    const correctionLimit = preserveEndpoints ? maxDeviation : Math.max(0.032, Math.min(0.085, maxDeviation));
    smoothed = smoothed.map((value, index) => !preserveEndpoints && (index === 0 || index === sampleCount - 1)
      ? expected
      : expected + clamp(value - expected, -correctionLimit, correctionLimit));
    return {
      values: smoothed,
      supported,
      support
    };
  }

  function traceFrameGuide(sourcePoints, stencilQuad, axis, perpendicular,
    sampleCount = 25, expected = 0) {
    if (!sourcePoints?.length || !stencilQuad?.every(Boolean)) return null;
    const observed = sourcePoints
      .map(source => ({ source, normalized: invertBilinear(stencilQuad, source) }))
      .filter(entry => entry.normalized
        && entry.normalized[axis] >= -0.08 && entry.normalized[axis] <= 1.08
        && Math.abs(entry.normalized[perpendicular] - expected) <= 0.1)
      .sort((first, second) => first.normalized[axis] - second.normalized[axis]);
    if (observed.length < 18) return null;
    // The neutral ridge was sampled at evenly spaced physical page rows. A
    // projective inverse of the provisional straight quad changes that row
    // parameter even when the spatial curve is correct, which makes the warp
    // slide content vertically along a side. Preserve the observed transverse
    // coordinate but restore the acquisition order as the canonical axis.
    const normalized = observed.map((entry, index) => ({
      ...entry.normalized,
      [axis]: index / Math.max(1, observed.length - 1)
    }));
    const values = new Array(sampleCount).fill(null);
    const supported = new Array(sampleCount).fill(false);
    const window = 0.032;
    for (let sample = 0; sample < sampleCount; sample += 1) {
      const position = sample / (sampleCount - 1);
      const local = normalized
        .filter(point => Math.abs(point[axis] - position) <= window)
        .map(point => point[perpendicular])
        .sort((first, second) => first - second);
      if (!local.length) continue;
      values[sample] = local[Math.floor(local.length / 2)];
      supported[sample] = true;
    }
    const support = supported.filter(Boolean).length / sampleCount;
    if (support < 0.55) return null;
    for (let sample = 0; sample < sampleCount; sample += 1) {
      if (Number.isFinite(values[sample])) continue;
      let before = sample - 1;
      let after = sample + 1;
      while (before >= 0 && !Number.isFinite(values[before])) before -= 1;
      while (after < sampleCount && !Number.isFinite(values[after])) after += 1;
      if (before >= 0 && after < sampleCount) {
        const mix = (sample - before) / (after - before);
        values[sample] = values[before] + (values[after] - values[before]) * mix;
      } else if (before >= 1) {
        values[sample] = values[before] + (values[before] - values[before - 1]) * (sample - before);
      } else if (after + 1 < sampleCount) {
        values[sample] = values[after] + (values[after] - values[after + 1]) * (after - sample);
      } else values[sample] = expected;
    }
    let smoothed = values.slice();
    for (let pass = 0; pass < 2; pass += 1) {
      smoothed = smoothed.map((value, index) => {
        if (!index || index === smoothed.length - 1) return value;
        return smoothed[index - 1] * 0.16 + value * 0.68 + smoothed[index + 1] * 0.16;
      });
    }
    smoothed = smoothed.map(value => expected + clamp(value - expected, -0.075, 0.075));
    const sourcePath = new Array(sampleCount).fill(null).map((_, sample) => {
      const scaled = sample / (sampleCount - 1) * (observed.length - 1);
      const index = Math.min(observed.length - 2, Math.floor(scaled));
      const mix = scaled - index;
      const first = observed[index].source;
      const second = observed[index + 1].source;
      return {
        x: first.x + (second.x - first.x) * mix,
        y: first.y + (second.y - first.y) * mix
      };
    });
    return { values: smoothed, supported, support, sourcePath };
  }

  function refineStencilQuad(stencilQuad, width, height, normalizedYellow) {
    const gridSize = 196;
    const minimum = -0.22;
    const maximum = 1.22;
    const span = maximum - minimum;
    const counts = new Uint16Array(gridSize * gridSize);
    const toBin = value => clamp(Math.round((value - minimum) / span * (gridSize - 1)), 0, gridSize - 1);
    const toValue = bin => minimum + bin / (gridSize - 1) * span;
    for (const point of normalizedYellow) {
      if (point.u < minimum || point.u > maximum || point.v < minimum || point.v > maximum) continue;
      counts[toBin(point.v) * gridSize + toBin(point.u)] += 1;
    }
    const armLength = Math.round(gridSize * 0.13 / span);
    const bandRadius = 1;
    const armSupport = (x, y, dx, dy) => {
      let support = 0;
      for (let step = 2; step <= armLength; step += 1) {
        for (let band = -bandRadius; band <= bandRadius; band += 1) {
          const sampleX = x + dx * step + (dy ? band : 0);
          const sampleY = y + dy * step + (dx ? band : 0);
          if (sampleX < 0 || sampleX >= gridSize || sampleY < 0 || sampleY >= gridSize) continue;
          support += Math.min(4, counts[sampleY * gridSize + sampleX]);
        }
      }
      return support;
    };
    const definitions = [
      { u: 0, v: 0, horizontal: 1, vertical: 1 },
      { u: 1, v: 0, horizontal: -1, vertical: 1 },
      { u: 1, v: 1, horizontal: -1, vertical: -1 },
      { u: 0, v: 1, horizontal: 1, vertical: -1 }
    ];
    let minimumArmSupport = Infinity;
    const normalizedCorners = definitions.map(definition => {
      const targetX = toBin(definition.u);
      const targetY = toBin(definition.v);
      const radius = Math.round(gridSize * 0.2 / span);
      let best = null;
      for (let y = Math.max(0, targetY - radius); y <= Math.min(gridSize - 1, targetY + radius); y += 1) {
        for (let x = Math.max(0, targetX - radius); x <= Math.min(gridSize - 1, targetX + radius); x += 1) {
          const centerCount = counts[y * gridSize + x];
          if (!centerCount) continue;
          const horizontal = armSupport(x, y, definition.horizontal, 0);
          const vertical = armSupport(x, y, 0, definition.vertical);
          const weakest = Math.min(horizontal, vertical);
          if (weakest < 4) continue;
          const distance = Math.hypot(x - targetX, y - targetY);
          const score = weakest * 4 + horizontal + vertical + Math.min(12, centerCount) * 2 - distance * 1.4;
          if (!best || score > best.score) best = { x, y, score, weakest };
        }
      }
      if (!best) return null;
      minimumArmSupport = Math.min(minimumArmSupport, best.weakest);
      const coarse = { u: toValue(best.x), v: toValue(best.y) };
      const horizontalPoints = normalizedYellow.filter(point => {
        const inward = (point.u - coarse.u) * definition.horizontal;
        // The lower calibration box is only 0.0185 frame units above the
        // outer border. A wider corner band lets that parallel rail pull the
        // fitted corner into the box instead of the rectangle intersection.
        return inward >= 0.01 && inward <= 0.18 && Math.abs(point.v - coarse.v) <= 0.009;
      });
      const verticalPoints = normalizedYellow.filter(point => {
        const inward = (point.v - coarse.v) * definition.vertical;
        return inward >= 0.01 && inward <= 0.18 && Math.abs(point.u - coarse.u) <= 0.009;
      });
      const horizontalLine = guidedRansacLine(horizontalPoints, "u", "v", {
        minimumSpan: 0.07,
        tolerance: 0.0045,
        maximumSlope: 0.55,
        slopePenaltyScale: 0.24,
        expected: null
      });
      const verticalLine = guidedRansacLine(verticalPoints, "v", "u", {
        minimumSpan: 0.07,
        tolerance: 0.0045,
        maximumSlope: 0.55,
        slopePenaltyScale: 0.24,
        expected: null
      });
      const precise = lineIntersection(verticalLine, horizontalLine);
      if (precise && Math.hypot(precise.x - coarse.u, precise.y - coarse.v) <= 0.045) {
        return { u: precise.x, v: precise.y };
      }
      return coarse;
    });
    if (!normalizedCorners.every(Boolean) || minimumArmSupport < 5) return null;
    if (normalizedCorners.some((corner, index) => Math.hypot(
      corner.u - definitions[index].u,
      corner.v - definitions[index].v
    ) > 0.135)) return null;
    const refined = normalizedCorners.map(corner => bilinear(stencilQuad, corner.u, corner.v));
    if (!isPlausibleQuad(refined, width, height, {
      allowOutside: true,
      minAreaRatio: 0.16,
      maxOppositeRatio: 2.2
    })) return null;
    return { quad: refined, minimumArmSupport };
  }

  function traceLowerStencilRails(points, sampleCount = 25) {
    const expectedValues = [1 - 1 / 27, 1 - 0.5 / 27, 1];
    const firstBoxStart = (2 - 1.5) / 18;
    const firstBoxEnd = (10 - 1.5) / 18;
    const secondBoxStart = (11 - 1.5) / 18;
    const secondBoxEnd = (19 - 1.5) / 18;
    const railExistsAt = (rail, u) => rail === 2
      || (u >= firstBoxStart && u <= firstBoxEnd)
      || (u >= secondBoxStart && u <= secondBoxEnd);
    const deltas = new Array(sampleCount).fill(null);
    const matched = expectedValues.map(() => new Array(sampleCount).fill(false));
    const window = 0.058;
    const occupancyBins = 20;
    const ridgeTolerance = 0.0048;
    let previousDelta = 0;
    for (let sample = 0; sample < sampleCount; sample += 1) {
      const position = sample / (sampleCount - 1);
      const localPoints = points.filter(point => Math.abs(point.u - position) <= window
        && point.v >= 0.86 && point.v <= 1.12);
      const horizontalSupport = (target, rail) => {
        const occupied = new Uint8Array(occupancyBins);
        for (const point of localPoints) {
          if (!railExistsAt(rail, point.u)) continue;
          if (Math.abs(point.v - target) > ridgeTolerance) continue;
          const bin = clamp(Math.floor((point.u - (position - window)) / (window * 2) * occupancyBins), 0, occupancyBins - 1);
          occupied[bin] = 1;
        }
        let available = 0;
        for (let bin = 0; bin < occupancyBins; bin += 1) {
          const u = position - window + (bin + 0.5) / occupancyBins * window * 2;
          if (railExistsAt(rail, u)) available += 1;
        }
        const count = occupied.reduce((sum, value) => sum + value, 0);
        return { count, available, ratio: count / Math.max(1, available) };
      };
      let best = null;
      for (let deltaStep = -42; deltaStep <= 42; deltaStep += 1) {
        const delta = deltaStep * 0.002;
        const supports = expectedValues.map((expected, rail) => horizontalSupport(expected + delta, rail));
        const valid = supports.map(value => value.count >= Math.max(2, Math.floor(value.available * 0.24))
          && value.ratio >= 0.24);
        const matchCount = valid.filter(Boolean).length;
        if (matchCount < 2) continue;
        const score = matchCount * 100 + supports.reduce((sum, value) => sum
          + Math.min(16, value.count) * 4 + Math.min(1, value.ratio) * 32, 0)
          - Math.abs(delta) * 100 - Math.abs(delta - previousDelta) * 150;
        if (!best || score > best.score) best = { delta, valid, score };
      }
      if (!best) continue;

      // The occupancy pass deliberately ignores slope so vertical box teeth
      // cannot win. It is a reliable coarse selector, but on a curled sheet a
      // wide window returns a delayed/averaged height. Refine the chosen three
      // rails with short horizontal RANSAC fits and evaluate each fit exactly
      // at this sample. This follows real local curvature without ever
      // mistaking a vertical cell divider for a rail.
      const refinedDeltas = [];
      expectedValues.forEach((expected, rail) => {
        if (!best.valid[rail]) return;
        const selected = localPoints.filter(point => railExistsAt(rail, point.u)
          && Math.abs(point.v - (expected + best.delta)) <= 0.0125);
        const line = guidedRansacLine(selected, "u", "v", {
          minimumSpan: 0.035,
          tolerance: 0.0042,
          maximumSlope: 0.42,
          slopePenaltyScale: 0.3,
          expected: null,
          anchorIndependent: position,
          anchorDependent: expected + best.delta
        });
        if (!line) return;
        const delta = line.a * position + line.b - expected;
        if (Math.abs(delta - best.delta) > 0.016) return;
        refinedDeltas.push(delta);
        matched[rail][sample] = true;
      });
      refinedDeltas.sort((first, second) => first - second);
      const resolvedDelta = refinedDeltas.length >= 2
        ? refinedDeltas[Math.floor(refinedDeltas.length / 2)]
        : best.delta;
      deltas[sample] = resolvedDelta;
      previousDelta = resolvedDelta;
      if (refinedDeltas.length < 2) {
        best.valid.forEach((value, rail) => { matched[rail][sample] = value; });
      }
    }
    const sharedSupport = deltas.filter(Number.isFinite).length / sampleCount;
    if (sharedSupport < 0.32) return null;
    for (let index = 0; index < sampleCount; index += 1) {
      if (Number.isFinite(deltas[index])) continue;
      let before = index - 1;
      let after = index + 1;
      while (before >= 0 && !Number.isFinite(deltas[before])) before -= 1;
      while (after < sampleCount && !Number.isFinite(deltas[after])) after += 1;
      if (before >= 0 && after < sampleCount) {
        const mix = (index - before) / (after - before);
        deltas[index] = deltas[before] + (deltas[after] - deltas[before]) * mix;
      } else if (before >= 0) deltas[index] = deltas[before];
      else if (after < sampleCount) deltas[index] = deltas[after];
      else deltas[index] = 0;
    }
    let smoothed = deltas.map((value, index) => {
      const neighborhood = deltas
        .slice(Math.max(0, index - 2), Math.min(sampleCount, index + 3))
        .slice()
        .sort((first, second) => first - second);
      return neighborhood[Math.floor(neighborhood.length / 2)];
    });
    for (let pass = 0; pass < 3; pass += 1) {
      smoothed = smoothed.map((value, index) => {
        if (!index) return value * 0.78 + smoothed[index + 1] * 0.22;
        if (index === sampleCount - 1) return smoothed[index - 1] * 0.22 + value * 0.78;
        return smoothed[index - 1] * 0.2 + value * 0.6 + smoothed[index + 1] * 0.2;
      });
    }
    // A partial non-template rectangle can accidentally supply two apparent
    // rails. Never let a degenerate fit contaminate the frame corners; the
    // independent outer-border tracer below is the safe fallback.
    if (smoothed.some(value => !Number.isFinite(value))) return null;
    const maximumStep = 0.009;
    for (let index = 1; index < sampleCount; index += 1) {
      smoothed[index] = clamp(smoothed[index], smoothed[index - 1] - maximumStep, smoothed[index - 1] + maximumStep);
    }
    for (let index = sampleCount - 2; index >= 0; index -= 1) {
      smoothed[index] = clamp(smoothed[index], smoothed[index + 1] - maximumStep, smoothed[index + 1] + maximumStep);
    }
    return expectedValues.map((expected, rail) => ({
      values: smoothed.map(delta => expected + clamp(delta, -0.075, 0.075)),
      supported: matched[rail],
      support: Math.max(sharedSupport, matched[rail].filter(Boolean).length / sampleCount)
    }));
  }

  function traceYellowFrame(stencilQuad, width, height, yellowMask,
    allowRefinement = true, trustedAxis = false, safeTopMask = null,
    neutralRecovery = false, neutralGuides = null) {
    if (!stencilQuad?.every(Boolean)) return null;
    const normalizedYellow = [];
    for (let index = 0; index < yellowMask.length; index += 1) {
      if (!yellowMask[index]) continue;
      const normalized = invertBilinear(stencilQuad, {
        x: index % width,
        y: (index / width) | 0
      });
      if (normalized && normalized.u > -0.22 && normalized.u < 1.22
        && normalized.v > -0.22 && normalized.v < 1.22) {
        normalizedYellow.push({
          ...normalized,
          strength: yellowMask[index],
          safeTop: safeTopMask ? safeTopMask[index] : yellowMask[index]
        });
      }
    }
    if (normalizedYellow.length < 48) return null;

    if (allowRefinement) {
      const refinement = refineStencilQuad(stencilQuad, width, height, normalizedYellow);
      const refinedStencilQuad = refinement?.quad || null;
      if (refinedStencilQuad) {
        const movement = Math.max(...refinedStencilQuad.map((point, index) => Math.hypot(
          point.x - stencilQuad[index].x,
          point.y - stencilQuad[index].y
        )));
        if (movement >= 1.25) {
          const retraced = traceYellowFrame(refinedStencilQuad, width, height, yellowMask,
            false, trustedAxis, safeTopMask, neutralRecovery, neutralGuides);
          if (retraced && retraced.support >= 0.2) {
            return {
              ...retraced,
              refinedStencilQuad: retraced.refinedStencilQuad || refinedStencilQuad,
              refinementMovement: movement,
              refinementArmSupport: refinement.minimumArmSupport
            };
          }
        }
      }
    }

    // The lower box spans y=27.43..27.93 cm while the outer frame ends at
    // y=28.43 cm. Detect both box rails independently; the coloured marker
    // centres at y=27.68 are calibration references, not a yellow border.
    const boxTopExpected = 1 - 1 / 27;
    const boxBottomExpected = 1 - 0.5 / 27;
    const borderDeviation = trustedAxis ? 0.09 : 0.07;
    // Handheld pages can curl a long side by more than ten percent of their
    // width while its two corners remain fixed. Trace both a conservative and
    // a wide corridor: the latter is accepted only when it finds a coherent
    // inward rail. This recovers a strongly bowed printed frame without
    // jumping to a parallel line on a neighbouring sheet.
    const sideDeviation = trustedAxis ? 0.17 : 0.15;
    const safeYellow = normalizedYellow.filter(point => point.safeTop);
    const selectInteriorTrace = (all, safe, inwardSign) => {
      const signedDeltas = all.values.slice(2, -2)
        .map((value, index) => (safe.values[index + 2] - value) * inwardSign)
        .sort((first, second) => first - second);
      const medianShift = signedDeltas[Math.floor(signedDeltas.length * 0.5)] || 0;
      const inwardShare = signedDeltas.filter(value => value > 0.012).length
        / Math.max(1, signedDeltas.length);
      // If the relaxed mask has locked onto a physical page silhouette, the
      // paper-on-both-sides trace reveals a second, consistently inward rail.
      // Require a coherent shift across most of the edge; isolated rejected
      // pixels near a curled corner must not move the complete boundary.
      const useSafe = safe.support >= 0.55
        && medianShift > 0.018
        && inwardShare >= 0.68;
      return {
        trace: useSafe ? safe : all,
        diagnostics: {
          source: useSafe ? "paper-interior" : "all-yellow",
          medianShift,
          inwardShare,
          allSupport: all.support,
          safeSupport: safe.support
        }
      };
    };
    const selectWideSideTrace = (normal, wide, inwardSign) => {
      const signedDeltas = normal.trace.values.slice(2, -2)
        .map((value, index) => (wide.trace.values[index + 2] - value) * inwardSign);
      const sorted = signedDeltas.slice().sort((first, second) => first - second);
      const medianShift = sorted[Math.floor(sorted.length * 0.5)] || 0;
      const inwardShare = signedDeltas.filter(value => value > 0.016).length
        / Math.max(1, signedDeltas.length);
      const outwardShare = signedDeltas.filter(value => value < -0.012).length
        / Math.max(1, signedDeltas.length);
      const useWide = wide.trace.support >= 0.55
        && medianShift > 0.022
        && inwardShare >= 0.62
        && outwardShare <= 0.16;
      return {
        trace: useWide ? wide.trace : normal.trace,
        diagnostics: {
          ...((useWide ? wide : normal).diagnostics),
          corridor: useWide ? "wide" : "normal",
          wideMedianShift: medianShift,
          wideInwardShare: inwardShare,
          wideOutwardShare: outwardShare,
          normalSupport: normal.trace.support,
          wideSupport: wide.trace.support
        }
      };
    };
    // The marker row already fixes the grey-frame top in neutral recovery
    // mode. Keep its curved trace close to that observed seed so a darker,
    // longer first line of handwriting cannot replace the actual border.
    const topDeviation = neutralRecovery ? Math.min(0.045, borderDeviation) : borderDeviation;
    const topSelection = selectInteriorTrace(
      traceFrameSide(normalizedYellow, "u", "v", 0, 25, topDeviation, true),
      traceFrameSide(safeYellow, "u", "v", 0, 25, topDeviation, true),
      1
    );
    const top = topSelection.trace;
    const lowerRails = traceLowerStencilRails(normalizedYellow);
    // Keep every horizontal guide on the same sampling grid as the top edge.
    // A plain page with no lower calibration boxes legitimately falls back to
    // these independent tracers. The old 19-vs-25 sample mismatch left the
    // final six bottom coordinates undefined and could poison the output quad.
    const horizontalSamples = top.values.length;
    const boxTop = lowerRails?.[0] || traceFrameSide(
      normalizedYellow, "u", "v", boxTopExpected, horizontalSamples, 0.032
    );
    const boxBottom = lowerRails?.[1] || traceFrameSide(
      normalizedYellow, "u", "v", boxBottomExpected, horizontalSamples, 0.032
    );
    const bottom = lowerRails?.[2] || traceFrameSide(
      normalizedYellow, "u", "v", 1, horizontalSamples, 0.052
    );
    const leftGuide = neutralRecovery
      ? traceFrameGuide(neutralGuides?.left, stencilQuad, "v", "u", 25, 0)
      : null;
    const rightGuide = neutralRecovery
      ? traceFrameGuide(neutralGuides?.right, stencilQuad, "v", "u", 25, 1)
      : null;
    const leftSelection = leftGuide ? {
      trace: leftGuide,
      diagnostics: { source: "neutral-ridge-guide", allSupport: leftGuide.support, safeSupport: leftGuide.support }
    } : selectWideSideTrace(
      selectInteriorTrace(
        traceFrameSide(normalizedYellow, "v", "u", 0, 25, borderDeviation, true),
        traceFrameSide(safeYellow, "v", "u", 0, 25, borderDeviation, true),
        1
      ),
      selectInteriorTrace(
        traceFrameSide(normalizedYellow, "v", "u", 0, 25, sideDeviation, true),
        traceFrameSide(safeYellow, "v", "u", 0, 25, sideDeviation, true),
        1
      ),
      1
    );
    const rightSelection = rightGuide ? {
      trace: rightGuide,
      diagnostics: { source: "neutral-ridge-guide", allSupport: rightGuide.support, safeSupport: rightGuide.support }
    } : selectWideSideTrace(
      selectInteriorTrace(
        traceFrameSide(normalizedYellow, "v", "u", 1, 25, borderDeviation, true),
        traceFrameSide(safeYellow, "v", "u", 1, 25, borderDeviation, true),
        -1
      ),
      selectInteriorTrace(
        traceFrameSide(normalizedYellow, "v", "u", 1, 25, sideDeviation, true),
        traceFrameSide(safeYellow, "v", "u", 1, 25, sideDeviation, true),
        -1
      ),
      -1
    );
    const left = leftSelection.trace;
    const right = rightSelection.trace;
    const sideSupports = [top.support, right.support, bottom.support, left.support];
    const supportedSides = sideSupports.filter(value => value >= 0.26).length;
    const support = sideSupports.reduce((sum, value) => sum + value, 0) / sideSupports.length;
    if (supportedSides < 2 || support < 0.2) return null;

    // The repeated square row is a second observation of the same paper bend.
    // Pairing its top with the outer baseline prevents either line from being
    // mistaken for the other, while the vertical square teeth never become a
    // candidate for the page boundary.
    const count = top.values.length;
    for (let index = 1; index < count - 1; index += 1) {
      const deltas = [];
      if (bottom.supported[index]) deltas.push(bottom.values[index] - 1);
      if (boxTop.supported[index]) deltas.push(boxTop.values[index] - boxTopExpected);
      if (boxBottom.supported[index]) deltas.push(boxBottom.values[index] - boxBottomExpected);
      if (!deltas.length) continue;
      deltas.sort((a, b) => a - b);
      const sharedDelta = deltas[Math.floor(deltas.length / 2)];
      // Keep the physically observed bend. The previous ±0.022 clamp threw
      // away more than half of the curvature on handheld sheets, so the
      // overlay looked correct but the final warp remained visibly bowed.
      const boundedDelta = clamp(sharedDelta, -0.078, 0.078);
      bottom.values[index] = 1 + boundedDelta;
      boxTop.values[index] = boxTopExpected + boundedDelta;
      boxBottom.values[index] = boxBottomExpected + boundedDelta;
    }

    const topNormalized = top.values.map((value, index) => ({ u: index / (count - 1), v: value }));
    const bottomNormalized = bottom.values.map((value, index) => ({ u: index / (count - 1), v: value }));
    const leftNormalized = left.values.map((value, index) => ({ u: value, v: index / (count - 1) }));
    const rightNormalized = right.values.map((value, index) => ({ u: value, v: index / (count - 1) }));
    const cornersNormalized = [
      { u: left.values[0], v: top.values[0] },
      { u: right.values[0], v: top.values[count - 1] },
      { u: right.values[count - 1], v: bottom.values[count - 1] },
      { u: left.values[count - 1], v: bottom.values[0] }
    ];
    topNormalized[0] = leftNormalized[0] = cornersNormalized[0];
    topNormalized[count - 1] = rightNormalized[0] = cornersNormalized[1];
    bottomNormalized[count - 1] = rightNormalized[count - 1] = cornersNormalized[2];
    bottomNormalized[0] = leftNormalized[count - 1] = cornersNormalized[3];

    const toSource = point => bilinear(stencilQuad, point.u, point.v);
    const paths = {
      top: topNormalized.map(toSource),
      right: rightNormalized.map(toSource),
      bottom: bottomNormalized.map(toSource),
      left: leftNormalized.map(toSource)
    };
    const alignGuideEndpoints = (sourcePath, start, end) => {
      const first = sourcePath[0];
      const last = sourcePath[sourcePath.length - 1];
      return sourcePath.map((point, index) => {
        const amount = index / Math.max(1, sourcePath.length - 1);
        return {
          x: point.x + (start.x - first.x) * (1 - amount) + (end.x - last.x) * amount,
          y: point.y + (start.y - first.y) * (1 - amount) + (end.y - last.y) * amount
        };
      });
    };
    if (left.sourcePath?.length === count) {
      paths.left = alignGuideEndpoints(left.sourcePath, paths.top[0], paths.bottom[0]);
    }
    if (right.sourcePath?.length === count) {
      paths.right = alignGuideEndpoints(
        right.sourcePath, paths.top[count - 1], paths.bottom[count - 1]
      );
    }
    const boxTopPath = boxTop.values.map((value, index) => toSource({
      u: index / (count - 1),
      v: value
    }));
    const boxBottomPath = boxBottom.values.map((value, index) => toSource({
      u: index / (count - 1),
      v: value
    }));
    const yellowBoxNormalized = normalizedYellow.filter(point => point.u >= -0.015 && point.u <= 1.015
      && point.v >= boxTopExpected - 0.018 && point.v <= boxBottomExpected + 0.018);
    const pointStride = Math.max(1, Math.ceil(yellowBoxNormalized.length / 1600));
    const sampledBoxNormalized = yellowBoxNormalized.filter((point, index) => index % pointStride === 0);
    const sampleValues = (values, amount) => {
      const scaled = clamp(amount, 0, 1) * (values.length - 1);
      const index = Math.min(values.length - 2, Math.floor(scaled));
      const mix = scaled - index;
      return values[index] + (values[index + 1] - values[index]) * mix;
    };
    // Keep the real pixels that justified the geometry. The processing
    // animation reveals these fragments first and then consolidates them into
    // the fitted rails, so it never paints a synthetic yellow rectangle over
    // unrelated grey content.
    const frameEvidenceNormalized = normalizedYellow.filter(point => {
      if (point.u < -0.018 || point.u > 1.018 || point.v < -0.018 || point.v > 1.018) return false;
      if (point.v >= boxTopExpected - 0.02 && point.v <= boxBottomExpected + 0.02) return true;
      const distances = [];
      if (point.u >= 0 && point.u <= 1) {
        distances.push(Math.abs(point.v - sampleValues(top.values, point.u)));
        distances.push(Math.abs(point.v - sampleValues(bottom.values, point.u)));
      }
      if (point.v >= 0 && point.v <= 1) {
        distances.push(Math.abs(point.u - sampleValues(left.values, point.v)));
        distances.push(Math.abs(point.u - sampleValues(right.values, point.v)));
      }
      return Math.min(...distances) <= 0.014;
    });
    const evidenceStride = Math.max(1, Math.ceil(frameEvidenceNormalized.length / 2200));
    const sampledFrameEvidence = frameEvidenceNormalized
      .filter((point, index) => index % evidenceStride === 0);
    const corners = cornersNormalized.map(toSource);
    const curvatureFor = (values, start, end) => Math.max(...values.map((value, index) => {
      const expected = start + (end - start) * (index / (values.length - 1));
      return Math.abs(value - expected);
    }));
    const curvature = Math.max(
      curvatureFor(top.values, top.values[0], top.values[count - 1]),
      curvatureFor(bottom.values, bottom.values[0], bottom.values[count - 1]),
      curvatureFor(left.values, left.values[0], left.values[count - 1]),
      curvatureFor(right.values, right.values[0], right.values[count - 1])
    );
    return {
      paths,
      evidence: {
        points: sampledFrameEvidence.map(toSource),
        normalized: sampledFrameEvidence.map(point => ({ u: point.u, v: point.v }))
      },
      box: {
        top: boxTopPath,
        bottom: boxBottomPath,
        outer: paths.bottom,
        points: sampledBoxNormalized.map(toSource),
        normalized: sampledBoxNormalized.map(point => ({ u: point.u, v: point.v })),
        support: (boxTop.support + boxBottom.support) * 0.5
      },
      corners,
      samples: count,
      sideSupports,
      support,
      confidence: clamp((support - 0.16) / 0.68, 0, 1),
      curvature,
      interiorTrace: {
        top: topSelection.diagnostics,
        left: leftSelection.diagnostics,
        right: rightSelection.diagnostics
      },
      refinedStencilQuad: corners
    };
  }

  function sampleCurve(curve, amount) {
    if (!curve?.length) return null;
    if (curve.length === 1) return { x: curve[0].x, y: curve[0].y };
    const scaled = (Number.isFinite(amount) ? amount : 0) * (curve.length - 1);
    let index = Math.floor(scaled);
    let mix = scaled - index;
    if (index < 0) { mix = scaled; index = 0; }
    if (index >= curve.length - 1) { index = curve.length - 2; mix = scaled - index; }
    const fallback = curve.find(Boolean);
    const first = curve[index] || fallback;
    const second = curve[index + 1] || first;
    if (!first || !second) return null;
    return {
      x: first.x + (second.x - first.x) * mix,
      y: first.y + (second.y - first.y) * mix
    };
  }

  function mapFramePoint(frame, pageU, pageV) {
    const frameU0 = 1.5 / 21;
    const frameU1 = 19.5 / 21;
    const frameV0 = 1.43 / 29.7;
    const frameV1 = 28.43 / 29.7;
    const u = (pageU - frameU0) / (frameU1 - frameU0);
    const v = (pageV - frameV0) / (frameV1 - frameV0);
    const top = sampleCurve(frame.paths.top, u);
    const bottom = sampleCurve(frame.paths.bottom, u);
    const left = sampleCurve(frame.paths.left, v);
    const right = sampleCurve(frame.paths.right, v);
    if (!top || !bottom || !left || !right) return null;
    const sideBase = {
      x: left.x * (1 - u) + right.x * u,
      y: left.y * (1 - u) + right.y * u
    };
    const horizontalGuides = [
      { v: 0, curve: frame.paths.top },
      ...(frame.box?.support >= 0.25 ? [
        { v: 1 - 1 / 27, curve: frame.box.top, taperAtSides: true },
        { v: 1 - 0.5 / 27, curve: frame.box.bottom, taperAtSides: true }
      ] : []),
      { v: 1, curve: frame.paths.bottom }
    ];
    const displacementAt = guide => {
      const curvePoint = sampleCurve(guide.curve, u);
      const guideLeft = sampleCurve(frame.paths.left, guide.v);
      const guideRight = sampleCurve(frame.paths.right, guide.v);
      const guideBase = {
        x: guideLeft.x * (1 - u) + guideRight.x * u,
        y: guideLeft.y * (1 - u) + guideRight.y * u
      };
      const sideTaper = guide.taperAtSides
        ? (() => {
          // Each lower calibration box stops 0.5 cm before the 1.5/19.5 cm
          // outer frame. Its smoothed continuation is useful through the
          // marker gap, but it is not a real observation at the two sides.
          // Fade only that inferred displacement over the exact missing
          // half-centimetre so it can never bend an independently observed
          // left/right border during the warp.
          const amount = clamp(Math.min(u, 1 - u) / (0.5 / 18), 0, 1);
          return amount * amount * (3 - 2 * amount);
        })()
        : 1;
      return {
        x: (curvePoint.x - guideBase.x) * sideTaper,
        y: (curvePoint.y - guideBase.y) * sideTaper
      };
    };
    let before = horizontalGuides[0];
    let after = horizontalGuides[horizontalGuides.length - 1];
    for (let index = 1; index < horizontalGuides.length; index += 1) {
      if (v <= horizontalGuides[index].v) {
        before = horizontalGuides[index - 1];
        after = horizontalGuides[index];
        break;
      }
    }
    const range = Math.max(1e-6, after.v - before.v);
    const mix = clamp((v - before.v) / range, 0, 1);
    const beforeDisplacement = displacementAt(before);
    const afterDisplacement = displacementAt(after);
    return {
      x: sideBase.x + beforeDisplacement.x * (1 - mix) + afterDisplacement.x * mix,
      y: sideBase.y + beforeDisplacement.y * (1 - mix) + afterDisplacement.y * mix
    };
  }

  Light.mapDetectedFrame = mapFramePoint;

  function rotateNormalized(point, rotation) {
    if (rotation === 1) return { u: 1 - point.v, v: point.u };
    if (rotation === 2) return { u: 1 - point.u, v: 1 - point.v };
    if (rotation === 3) return { u: point.v, v: 1 - point.u };
    return { u: point.u, v: point.v };
  }

  function unrotateNormalized(point, rotation) {
    if (rotation === 1) return { u: point.v, v: 1 - point.u };
    if (rotation === 2) return { u: 1 - point.u, v: 1 - point.v };
    if (rotation === 3) return { u: 1 - point.v, v: point.u };
    return { u: point.u, v: point.v };
  }

  function chromaticMarkerKind(r, g, b) {
    const max = Math.max(r, g, b);
    const min = Math.min(r, g, b);
    if (max < 38 || max - min < 8) return null;
    // Pale stencil yellow has R slightly above G and used to be classified as
    // one enormous red component, swallowing the real red reference circle
    // wherever its yellow outline touched a box rail. Require a genuinely red
    // opponent ratio so the four calibration dots stay separate.
    if (r - g > 10 && r - b > 8 && r > g * 1.22 && r > b * 1.16) return "red";
    if (b - r > 7 && b - g > 4 && b > r * 1.05) return "blue";
    if (g - r > 6 && g - b > 10 && g > r * 1.04) return "green";
    return null;
  }

  function findChromaticComponents(width, height, pixels, paperQuad, yellowMask) {
    const size = width * height;
    const categories = new Uint8Array(size);
    const names = [null, "red", "blue", "green"];
    for (let index = 0, offset = 0; index < size; index += 1, offset += 4) {
      const kind = chromaticMarkerKind(pixels[offset], pixels[offset + 1], pixels[offset + 2]);
      categories[index] = kind === "red" ? 1 : kind === "blue" ? 2 : kind === "green" ? 3 : 0;
    }
    const queue = new Int32Array(size);
    const components = [];
    for (let seed = 0; seed < size; seed += 1) {
      const category = categories[seed];
      if (!category) continue;
      let head = 0;
      let tail = 0;
      let sumX = 0;
      let sumY = 0;
      let minimumX = width;
      let maximumX = 0;
      let minimumY = height;
      let maximumY = 0;
      queue[tail++] = seed;
      categories[seed] = 0;
      while (head < tail) {
        const index = queue[head++];
        const x = index % width;
        const y = (index / width) | 0;
        sumX += x;
        sumY += y;
        minimumX = Math.min(minimumX, x);
        maximumX = Math.max(maximumX, x);
        minimumY = Math.min(minimumY, y);
        maximumY = Math.max(maximumY, y);
        for (let dy = -1; dy <= 1; dy += 1) {
          const nextY = y + dy;
          if (nextY < 0 || nextY >= height) continue;
          for (let dx = -1; dx <= 1; dx += 1) {
            const nextX = x + dx;
            if ((!dx && !dy) || nextX < 0 || nextX >= width) continue;
            const next = nextY * width + nextX;
            if (categories[next] !== category) continue;
            categories[next] = 0;
            queue[tail++] = next;
          }
        }
      }
      const boxWidth = maximumX - minimumX + 1;
      const boxHeight = maximumY - minimumY + 1;
      if (tail < 1 || tail > 180 || boxWidth > 28 || boxHeight > 28) continue;
      const source = { x: sumX / tail, y: sumY / tail };
      const normalized = paperQuad
        ? invertBilinear(paperQuad, source)
        : { u: source.x / width, v: source.y / height };
      if (!normalized || (paperQuad && (normalized.u <= -0.18 || normalized.u >= 1.18
        || normalized.v <= -0.18 || normalized.v >= 1.18))) continue;
      const neighborhoodRadius = Math.max(4, Math.ceil(Math.max(boxWidth, boxHeight) * 0.75) + 3);
      let yellowNearby = 0;
      for (let y = Math.max(0, Math.floor(source.y - neighborhoodRadius)); y <= Math.min(height - 1, Math.ceil(source.y + neighborhoodRadius)); y += 1) {
        for (let x = Math.max(0, Math.floor(source.x - neighborhoodRadius)); x <= Math.min(width - 1, Math.ceil(source.x + neighborhoodRadius)); x += 1) {
          if (yellowMask[y * width + x]) yellowNearby += 1;
        }
      }
      components.push({
        ...normalized,
        sourceX: source.x,
        sourceY: source.y,
        kind: names[category],
        count: tail,
        compactness: tail / (boxWidth * boxHeight),
        yellowNearby
      });
    }
    return components;
  }

  function markerAnchoredPaperCandidates(width, height, pixels, yellowMask, paperQuad = null) {
    const canonical = {
      red: 10.125 / 21,
      blue: 10.625 / 21,
      green: 10.875 / 21
    };
    const markerV = 27.68 / 29.7;
    const frameV0 = 1.43 / 29.7;
    const components = findChromaticComponents(width, height, pixels, null, yellowMask);
    const paperCenter = paperQuad?.every(Boolean)
      ? paperQuad.reduce((center, point) => ({
        x: center.x + point.x / 4,
        y: center.y + point.y / 4
      }), { x: 0, y: 0 })
      : null;
    const quality = marker => marker.yellowNearby * 5
      + marker.compactness * Math.min(30, marker.count) * 2
      + Math.min(24, marker.count);
    const byKind = Object.fromEntries(Object.keys(canonical).map(kind => [
      kind,
      components.filter(component => component.kind === kind)
        .filter(component => component.count >= 1 && component.compactness >= 0.08)
        .sort((first, second) => quality(second) - quality(first))
        .slice(0, 56)
    ]));
    if (!byKind.red.length || !byKind.blue.length || !byKind.green.length) return [];

    const darkSupportAt = (x, y, radius) => {
      let dark = 0;
      let total = 0;
      for (let sampleY = Math.max(0, Math.floor(y - radius)); sampleY <= Math.min(height - 1, Math.ceil(y + radius)); sampleY += 1) {
        for (let sampleX = Math.max(0, Math.floor(x - radius)); sampleX <= Math.min(width - 1, Math.ceil(x + radius)); sampleX += 1) {
          const offset = (sampleY * width + sampleX) * 4;
          const r = pixels[offset], g = pixels[offset + 1], b = pixels[offset + 2];
          const maximum = Math.max(r, g, b);
          const minimum = Math.min(r, g, b);
          const luminance = r * 0.2126 + g * 0.7152 + b * 0.0722;
          if (luminance < 118 && (maximum - minimum) / Math.max(1, maximum) < 0.42) dark += 1;
          total += 1;
        }
      }
      return dark / Math.max(1, total);
    };
    const fits = [];
    for (const red of byKind.red) {
      for (const green of byKind.green) {
        const canonicalSpan = canonical.green - canonical.red;
        const axisX = (green.sourceX - red.sourceX) / canonicalSpan;
        const axisY = (green.sourceY - red.sourceY) / canonicalSpan;
        const pageWidth = Math.hypot(axisX, axisY);
        if (pageWidth < Math.min(width, height) * 0.26
          || pageWidth > Math.max(width, height) * 1.35) continue;
        const blueMix = (canonical.blue - canonical.red) / canonicalSpan;
        const expectedBlue = {
          x: red.sourceX + (green.sourceX - red.sourceX) * blueMix,
          y: red.sourceY + (green.sourceY - red.sourceY) * blueMix
        };
        const nearestBlue = byKind.blue.map(blue => ({
          blue,
          error: Math.hypot(blue.sourceX - expectedBlue.x, blue.sourceY - expectedBlue.y)
        })).sort((first, second) => first.error - second.error)[0];
        if (!nearestBlue || nearestBlue.error > Math.max(14, pageWidth * 0.04)) continue;
        // Red and green are the widest-spaced unique pair and therefore give
        // the least quantisation-sensitive scale. Blue is an independent
        // sequence check; it must be nearby but does not pull the axis when a
        // JPEG block shifts its tiny component by a few pixels.
        const origin = {
          x: (red.sourceX - axisX * canonical.red
            + green.sourceX - axisX * canonical.green) * 0.5,
          y: (red.sourceY - axisY * canonical.red
            + green.sourceY - axisY * canonical.green) * 0.5
        };
        const black = {
          x: origin.x + axisX * (10.375 / 21),
          y: origin.y + axisY * (10.375 / 21)
        };
        const blackSupport = darkSupportAt(black.x, black.y, clamp(pageWidth * 0.012, 3, 10));
        if (blackSupport < 0.004) continue;
        const blue = nearestBlue.blue;
        const blueError = nearestBlue.error;
        const markerQuality = quality(red) + quality(blue) + quality(green);
        fits.push({
          axisX,
          axisY,
          origin,
          pageWidth,
          blueError,
          blackSupport,
          score: markerQuality + blackSupport * 500 - blueError * 28,
          markers: { red, blue, green }
        });
      }
    }
    fits.sort((first, second) => second.score - first.score);
    const distinct = fits.filter((fit, index, values) => index === values.findIndex(other =>
      Math.hypot(fit.origin.x - other.origin.x, fit.origin.y - other.origin.y) < 5
      && Math.abs(fit.pageWidth - other.pageWidth) < 12)).slice(0, 14);

    return distinct.map(fit => {
      const axisUnit = { x: fit.axisX / fit.pageWidth, y: fit.axisY / fit.pageWidth };
      const firstNormal = { x: -axisUnit.y, y: axisUnit.x };
      const initialHeight = fit.pageWidth / A4_RATIO;
      const sideEvidence = normal => {
        const occupied = new Uint8Array(64);
        const distances = [];
        for (let index = 0; index < yellowMask.length; index += 1) {
          if (!yellowMask[index]) continue;
          const x = index % width;
          const y = (index / width) | 0;
          const dx = x - fit.origin.x;
          const dy = y - fit.origin.y;
          const u = (dx * axisUnit.x + dy * axisUnit.y) / fit.pageWidth;
          if (Math.min(Math.abs(u - 1.5 / 21), Math.abs(u - 19.5 / 21)) > 0.065) continue;
          const distance = (dx * normal.x + dy * normal.y) / initialHeight;
          if (distance < -0.015 || distance > 1.45) continue;
          distances.push(distance);
          occupied[clamp(Math.floor(distance / 1.45 * occupied.length), 0, occupied.length - 1)] = 1;
        }
        distances.sort((first, second) => first - second);
        const far = distances.length ? distances[Math.floor((distances.length - 1) * 0.985)] : 0;
        const bins = occupied.reduce((sum, value) => sum + value, 0);
        return { score: bins * 3 + far * 18 + Math.min(20, distances.length / 20), far, bins };
      };
      const firstEvidence = sideEvidence(firstNormal);
      const opposite = { x: -firstNormal.x, y: -firstNormal.y };
      const oppositeEvidence = sideEvidence(opposite);
      const markerCenter = {
        x: fit.origin.x + fit.axisX * 0.5,
        y: fit.origin.y + fit.axisY * 0.5
      };
      const paperDirection = paperCenter
        ? (paperCenter.x - markerCenter.x) * firstNormal.x
          + (paperCenter.y - markerCenter.y) * firstNormal.y
        : 0;
      // When the printed yellow has become nearly grey, the first colour-only
      // marker pass can have no side evidence yet. The paper centre still
      // unambiguously says which side of the marker row contains the sheet,
      // preventing a 180-degree candidate with plausible-looking corners.
      const evidenceGap = Math.abs(firstEvidence.score - oppositeEvidence.score);
      const usePaperDirection = paperCenter && Math.max(firstEvidence.score, oppositeEvidence.score) < 12
        && evidenceGap < 5;
      const useFirst = usePaperDirection
        ? paperDirection >= 0
        : firstEvidence.score >= oppositeEvidence.score;
      const up = useFirst ? firstNormal : opposite;
      const evidence = useFirst ? firstEvidence : oppositeEvidence;
      const observedHeight = evidence.far > 0.25
        ? initialHeight * evidence.far / (markerV - frameV0)
        : initialHeight;
      // A steep keystone makes the two long rails leave the affine side band
      // near the top, so their observed extent is a lower bound rather than a
      // reason to collapse the page height. A4 geometry supplies a stable
      // floor while the yellow sides can still extend it for foreshortening.
      const pageHeight = clamp(observedHeight, initialHeight * 0.86, initialHeight * 1.48);
      const down = { x: -up.x * pageHeight, y: -up.y * pageHeight };
      const map = (u, v) => ({
        x: fit.origin.x + fit.axisX * u + down.x * (v - markerV),
        y: fit.origin.y + fit.axisY * u + down.y * (v - markerV)
      });
      return {
        quad: [map(0, 0), map(1, 0), map(1, 1), map(0, 1)],
        score: fit.score + evidence.score,
        diagnostics: {
          pageWidth: fit.pageWidth,
          pageHeight,
          markerScore: fit.score,
          sideBins: evidence.bins,
          blueError: fit.blueError,
          blackSupport: fit.blackSupport,
          directionSource: usePaperDirection ? "paper-centre" : "yellow-rails"
        }
      };
    });
  }

  function guidedRansacLine(points, independentKey, dependentKey, options) {
    if (points.length < 14) return null;
    let seed = (points.length * 2246822519 + Math.round(options.expected * 10000)) >>> 0;
    const randomIndex = () => {
      seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
      return seed % points.length;
    };
    let best = null;
    let bestScore = 0;
    const hypotheses = Math.min(360, Math.max(140, points.length * 2));
    for (let attempt = 0; attempt < hypotheses; attempt += 1) {
      const first = points[randomIndex()];
      const second = points[randomIndex()];
      const delta = second[independentKey] - first[independentKey];
      if (Math.abs(delta) < options.minimumSpan * 0.34) continue;
      const a = (second[dependentKey] - first[dependentKey]) / delta;
      // Coordinates are already normalized through the detected paper quad;
      // stencil borders must therefore stay close to horizontal/vertical.
      // A steeper candidate is almost always handwriting or a neighbouring
      // sheet crossing the expected band (not a perspective effect).
      if (Math.abs(a) > (options.maximumSlope || 0.12)) continue;
      const b = first[dependentKey] - a * first[independentKey];
      const inliers = [];
      let minimum = Infinity;
      let maximum = -Infinity;
      for (const point of points) {
        if (Math.abs(point[dependentKey] - (a * point[independentKey] + b)) > options.tolerance) continue;
        inliers.push(point);
        minimum = Math.min(minimum, point[independentKey]);
        maximum = Math.max(maximum, point[independentKey]);
      }
      const span = maximum - minimum;
      if (span < options.minimumSpan || inliers.length < 12) continue;
      const atCenter = a * 0.5 + b;
      const expectedPenalty = Number.isFinite(options.expected)
        ? 1 + Math.abs(atCenter - options.expected) / (options.expectedTolerance || 0.1)
        : 1;
      const slopePenalty = 1 + Math.abs(a) / (options.slopePenaltyScale || 0.075);
      const anchorPenalty = Number.isFinite(options.anchorIndependent)
        ? 1 + Math.abs(a * options.anchorIndependent + b - options.anchorDependent) / 0.025
        : 1;
      const inwardOffset = Number.isFinite(options.boundaryExpected)
        && Number.isFinite(options.inwardSign)
        ? (atCenter - options.boundaryExpected) * options.inwardSign
        : 0;
      const boundaryPenalty = 1 + Math.max(0, inwardOffset)
        / (options.boundaryTolerance || 0.012);
      const weightedSupport = inliers.reduce((sum, point) => sum + Math.min(3, point.strength || 1), 0);
      const continuityBins = Math.min(120, Math.max(36,
        Math.ceil(span / Math.max(0.006, options.tolerance * 0.7))));
      const occupied = new Uint8Array(continuityBins);
      for (const point of inliers) {
        const bin = clamp(Math.floor((point[independentKey] - minimum)
          / Math.max(1e-6, span) * continuityBins), 0, continuityBins - 1);
        occupied[bin] = 1;
      }
      const continuity = occupied.reduce((sum, value) => sum + value, 0) / continuityBins;
      const continuityWeight = clamp(options.continuityWeight || 0, 0, 0.95);
      const continuityFactor = (1 - continuityWeight)
        + continuityWeight * Math.pow(continuity, 1.35);
      const score = weightedSupport * continuityFactor * (1 + Math.min(1, span / 0.82))
        / (expectedPenalty * slopePenalty * anchorPenalty * boundaryPenalty);
      if (score > bestScore) {
        bestScore = score;
        best = { inliers, atCenter, span, minimum, maximum, continuity };
      }
    }
    if (!best) return null;
    const fitted = robustLine(best.inliers, independentKey, dependentKey);
    return fitted ? {
      ...fitted,
      support: best.inliers.length,
      span: best.span,
      continuity: best.continuity,
      independentMinimum: best.minimum,
      independentMaximum: best.maximum
    } : null;
  }

  function medianNumber(values) {
    if (!values.length) return null;
    const sorted = values.slice().sort((a, b) => a - b);
    return sorted[Math.floor(sorted.length / 2)];
  }

  function estimateCalibrationStrip(yellowPoints, stripLine, tolerance = 0.05) {
    const minimumU = -0.12;
    const maximumU = 1.12;
    const binCount = 180;
    const counts = new Uint16Array(binCount);
    for (const point of yellowPoints) {
      if (point.u < minimumU || point.u > maximumU
        || Math.abs(point.v - (stripLine.a * point.u + stripLine.b)) > tolerance) continue;
      const bin = clamp(Math.floor((point.u - minimumU) / (maximumU - minimumU) * binCount), 0, binCount - 1);
      counts[bin] += 1;
    }
    const smoothed = new Float32Array(binCount);
    let maximumDensity = 0;
    for (let bin = 0; bin < binCount; bin += 1) {
      let sum = 0;
      for (let offset = -2; offset <= 2; offset += 1) {
        const sample = bin + offset;
        if (sample >= 0 && sample < binCount) sum += counts[sample];
      }
      smoothed[bin] = sum;
      maximumDensity = Math.max(maximumDensity, sum);
    }
    const threshold = Math.max(3, maximumDensity * 0.16);
    const segments = [];
    let start = -1;
    let lastActive = -1;
    let density = 0;
    for (let bin = 0; bin <= binCount; bin += 1) {
      const active = bin < binCount && smoothed[bin] >= threshold;
      if (active) {
        if (start < 0) start = bin;
        lastActive = bin;
        density += smoothed[bin];
      }
      if (start >= 0 && (!active && (bin - lastActive > 3 || bin === binCount))) {
        const length = lastActive - start + 1;
        if (length >= binCount * 0.22) segments.push({ start, end: lastActive, length, density });
        start = -1;
        lastActive = -1;
        density = 0;
      }
    }
    const best = segments.sort((a, b) => (b.length * Math.sqrt(b.density)) - (a.length * Math.sqrt(a.density)))[0];
    if (!best) return null;
    const binWidth = (maximumU - minimumU) / binCount;
    const minimum = minimumU + Math.max(0, best.start - 1) * binWidth;
    const maximum = minimumU + Math.min(binCount, best.end + 2) * binWidth;
    const span = maximum - minimum;
    if (span < 0.38 || span > 1.14) return null;
    return {
      minimum,
      maximum,
      center: (minimum + maximum) * 0.5,
      span,
      density: best.density,
      threshold,
      maximumDensity
    };
  }

  function fitMarkerAxis(chromaticPoints, stripLine, stripInterval) {
    const canonical = {
      red: 10.125 / 21,
      blue: 10.625 / 21,
      green: 10.875 / 21
    };
    const candidates = chromaticPoints.filter(point => point.u > 0.2 && point.u < 0.8
      && Math.abs(point.v - (stripLine.a * point.u + stripLine.b)) < 0.075);
    const stripCenter = stripInterval.center;
    const gridScale = clamp(stripInterval.span / (18 / 21), 0.45, 1.45);
    let best = null;
    let bestScore = -Infinity;
    for (let firstIndex = 0; firstIndex < candidates.length; firstIndex += 1) {
      for (let secondIndex = firstIndex + 1; secondIndex < candidates.length; secondIndex += 1) {
        const first = candidates[firstIndex];
        const second = candidates[secondIndex];
        if (first.kind === second.kind) continue;
        const canonicalDelta = canonical[second.kind] - canonical[first.kind];
        const scale = (second.u - first.u) / canonicalDelta;
        if (scale < 0.32 || scale > 2.1) continue;
        if (Math.abs(scale - gridScale) > gridScale * 0.32) continue;
        const offset = (first.u - scale * canonical[first.kind]
          + second.u - scale * canonical[second.kind]) * 0.5;
        const matched = {};
        let residual = 0;
        for (const kind of Object.keys(canonical)) {
          const expected = offset + scale * canonical[kind];
          const sameKind = candidates
            .filter(point => point.kind === kind)
            .map(point => ({ point, distance: Math.abs(point.u - expected) }))
            .sort((a, b) => a.distance - b.distance)[0];
          if (!sameKind || sameKind.distance > 0.024) continue;
          matched[kind] = sameKind.point;
          residual += sameKind.distance;
        }
        const kinds = Object.keys(matched);
        if (kinds.length < 2) continue;
        const markerRows = kinds.map(kind => matched[kind].v
          - (stripLine.a * matched[kind].u + stripLine.b));
        if (Math.max(...markerRows) - Math.min(...markerRows) > 0.012) continue;
        const ordered = kinds
          .map(kind => ({ canonical: canonical[kind], observed: matched[kind].u }))
          .sort((a, b) => a.canonical - b.canonical);
        if (ordered.some((entry, index) => index > 0 && entry.observed <= ordered[index - 1].observed)) continue;
        const compactness = kinds.reduce((sum, kind) => sum + matched[kind].compactness, 0);
        const sizeScore = kinds.reduce((sum, kind) => sum + Math.min(24, matched[kind].count), 0);
        const ringScore = kinds.reduce((sum, kind) => sum + Math.min(20, matched[kind].yellowNearby), 0);
        const center = offset + scale * 0.5;
        if (Math.abs(center - stripCenter) > 0.14) continue;
        const score = kinds.length * 120 + compactness * 18 + sizeScore + ringScore * 5
          - residual * 900 - Math.abs(center - stripCenter) * 620
          - Math.abs(scale - gridScale) * 90;
        if (score > bestScore) {
          bestScore = score;
          best = {
            scale,
            offset,
            centers: matched,
            kinds,
            markerV: medianNumber(kinds.map(kind => matched[kind].v)),
            source: "colours"
          };
        }
      }
    }
    if (best) return best;

    // Severe blur can collapse blue/green into black. The unique red circle
    // still fixes the strip centre; repeated square edges provide its scale.
    const expectedRed = stripCenter + gridScale * (canonical.red - 0.5);
    const redCandidates = candidates.filter(point => point.kind === "red")
      .filter(point => Math.abs(point.u - expectedRed) < 0.14)
      .sort((a, b) => (Math.abs(a.u - expectedRed) * 900 - a.yellowNearby * 2
          - a.compactness * Math.min(30, a.count))
        - (Math.abs(b.u - expectedRed) * 900 - b.yellowNearby * 2
          - b.compactness * Math.min(30, b.count)));
    for (const red of redCandidates) {
      const scale = gridScale;
      const offset = red.u - scale * canonical.red;
      const left = offset + scale * (1.5 / 21);
      const right = offset + scale * (19.5 / 21);
      const center = offset + scale * 0.5;
      if (left < -0.1 || right > 1.1 || right - left < 0.42
        || Math.abs(center - stripCenter) > 0.16) continue;
      return {
        scale,
        offset,
        centers: { red },
        kinds: ["red"],
        markerV: red.v,
        source: "red+grid"
      };
    }
    return null;
  }

  function blackMarkerIsPresent(markerAxis, rotation, paperQuad, pixels, width, height) {
    if (!markerAxis) return false;
    const canonicalPoint = {
      u: markerAxis.offset + markerAxis.scale * (10.375 / 21),
      v: markerAxis.markerV
    };
    const base = unrotateNormalized(canonicalPoint, rotation);
    const source = bilinear(paperQuad, base.u, base.v);
    const radius = Math.max(3, Math.round(Math.min(width, height) * 0.009));
    let dark = 0;
    let total = 0;
    for (let y = Math.max(0, Math.floor(source.y - radius)); y <= Math.min(height - 1, Math.ceil(source.y + radius)); y += 1) {
      for (let x = Math.max(0, Math.floor(source.x - radius)); x <= Math.min(width - 1, Math.ceil(source.x + radius)); x += 1) {
        const offset = (y * width + x) * 4;
        const r = pixels[offset], g = pixels[offset + 1], b = pixels[offset + 2];
        const max = Math.max(r, g, b);
        const min = Math.min(r, g, b);
        const luminance = r * 0.2126 + g * 0.7152 + b * 0.0722;
        if (luminance < 105 && (max - min) / Math.max(1, max) < 0.38) dark += 1;
        total += 1;
      }
    }
    return dark >= Math.max(2, total * 0.015);
  }

  function detectMarkerGuidedStencil(paperQuad, width, height, pixels, yellowMask, options = {}) {
    if (!paperQuad) return null;
    const yellowPoints = [];
    let chromaticPoints = [];
    // Sampling every detected pixel preserves the tiny colour circles, while
    // the low-resolution analysis canvas keeps this bounded and inexpensive.
    for (let index = 0; index < yellowMask.length; index += 1) {
      if (!yellowMask[index]) continue;
      const point = { x: index % width, y: (index / width) | 0 };
      const normalized = invertBilinear(paperQuad, point);
      if (normalized && normalized.u > -0.18 && normalized.u < 1.18
        && normalized.v > -0.18 && normalized.v < 1.18) {
        yellowPoints.push({ ...normalized, strength: yellowMask[index] || 1 });
      }
    }
    chromaticPoints = findChromaticComponents(width, height, pixels, paperQuad, yellowMask);
    if (yellowPoints.length < 80) return null;

    let bestRotation = 0;
    let bestOrientationScore = -1;
    let secondOrientationScore = -1;
    let bestMarkerHits = 0;
    for (let rotation = 0; rotation < 4; rotation += 1) {
      let stripScore = 0;
      let markerHits = 0;
      const markerKinds = new Set();
      for (const raw of yellowPoints) {
        const point = rotateNormalized(raw, rotation);
        if (point.u < -0.06 || point.u > 1.06 || point.v < 0.72 || point.v > 1.12) continue;
        stripScore += point.v >= 0.86 && point.v <= 1.02 ? 2.2 : 0.65;
        if (point.u >= 0.08 && point.u <= 0.92) stripScore += 0.35;
      }
      for (const raw of chromaticPoints) {
        const point = rotateNormalized(raw, rotation);
        if (point.u < 0.38 || point.u > 0.62 || point.v < 0.82 || point.v > 1.06) continue;
        markerHits += Math.min(6, raw.count || 1);
        markerKinds.add(raw.kind);
      }
      const score = stripScore + markerHits * 7 + markerKinds.size * 45;
      if (score > bestOrientationScore) {
        secondOrientationScore = bestOrientationScore;
        bestOrientationScore = score;
        bestRotation = rotation;
        bestMarkerHits = markerHits;
      } else if (score > secondOrientationScore) {
        secondOrientationScore = score;
      }
    }
    if (bestOrientationScore < 90
      || (bestMarkerHits < 2 && bestOrientationScore < secondOrientationScore * 1.08)) return null;

    const oriented = yellowPoints.map(point => ({ ...point, ...rotateNormalized(point, bestRotation) }));
    const orientedChromatic = chromaticPoints.map(point => ({ ...point, ...rotateNormalized(point, bestRotation) }));
    const bottomPoints = oriented.filter(point => point.v >= 0.72 && point.v <= 1.08 && point.u > 0.02 && point.u < 0.98);
    const common = { minimumSpan: 0.42, tolerance: 0.012, maximumSlope: 0.34 };
    const stripLine = guidedRansacLine(bottomPoints, "u", "v", {
      ...common,
      expected: 27.68 / 29.7
    });
    if (!stripLine) return null;
    const tightStripInterval = estimateCalibrationStrip(oriented, stripLine, 0.024);
    const broadStripInterval = estimateCalibrationStrip(oriented, stripLine, 0.05);
    const stripInterval = tightStripInterval || broadStripInterval;
    if (!stripInterval) return null;
    const markerAxes = [tightStripInterval, broadStripInterval]
      .filter(Boolean)
      .map(interval => fitMarkerAxis(orientedChromatic, stripLine, interval))
      .filter(Boolean)
      .filter((axis, index, axes) => index === axes.findIndex(other =>
        Math.abs(other.scale - axis.scale) < 0.01 && Math.abs(other.offset - axis.offset) < 0.01));
    if (!markerAxes.length) return null;
    const blackMarker = markerAxes.some(axis =>
      blackMarkerIsPresent(axis, bestRotation, paperQuad, pixels, width, height));
    const stripAtCenter = stripLine.a * 0.5 + stripLine.b;
    const neutralBaseline = estimatePaperYellowBaseline(width, height, pixels, paperQuad);
    const sampleNeutralRidge = ({ axis, perpendicular, expected, start, end, search = 0.07 }) => {
      const sampleSets = [];
      const sampleCount = 72;
      const step = 0.0024;
      for (let sample = 0; sample < sampleCount; sample += 1) {
        const independent = start + (end - start) * (sample / (sampleCount - 1));
        const candidates = [];
        let cluster = [];
        const finishCluster = () => {
          if (!cluster.length) return;
          cluster.sort((first, second) => second.score - first.score);
          candidates.push(cluster[0]);
          cluster = [];
        };
        for (let delta = -search; delta <= search + step * 0.5; delta += step) {
          const orientedPoint = { [axis]: independent, [perpendicular]: expected + delta };
          const basePoint = unrotateNormalized(orientedPoint, bestRotation);
          const colour = sampleColour(width, height, pixels, paperQuad, basePoint);
          const evidence = isNeutralStencilTone(...colour, neutralBaseline)
            ? localStencilEvidence(basePoint, ...colour, width, height, pixels, paperQuad)
            : null;
          if (!evidence?.neutral) {
            finishCluster();
            continue;
          }
          cluster.push({
            ...orientedPoint,
            strength: clamp(2 + evidence.score * 0.45, 2, 6),
            score: evidence.score
          });
        }
        finishCluster();
        // One candidate per physical ridge and per longitudinal sample keeps
        // the dotted writing grid from winning merely because each dot spans
        // several pixels. A real border contributes at virtually every row.
        sampleSets.push(candidates
          .sort((first, second) => second.score - first.score)
          .slice(0, 10));
      }
      return sampleSets;
    };
    const fitSampledNeutralRidge = (sampleSets, axis, perpendicular,
      expected, lineOptions, inwardSign) => {
      if (sampleSets.filter(set => set.length).length < 20) return null;
      const choices = sampleSets.map((set, sample) => set.length
        ? set
        : [{
            [axis]: sample / Math.max(1, sampleSets.length - 1),
            [perpendicular]: expected,
            strength: 0,
            score: -10,
            missing: true
          }]);
      const costs = choices.map(set => set.map(() => Infinity));
      const parents = choices.map(set => set.map(() => -1));
      choices[0].forEach((choice, index) => {
        const value = choice[perpendicular];
        costs[0][index] = Math.abs(value - expected) * 45
          + Math.max(0, (value - expected) * inwardSign) * 110
          - choice.score * 0.45;
      });
      for (let sample = 1; sample < choices.length; sample += 1) {
        choices[sample].forEach((choice, choiceIndex) => {
          const value = choice[perpendicular];
          const endpointAnchor = sample === choices.length - 1
            ? Math.abs(value - expected) * 620
            : 0;
          const nodeCost = Math.abs(value - expected) * 45
            + Math.max(0, (value - expected) * inwardSign) * 110
            - choice.score * 0.45 + endpointAnchor;
          choices[sample - 1].forEach((previous, previousIndex) => {
            const stepDistance = Math.abs(value - previous[perpendicular]);
            const transition = stepDistance * 720
              + Math.max(0, stepDistance - 0.007) * 2600;
            const cost = costs[sample - 1][previousIndex] + transition + nodeCost;
            if (cost < costs[sample][choiceIndex]) {
              costs[sample][choiceIndex] = cost;
              parents[sample][choiceIndex] = previousIndex;
            }
          });
        });
      }
      let selectedIndex = costs[costs.length - 1]
        .reduce((best, cost, index, values) => cost < values[best] ? index : best, 0);
      const selected = [];
      for (let sample = choices.length - 1; sample >= 0; sample -= 1) {
        const choice = choices[sample][selectedIndex];
        if (!choice.missing) selected.push(choice);
        selectedIndex = parents[sample][selectedIndex];
        if (sample && selectedIndex < 0) selectedIndex = 0;
      }
      if (selected.length < 18) return null;
      const line = guidedRansacLine(selected, axis, perpendicular, {
        ...lineOptions,
        continuityWeight: 0.92,
        expected,
        expectedTolerance: 0.022,
        boundaryExpected: expected,
        inwardSign,
        boundaryTolerance: 0.01
      });
      return line ? { line, points: selected } : null;
    };
    const tryGeometry = ({ leftAnchor, rightAnchor, geometryScale }) => {
      const bottomAtCenter = stripAtCenter + geometryScale * (0.75 / 29.7);
      const topExpected = stripAtCenter - geometryScale * ((27.68 - 1.43) / 29.7);
      const bottomLine = {
        ...stripLine,
        b: stripLine.b + geometryScale * (0.75 / 29.7)
      };
      const leftPoints = oriented.filter(point => Math.abs(point.u - leftAnchor) < 0.29
        && point.v > topExpected - 0.1 && point.v < bottomAtCenter + 0.06);
      const rightPoints = oriented.filter(point => Math.abs(point.u - rightAnchor) < 0.29
        && point.v > topExpected - 0.1 && point.v < bottomAtCenter + 0.06);
      // Perspective and a poor first paper silhouette can change the vertical
      // scale independently of the coloured marker row's horizontal scale.
      // Search the entire upper region for the one long yellow rail instead of
      // synthesising its height from marker spacing.
      const topGap = clamp(geometryScale * 0.34, 0.24, 0.48);
      const topPoints = oriented.filter(point => point.v < stripAtCenter - topGap
        && point.v > -0.2
        && point.u > leftAnchor - 0.14 && point.u < rightAnchor + 0.14);
      const leftLineOptions = {
        ...common,
        minimumSpan: Math.max(options.trustedAxis ? 0.28 : 0.36,
          geometryScale * (options.trustedAxis ? 0.4 : 0.56)),
        expected: null,
        maximumSlope: 0.85,
        slopePenaltyScale: 0.3,
        anchorIndependent: bottomAtCenter,
        anchorDependent: leftAnchor
      };
      const rightLineOptions = {
        ...common,
        minimumSpan: Math.max(options.trustedAxis ? 0.28 : 0.36,
          geometryScale * (options.trustedAxis ? 0.4 : 0.56)),
        expected: null,
        maximumSlope: 0.85,
        slopePenaltyScale: 0.3,
        anchorIndependent: bottomAtCenter,
        anchorDependent: rightAnchor
      };
      const neutralLeftPoints = leftPoints.filter(point => (point.strength || 1) >= 3);
      const neutralRightPoints = rightPoints.filter(point => (point.strength || 1) >= 3);
      const sampledNeutralLeft = neutralLeftPoints.length >= 14
        ? sampleNeutralRidge({
            axis: "v",
            perpendicular: "u",
            expected: leftAnchor,
            start: Math.max(-0.08, topExpected),
            end: Math.min(1.08, bottomAtCenter),
            search: 0.075
          })
        : [];
      const sampledNeutralRight = neutralRightPoints.length >= 14
        ? sampleNeutralRidge({
            axis: "v",
            perpendicular: "u",
            expected: rightAnchor,
            start: Math.max(-0.08, topExpected),
            end: Math.min(1.08, bottomAtCenter),
            search: 0.075
          })
        : [];
      const sampledLeftFit = fitSampledNeutralRidge(sampledNeutralLeft, "v", "u",
        leftAnchor, leftLineOptions, 1);
      let leftLine = sampledLeftFit?.line || null;
      leftLine ||= neutralLeftPoints.length >= 14
        ? guidedRansacLine(neutralLeftPoints, "v", "u", {
            ...leftLineOptions,
            continuityWeight: 0.85,
            expected: leftAnchor,
            expectedTolerance: 0.022,
            boundaryExpected: leftAnchor,
            inwardSign: 1,
            boundaryTolerance: 0.01
          })
        : null;
      leftLine ||= guidedRansacLine(leftPoints, "v", "u", leftLineOptions);
      const sampledRightFit = fitSampledNeutralRidge(sampledNeutralRight, "v", "u",
        rightAnchor, rightLineOptions, -1);
      let rightLine = sampledRightFit?.line || null;
      rightLine ||= neutralRightPoints.length >= 14
        ? guidedRansacLine(neutralRightPoints, "v", "u", {
            ...rightLineOptions,
            continuityWeight: 0.85,
            expected: rightAnchor,
            expectedTolerance: 0.022,
            boundaryExpected: rightAnchor,
            inwardSign: -1,
            boundaryTolerance: 0.01
          })
        : null;
      rightLine ||= guidedRansacLine(rightPoints, "v", "u", rightLineOptions);
      const topLineOptions = {
        ...common,
        tolerance: 0.018,
        minimumSpan: Math.max(options.trustedAxis ? 0.25 : 0.36,
          geometryScale * (options.trustedAxis ? 0.32 : 0.5)),
        expected: null,
        slopePenaltyScale: 0.2
      };
      // When exposure or white balance has removed the stencil hue, both the
      // grey frame and a long blue handwriting stroke can satisfy the generic
      // ridge test. Neutral recovery pixels carry strength 3; fit those first
      // and let the uniquely identified calibration strip supply only a soft
      // vertical prior. This rejects the first line of writing without
      // constraining ordinary yellow scans, whose perspective/curvature is
      // still determined solely from observed colour evidence.
      const neutralTopPoints = topPoints.filter(point => (point.strength || 1) >= 3);
      let topLine = neutralTopPoints.length >= 14
        ? guidedRansacLine(neutralTopPoints, "u", "v", {
            ...topLineOptions,
            expected: topExpected,
            expectedTolerance: 0.01,
            maximumSlope: 0.08
          })
        : null;
      const neutralTopAccepted = !!topLine;
      topLine ||= guidedRansacLine(topPoints, "u", "v", topLineOptions);
      const sideMatchesAnchor = (line, anchor) => line
        // The coloured circles and repeated cells locate the two bottom
        // intersections much more reliably than a long yellow line when
        // several ruled sheets overlap. Never let a neighbouring sheet pull
        // a side tens of pixels away from that calibration-strip anchor.
        && Math.abs(line.a * bottomAtCenter + line.b - anchor) <= (options.trustedAxis ? 0.055 : 0.034)
        && Math.abs(line.a * topExpected + line.b - anchor) <= (options.trustedAxis ? 0.5 : 0.1);
      if (!sideMatchesAnchor(leftLine, leftAnchor)) leftLine = null;
      if (!sideMatchesAnchor(rightLine, rightAnchor)) rightLine = null;
      if (topLine && topLine.a * 0.5 + topLine.b > stripAtCenter - topGap * 0.82) topLine = null;
      const observedLeft = !!leftLine;
      const observedRight = !!rightLine;
      const observedTop = !!topLine;
      if (!leftLine && rightLine) {
        leftLine = { a: rightLine.a, b: leftAnchor - rightLine.a * bottomAtCenter, count: 0, support: 0, span: 0 };
      }
      if (!rightLine && leftLine) {
        rightLine = { a: leftLine.a, b: rightAnchor - leftLine.a * bottomAtCenter, count: 0, support: 0, span: 0 };
      }
      leftLine ||= { a: 0, b: leftAnchor, count: 0, support: 0, span: 0 };
      rightLine ||= { a: 0, b: rightAnchor, count: 0, support: 0, span: 0 };
      // A guessed top edge creates a confident-looking crop through the middle
      // of a page. Keep looking at the next geometry candidate instead.
      if (!topLine) return null;
      const normalizedQuad = [
        lineIntersection(leftLine, topLine),
        lineIntersection(rightLine, topLine),
        lineIntersection(rightLine, bottomLine),
        lineIntersection(leftLine, bottomLine)
      ].map(point => point ? { u: point.x, v: point.y } : null);
      if (!normalizedQuad.every(Boolean)) return null;
      const canonicalQuad = normalizedQuad.map(point => ({ x: point.u, y: point.v }));
      if (!isPlausibleQuad(canonicalQuad, 1, 1, {
        allowOutside: true,
        minAreaRatio: options.trustedAxis ? 0.3 : 0.48,
        maxOppositeRatio: options.trustedAxis ? 3 : 1.4
      })) return null;
      const centerValues = {
        left: leftLine.a * 0.5 + leftLine.b,
        right: rightLine.a * 0.5 + rightLine.b,
        top: topLine.a * 0.5 + topLine.b,
        bottom: bottomLine.a * 0.5 + bottomLine.b
      };
      if (centerValues.left < (options.trustedAxis ? -0.3 : -0.16)
        || centerValues.left > (options.trustedAxis ? 0.5 : 0.32)
        || centerValues.right < (options.trustedAxis ? 0.5 : 0.68)
        || centerValues.right > (options.trustedAxis ? 1.3 : 1.16)
        || centerValues.top < (options.trustedAxis ? -0.5 : -0.2)
        || centerValues.top > (options.trustedAxis ? 0.5 : 0.42)
        || centerValues.bottom < (options.trustedAxis ? 0.6 : 0.72)
        || centerValues.bottom > (options.trustedAxis ? 1.3 : 1.12)) return null;
      const observedCount = Number(observedLeft) + Number(observedRight) + Number(observedTop);
      const evidenceScore = observedCount * 180
        + (leftLine.support || 0) + (rightLine.support || 0) + (topLine.support || 0)
        + ((leftLine.span || 0) + (rightLine.span || 0) + (topLine.span || 0)) * 90;
      return {
        normalizedQuad, leftLine, rightLine, topLine, evidenceScore, observedCount,
        leftAnchor, rightAnchor, geometryScale, topExpected,
        neutralTopPointCount: neutralTopPoints.length,
        topLineSource: neutralTopAccepted ? "neutral-recovery" : "chromatic",
        neutralGuides: sampledLeftFit && sampledRightFit ? {
          left: sampledLeftFit.points,
          right: sampledRightFit.points
        } : null
      };
    };

    // Prefer the unique colour sequence. If compression erases some dots, the
    // independently measured yellow strip remains a safe fallback.
    const markerGeometryCandidates = markerAxes.map(axis => ({
      leftAnchor: axis.offset + axis.scale * (1.5 / 21),
      rightAnchor: axis.offset + axis.scale * (19.5 / 21),
      geometryScale: axis.scale
    }));
    const intervalGeometryCandidates = [];
    for (const interval of [tightStripInterval, broadStripInterval]) {
      if (!interval) continue;
      intervalGeometryCandidates.push({
        leftAnchor: interval.minimum,
        rightAnchor: interval.maximum,
        geometryScale: interval.span / (18 / 21)
      });
    }
    const bestGeometry = candidates => candidates
      .map(tryGeometry)
      .filter(Boolean)
      .sort((first, second) => second.evidenceScore - first.evidenceScore)[0] || null;
    // A neighbouring ruled page may contain longer yellow lines than the
    // target. The asymmetric red/black/blue/green sequence is unique, so its
    // calibrated width is authoritative whenever it produces a plausible
    // complete frame; strip-only extents are strictly a blur fallback.
    const geometry = bestGeometry(markerGeometryCandidates)
      || bestGeometry(intervalGeometryCandidates);
    if (!geometry) return null;
    const { normalizedQuad, leftLine, rightLine, topLine } = geometry;

    const stencilQuad = normalizedQuad.map(point => {
      const base = unrotateNormalized(point, bestRotation);
      return bilinear(paperQuad, base.u, base.v);
    });
    const guideToSource = point => {
      const base = unrotateNormalized(point, bestRotation);
      return bilinear(paperQuad, base.u, base.v);
    };
    const support = leftLine.support + rightLine.support + topLine.support + stripLine.support;
    return {
      stencilQuad,
      neutralGuides: geometry.neutralGuides ? {
        left: geometry.neutralGuides.left.map(guideToSource),
        right: geometry.neutralGuides.right.map(guideToSource)
      } : null,
      confidence: Math.max(geometry.neutralGuides ? 0.94 : 0,
        clamp(0.58 + support / Math.max(900, yellowPoints.length * 2.2)
          + Math.min(0.14, bestMarkerHits * 0.01) + (blackMarker ? 0.04 : 0), 0, 1)),
      rotation: bestRotation,
      markerHits: bestMarkerHits,
      calibration: {
        stripInterval,
        stripAtCenter,
        leftAnchor: geometry.leftAnchor,
        rightAnchor: geometry.rightAnchor,
        geometryScale: geometry.geometryScale,
        leftLine: { a: geometry.leftLine.a, b: geometry.leftLine.b, support: geometry.leftLine.support || 0 },
        rightLine: { a: geometry.rightLine.a, b: geometry.rightLine.b, support: geometry.rightLine.support || 0 },
        topLine: {
          a: geometry.topLine.a,
          b: geometry.topLine.b,
          support: geometry.topLine.support || 0,
          span: geometry.topLine.span || 0,
          minimum: geometry.topLine.independentMinimum,
          maximum: geometry.topLine.independentMaximum,
          source: geometry.topLineSource,
          expected: geometry.topExpected,
          neutralPointCount: geometry.neutralTopPointCount
        },
        markerAxes: markerAxes.map(axis => ({
          scale: axis.scale,
          offset: axis.offset,
          markerV: axis.markerV,
          kinds: axis.kinds,
          source: axis.source
        }))
      }
    };
  }

  function detectPageAtScale(source, analysisMaximum) {
    const analysis = createScaledCanvas(source, analysisMaximum);
    const { canvas, context, scale } = analysis;
    const { width, height } = canvas;
    const pixels = context.getImageData(0, 0, width, height).data;
    const paperQuad = detectPaperQuad(width, height, pixels);
    const paperYellowBaseline = estimatePaperYellowBaseline(width, height, pixels, paperQuad);
    const mask = new Uint8Array(width * height);
    const traceMask = new Uint8Array(width * height);
    let yellowPixels = 0;
    for (let i = 0, p = 0; i < pixels.length; i += 4, p += 1) {
      const strictYellow = isYellow(pixels[i], pixels[i + 1], pixels[i + 2]);
      let yellow = strictYellow;
      let normalized = null;
      if (!strictYellow && paperQuad
        && isFaintYellow(pixels[i], pixels[i + 1], pixels[i + 2], paperYellowBaseline)) {
        normalized = invertBilinear(paperQuad, {
          x: p % width,
          y: (p / width) | 0
        });
        yellow = hasLocalStencilContrast(normalized,
          pixels[i], pixels[i + 1], pixels[i + 2],
          width, height, pixels, paperQuad);
      }
      if (yellow) {
        const strength = 1;
        mask[p] = strength;
        if (strictYellow || hasPaperOnBothSides(normalized,
          pixels[i], pixels[i + 1], pixels[i + 2],
          width, height, pixels, paperQuad)) {
          traceMask[p] = strength;
        }
        yellowPixels += 1;
      }
    }

    let markerPaperCandidates = markerAnchoredPaperCandidates(
      width, height, pixels, mask, paperQuad);
    let neutralBandDiagnostics = null;
    if (markerPaperCandidates.length) {
      // The first pass intentionally uses a strict colour mask so it can find
      // the unique marker sequence without flooding on cream paper. Once that
      // sequence supplies the target sheet's own coordinate system, perform a
      // second narrow-band pass around its expected frame and calibration
      // rails. This recovers the real pale #f0db4c stencil even when the paper
      // silhouette belongs to several touching sheets.
      for (const candidate of markerPaperCandidates.slice(0, 1)) {
        const baseline = estimatePaperYellowBaseline(width, height, pixels, candidate.quad);
        const chromaticEvidenceThreshold = Math.max(420, width * height * 0.0011);
        let chromaticBandEvidence = 0;
        for (let pixel = 0; pixel < mask.length; pixel += 1) {
          if (!mask[pixel]) continue;
          const normalized = invertBilinear(candidate.quad, {
            x: pixel % width,
            y: (pixel / width) | 0
          });
          if (!stencilReferenceBandAxes(normalized)) continue;
          const colourOffset = pixel * 4;
          const r = pixels[colourOffset];
          const g = pixels[colourOffset + 1];
          const b = pixels[colourOffset + 2];
          const maximum = Math.max(r, g, b);
          const minimum = Math.min(r, g, b);
          const saturation = maximum ? (maximum - minimum) / maximum : 0;
          // Count actual colour, not every pixel admitted by the first faint
          // pass. A grey printed rail can have a tiny positive yellow opponent
          // value and was previously misreported as healthy chromatic support,
          // preventing the neutral recovery path from ever activating.
          if (saturation >= 0.1
            && (isYellow(r, g, b) || isFaintYellow(r, g, b, baseline))) {
            chromaticBandEvidence += 1;
            // Once a complete coloured frame is proven, its exact pixel count
            // cannot change the decision. Stop early instead of scanning the
            // rest of a multi-megapixel image merely to reject grey recovery.
            if (chromaticBandEvidence >= chromaticEvidenceThreshold) break;
          }
        }
        // Neutral ridges are a recovery path, not extra evidence on an already
        // healthy colour scan. Enabling them only when chromatic frame support
        // is genuinely sparse prevents grey grid dots or writing from pulling
        // a well-detected yellow rail away from its real position.
        const neutralStripSupport = chromaticBandEvidence < chromaticEvidenceThreshold
          ? neutralCalibrationSupport(width, height, pixels, candidate.quad, baseline)
          : 0;
        const allowNeutralStencil = chromaticBandEvidence
          < chromaticEvidenceThreshold
          && neutralStripSupport >= 0.1;
        const sampleChromaticRail = (axis, fixed, start, end) => {
          const samples = 48;
          let hits = 0;
          for (let sample = 0; sample < samples; sample += 1) {
            const along = start + (end - start) * (sample / (samples - 1));
            let hit = false;
            for (let offsetStep = -3; offsetStep <= 3 && !hit; offsetStep += 1) {
              const normalized = axis === "u"
                ? { u: along, v: fixed + offsetStep * 0.006 }
                : { u: fixed + offsetStep * 0.006, v: along };
              const source = bilinear(candidate.quad, normalized.u, normalized.v);
              const x = clamp(Math.round(source.x), 0, width - 1);
              const y = clamp(Math.round(source.y), 0, height - 1);
              hit = !!mask[y * width + x];
            }
            if (hit) hits += 1;
          }
          return hits / samples;
        };
        const chromaticRailSupports = chromaticBandEvidence >= chromaticEvidenceThreshold
          ? [
              sampleChromaticRail("u", 1.43 / 29.7, 1.5 / 21, 19.5 / 21),
              sampleChromaticRail("v", 19.5 / 21, 1.43 / 29.7, 28.43 / 29.7),
              sampleChromaticRail("u", 28.43 / 29.7, 1.5 / 21, 19.5 / 21),
              sampleChromaticRail("v", 1.5 / 21, 1.43 / 29.7, 28.43 / 29.7)
            ]
          : [];
        const visibleChromaticRails = chromaticRailSupports
          .filter(support => support >= 0.34).length;
        neutralBandDiagnostics = {
          chromaticBandEvidence,
          threshold: chromaticEvidenceThreshold,
          neutralStripSupport,
          chromaticRailSupports,
          enabled: allowNeutralStencil
        };
        candidate.diagnostics.neutralBand = neutralBandDiagnostics;
        // If two long chromatic rails are genuinely hidden, a broad faint-hue
        // pass cannot restore them and only delays the marker/box reconstruction.
        // Complete or curved frames retain the full refinement so their
        // sub-pixel mesh precision is unchanged.
        if (chromaticBandEvidence >= chromaticEvidenceThreshold
          && visibleChromaticRails <= 2) continue;
        for (let offset = 0, pixel = 0; offset < pixels.length; offset += 4, pixel += 1) {
          const r = pixels[offset], g = pixels[offset + 1], b = pixels[offset + 2];
          if (mask[pixel]) {
            // The broad first pass may already have labelled a grey rail as
            // faint yellow. Revisit those pixels after the marker coordinate
            // system is known and promote genuine neutral ridges so the line
            // fitter can prefer them over handwriting.
            if (allowNeutralStencil && isNeutralStencilTone(r, g, b, baseline)) {
              const normalized = invertBilinear(candidate.quad, {
                x: pixel % width,
                y: (pixel / width) | 0
              });
              const evidence = localStencilEvidence(normalized,
                r, g, b, width, height, pixels, candidate.quad);
              if (evidence.neutral) {
                mask[pixel] = 3;
                if (traceMask[pixel]) traceMask[pixel] = 3;
              }
            }
            continue;
          }
          const faintYellow = isFaintYellow(r, g, b, baseline);
          const neutralStencil = allowNeutralStencil
            && !faintYellow
            && isNeutralStencilTone(r, g, b, baseline);
          if (!faintYellow && !neutralStencil) continue;
          const normalized = invertBilinear(candidate.quad, {
            x: pixel % width,
            y: (pixel / width) | 0
          });
          const evidence = localStencilEvidence(normalized,
            r, g, b, width, height, pixels, candidate.quad);
          if (!evidence.accepted || (neutralStencil && !evidence.neutral)) continue;
          // A nearly grey printed rail can still satisfy the very permissive
          // faint-yellow hue test (for example #b9b7ae). Classify by the
          // measured ridge itself, rather than by which colour predicate was
          // reached first, so geometry can distinguish recovered neutral
          // stencil evidence from ordinary chromatic content.
          const recoveryStrength = allowNeutralStencil && evidence.neutral ? 3 : 1;
          mask[pixel] = recoveryStrength;
          if (hasPaperOnBothSides(normalized,
            r, g, b,
            width, height, pixels, candidate.quad)) {
            traceMask[pixel] = recoveryStrength;
          }
          yellowPixels += 1;
        }
      }
      const refinedMarkerPaperCandidates = markerAnchoredPaperCandidates(
        width, height, pixels, mask, paperQuad);
      // The strict first pass already proves the unique colour sequence. A
      // heavily tinted photo can make the optional faint-line augmentation
      // too sparse for a second side fit; never throw away the valid marker
      // coordinate system merely because that refinement produced no result.
      if (refinedMarkerPaperCandidates.length) {
        if (neutralBandDiagnostics) refinedMarkerPaperCandidates.forEach(candidate => {
          candidate.diagnostics.neutralBand = neutralBandDiagnostics;
        });
        markerPaperCandidates = refinedMarkerPaperCandidates;
      }
    }

    const leftPoints = [];
    const rightPoints = [];
    const topPoints = [];
    const bottomPoints = [];
    for (let y = 0; y < height; y += 1) {
      let first = -1, last = -1, count = 0;
      const offset = y * width;
      for (let x = 0; x < width; x += 1) {
        if (!mask[offset + x]) continue;
        if (first < 0) first = x;
        last = x;
        count += 1;
      }
      if (count >= 1) {
        if (first < width * 0.52) leftPoints.push({ x: first, y });
        if (last > width * 0.48) rightPoints.push({ x: last, y });
      }
    }
    for (let x = 0; x < width; x += 1) {
      let first = -1, last = -1, count = 0;
      for (let y = 0; y < height; y += 1) {
        if (!mask[y * width + x]) continue;
        if (first < 0) first = y;
        last = y;
        count += 1;
      }
      if (count >= 1) {
        if (first < height * 0.52) topPoints.push({ x, y: first });
        if (last > height * 0.48) bottomPoints.push({ x, y: last });
      }
    }

    const lineTolerance = Math.max(2.1, Math.min(width, height) * 0.0055);
    const leftLine = ransacLine(leftPoints, "y", "x", height * 0.3, lineTolerance);
    const rightLine = ransacLine(rightPoints, "y", "x", height * 0.3, lineTolerance);
    const topLine = ransacLine(topPoints, "x", "y", width * 0.3, lineTolerance);
    const bottomLine = ransacLine(bottomPoints, "x", "y", width * 0.3, lineTolerance);
    let stencilQuad = [
      lineIntersection(leftLine, topLine),
      lineIntersection(rightLine, topLine),
      lineIntersection(rightLine, bottomLine),
      lineIntersection(leftLine, bottomLine)
    ];

    let confidence = 0;
    if (stencilQuad.every(Boolean)) {
      const topWidth = Math.hypot(stencilQuad[1].x - stencilQuad[0].x, stencilQuad[1].y - stencilQuad[0].y);
      const bottomWidth = Math.hypot(stencilQuad[2].x - stencilQuad[3].x, stencilQuad[2].y - stencilQuad[3].y);
      const leftHeight = Math.hypot(stencilQuad[3].x - stencilQuad[0].x, stencilQuad[3].y - stencilQuad[0].y);
      const rightHeight = Math.hypot(stencilQuad[2].x - stencilQuad[1].x, stencilQuad[2].y - stencilQuad[1].y);
      const plausible = Math.min(topWidth, bottomWidth) > width * 0.35
        && Math.min(leftHeight, rightHeight) > height * 0.42
        && isPlausibleQuad(stencilQuad, width, height, { minAreaRatio: 0.16 });
      if (plausible) {
        confidence = Math.min(1, yellowPixels / Math.max(1, width * height * 0.004));
      }
    }

    // The axis-aligned border fit can look numerically perfect on a page that
    // is rotated 90 degrees while actually swapping its long and short sides.
    // Always let the asymmetric calibration strip resolve orientation first.
    const markerAttempts = markerPaperCandidates.map(candidate => ({
      paper: candidate.quad,
      source: "marker-affine",
      diagnostics: candidate.diagnostics
    }));
    if (paperQuad) markerAttempts.push({ paper: paperQuad, source: "paper", diagnostics: null });
    const resolvedMarkerAttempts = markerAttempts.map(attempt => {
      const guided = detectMarkerGuidedStencil(attempt.paper, width, height, pixels, mask, {
        trustedAxis: attempt.source === "marker-affine"
      });
      if (!guided) return null;
      // The coloured sequence plus the two calibration boxes fixes the target
      // even when pale neighbouring sheets merge into one paper silhouette.
      // Trace each plausible colour-anchored coordinate system and retain the
      // one whose four frame rails and both lower rails agree best.
      let traceQuad = guided.stencilQuad;
      const neutralRecovery = guided.calibration?.topLine?.source === "neutral-recovery";
      let frame = traceYellowFrame(traceQuad, width, height, mask, false,
        attempt.source === "marker-affine", traceMask, neutralRecovery, guided.neutralGuides);
      // Re-seeding a neutral trace from its own result can ratchet the top
      // corridor inward until it lands on the first handwritten line. The
      // marker-calibrated seed is the stronger observation in this mode; one
      // curved trace is sufficient and keeps that absolute reference.
      for (let pass = 0; !neutralRecovery && frame?.refinedStencilQuad && pass < 2; pass += 1) {
        const nextQuad = frame.refinedStencilQuad;
        const movement = Math.max(...nextQuad.map((point, index) => Math.hypot(
          point.x - traceQuad[index].x,
          point.y - traceQuad[index].y
        )));
        if (movement < 0.7) break;
        const retraced = traceYellowFrame(nextQuad, width, height, mask,
          false, false, traceMask, neutralRecovery, guided.neutralGuides);
        if (!retraced || retraced.support + 0.08 < frame.support) break;
        traceQuad = nextQuad;
        frame = retraced;
      }
      const score = (frame?.support || 0) * 2
        + (frame?.box?.support || 0) * 1.25
        + guided.confidence * 0.45
        // Marker coordinates are valuable when the paper silhouette is weak,
        // but only as a small tie-breaker. A larger fixed bonus used to beat a
        // visibly stronger four-rail paper fit and follow a binder edge.
        + (attempt.source === "marker-affine" ? 0.02 : 0);
      return { guided, frame, score, ...attempt };
    }).filter(Boolean).sort((first, second) => second.score - first.score);
    // A colour-anchored coordinate system is independent of the pale-paper
    // segmentation and therefore remains trustworthy when several sheets
    // touch. Prefer it whenever it can explain a substantial part of both the
    // outer frame and the calibration boxes; the paper fit is only a fallback
    // for photos where one of the coloured references is genuinely lost.
    const reliableAttempt = attempt => {
      const frame = attempt.frame;
      if (!frame) return false;
      // Traced rails may be individually convincing while belonging to two
      // different sheets. Extrapolating that mixture creates the conspicuous
      // black triangle / imaginary corner seen in real camera photos. A page
      // corner just outside the photo is normal; one far outside it is not an
      // observation and must never be promoted to a confident stencil crop.
      const resolvedStencil = frame.refinedStencilQuad || attempt.guided?.stencilQuad;
      const resolvedPage = resolvedStencil ? extrapolateStencil(resolvedStencil) : null;
      if (!quadFitsObservedImage(resolvedPage, width, height)) return false;
      const supportedSides = (frame.sideSupports || []).filter(value => value >= 0.42).length;
      const ordinaryFrame = supportedSides >= 3
        && frame.support >= 0.54
        && (frame.box?.support || 0) >= 0.42
        && (frame.evidence?.points?.length || 0) >= 40;
      if (ordinaryFrame) return true;
      // Two genuinely occluded rails are still recoverable when the complete
      // red/black/blue/green sequence and the lower square box independently
      // fix the page coordinate system. Keep this exception chromatic-only:
      // neutral recovery must observe three rails, so grey handwriting can
      // never manufacture the two missing sides of a confident-looking crop.
      const completeColourAxis = (attempt.guided?.calibration?.markerAxes || [])
        .some(axis => ["red", "blue", "green"].every(kind => axis.kinds?.includes(kind)));
      const chromaticOcclusion = attempt.source === "marker-affine"
        && attempt.guided?.calibration?.topLine?.source === "chromatic"
        && completeColourAxis
        && (attempt.diagnostics?.blackSupport || 0) >= 0.004
        && attempt.guided.confidence >= 0.72
        && supportedSides >= 2
        && frame.support >= 0.42
        && (frame.box?.support || 0) >= 0.58
        && (frame.evidence?.points?.length || 0) >= 40;
      return chromaticOcclusion;
    };
    // Attempts are already sorted by independent frame + lower-box evidence.
    // Do not force a weaker marker-affine fit ahead of a stronger paper fit:
    // that old preference could reverse the page when the marker row was
    // partially occluded.
    const preferredMarkerAttempt = resolvedMarkerAttempts.find(attempt => reliableAttempt(attempt))
      || resolvedMarkerAttempts[0]
      || null;
    const markerResolved = preferredMarkerAttempt && reliableAttempt(preferredMarkerAttempt)
      ? preferredMarkerAttempt
      : null;
    const markerGuided = markerResolved?.guided || null;
    const scaleFrame = frame => frame ? {
      ...frame,
      paths: Object.fromEntries(Object.entries(frame.paths).map(([name, points]) => [
        name,
        points.map(point => ({ x: point.x / scale, y: point.y / scale }))
      ])),
      corners: frame.corners.map(point => ({ x: point.x / scale, y: point.y / scale })),
      refinedStencilQuad: frame.refinedStencilQuad?.map(point => ({ x: point.x / scale, y: point.y / scale })) || null,
      evidence: frame.evidence ? {
        points: frame.evidence.points.map(point => ({ x: point.x / scale, y: point.y / scale })),
        normalized: frame.evidence.normalized.map(point => ({ u: point.u, v: point.v }))
      } : null,
      box: frame.box ? {
        ...frame.box,
        top: frame.box.top.map(point => ({ x: point.x / scale, y: point.y / scale })),
        bottom: frame.box.bottom.map(point => ({ x: point.x / scale, y: point.y / scale })),
        outer: frame.box.outer.map(point => ({ x: point.x / scale, y: point.y / scale })),
        points: frame.box.points.map(point => ({ x: point.x / scale, y: point.y / scale }))
      } : null
    } : null;
    const finish = result => {
      canvas.width = 0;
      canvas.height = 0;
      return result;
    };
    if (markerGuided) {
      const tracedFrame = markerResolved.frame;
      const resolvedStencil = tracedFrame?.refinedStencilQuad || markerGuided.stencilQuad;
      stencilQuad = resolvedStencil.map(point => ({ x: point.x / scale, y: point.y / scale }));
      const supportedSides = (tracedFrame?.sideSupports || []).filter(value => value >= 0.42).length;
      const observedPaper = markerResolved.paper?.map(point => ({ x: point.x / scale, y: point.y / scale })) || null;
      // With one rail missing, keep the accurately detected three-sided mesh,
      // but take the physical sheet corners from the marker/paper observation
      // instead of extending a synthetic rail into the background.
      const extrapolatedPage = extrapolateStencil(stencilQuad);
      const resolvedPage = supportedSides === 4 || !observedPaper
        ? extrapolatedPage
        : observedPaper;
      return finish({
        pageQuad: resolvedPage,
        paperCandidate: paperQuad?.map(point => ({ x: point.x / scale, y: point.y / scale })) || null,
        markerCalibration: {
          coordinateSource: markerResolved.source,
          coordinateDiagnostics: markerResolved.diagnostics
        },
        stencilQuad,
        frame: scaleFrame(tracedFrame),
        confidence: markerGuided.confidence,
        method: "marker-guided"
      });
    }
    if (confidence >= 0.24) {
      const tracedFrame = traceYellowFrame(stencilQuad, width, height, mask, true, false, traceMask);
      const resolvedStencil = tracedFrame?.refinedStencilQuad || stencilQuad;
      stencilQuad = resolvedStencil.map(point => ({ x: point.x / scale, y: point.y / scale }));
      return finish({
        pageQuad: extrapolateStencil(stencilQuad),
        paperCandidate: paperQuad?.map(point => ({ x: point.x / scale, y: point.y / scale })) || null,
        stencilQuad,
        frame: scaleFrame(tracedFrame),
        confidence,
        method: "stencil"
      });
    }
    if (paperQuad) {
      const hasStencilSignal = yellowPixels >= width * height * 0.00045;
      return finish({
        pageQuad: paperQuad.map(point => ({ x: point.x / scale, y: point.y / scale })),
        stencilQuad: null,
        frame: null,
        confidence: hasStencilSignal ? 0.35 : 0,
        method: "paper"
      });
    }
    return finish({ pageQuad: fallbackPageQuad(source), stencilQuad: null, frame: null, confidence: 0, method: "fallback" });
  }

  Light.detectPage = function detectPage(source) {
    const quick = detectPageAtScale(source, ANALYSIS_FAST_MAX);
    const frame = quick.frame;
    const finiteQuickQuad = quick.pageQuad?.length === 4
      && quick.pageQuad.every(point => Number.isFinite(point?.x) && Number.isFinite(point?.y));
    const needsDetailedPass = !finiteQuickQuad
      || quick.method === "fallback"
      || (quick.method === "paper" && quick.confidence > 0)
      || (quick.method === "marker-guided"
        && (quick.confidence < 0.58
          || (frame?.support || 0) < 0.42
          || (frame?.box?.support || 0) < 0.42
          // Strong paper curl is precisely the case where the extra 160 px
          // of analysis materially improves the physical corner fit. Keep
          // ordinary scans on the fast path, but never accept a coarse trace
          // whose measured bend is already this large.
          || (frame?.curvature || 0) > 0.045
          // Sparse black-marker support is an independent warning that the
          // affine coordinate basis was inferred near the resolution limit.
          // This catches curled or shadowed sheets whose coarse curvature can
          // otherwise look deceptively smooth.
          || (quick.markerCalibration?.coordinateDiagnostics?.blackSupport ?? 1) < 0.05))
      || (quick.method === "stencil"
        && (quick.confidence < 0.72
          || (frame?.support || 0) < 0.78
          // Plain bordered paper has no calibration boxes. A strong complete
          // outer frame is already authoritative and must not pay for a
          // redundant second pass merely because those optional rails are
          // absent.
          || ((frame?.box?.support || 0) < 0.72
            && (frame?.support || 0) < 0.9)));
    return needsDetailedPass
      ? detectPageAtScale(source, ANALYSIS_MAX)
      : quick;
  };

  function drawTriangle(context, source, src, dst) {
    const [s0, s1, s2] = src;
    const [d0, d1, d2] = dst;
    const denominator = s0.x * (s1.y - s2.y) + s1.x * (s2.y - s0.y) + s2.x * (s0.y - s1.y);
    if (Math.abs(denominator) < 1e-5) return;
    const a = (d0.x * (s1.y - s2.y) + d1.x * (s2.y - s0.y) + d2.x * (s0.y - s1.y)) / denominator;
    const b = (d0.y * (s1.y - s2.y) + d1.y * (s2.y - s0.y) + d2.y * (s0.y - s1.y)) / denominator;
    const c = (d0.x * (s2.x - s1.x) + d1.x * (s0.x - s2.x) + d2.x * (s1.x - s0.x)) / denominator;
    const d = (d0.y * (s2.x - s1.x) + d1.y * (s0.x - s2.x) + d2.y * (s1.x - s0.x)) / denominator;
    const e = (d0.x * (s1.x * s2.y - s2.x * s1.y) + d1.x * (s2.x * s0.y - s0.x * s2.y) + d2.x * (s0.x * s1.y - s1.x * s0.y)) / denominator;
    const f = (d0.y * (s1.x * s2.y - s2.x * s1.y) + d1.y * (s2.x * s0.y - s0.x * s2.y) + d2.y * (s0.x * s1.y - s1.x * s0.y)) / denominator;
    // Canvas anti-aliases each clipped triangle independently. Clipping on the
    // exact shared diagonal can therefore leave a translucent one-pixel seam
    // between two otherwise adjacent mesh cells. Grow only the clip polygon
    // by a sub-pixel amount while preserving the affine transform itself.
    const centre = {
      x: (d0.x + d1.x + d2.x) / 3,
      y: (d0.y + d1.y + d2.y) / 3
    };
    const expand = point => {
      const dx = point.x - centre.x;
      const dy = point.y - centre.y;
      const length = Math.max(1e-5, Math.hypot(dx, dy));
      return {
        x: point.x + dx / length * 0.85,
        y: point.y + dy / length * 0.85
      };
    };
    const [clip0, clip1, clip2] = dst.map(expand);
    context.save();
    context.beginPath();
    context.moveTo(clip0.x, clip0.y);
    context.lineTo(clip1.x, clip1.y);
    context.lineTo(clip2.x, clip2.y);
    context.closePath();
    context.clip();
    context.setTransform(a, b, c, d, e, f);
    context.drawImage(source, 0, 0);
    context.restore();
  }

  Light.warp = async function warp(source, pageQuad, frame = null) {
    const output = document.createElement("canvas");
    output.height = OUTPUT_LONG_EDGE;
    output.width = Math.round(OUTPUT_LONG_EDGE * A4_RATIO);
    const context = output.getContext("2d", { willReadFrequently: true });
    context.fillStyle = "#fff";
    context.fillRect(0, 0, output.width, output.height);
    context.imageSmoothingEnabled = true;
    context.imageSmoothingQuality = "high";
    const useCurvedFrame = !!frame?.paths && frame.confidence >= 0.18;
    if (useCurvedFrame) {
      const columns = clamp((frame.samples || 25) - 1, 12, 24);
      const rows = clamp((frame.samples || 25) - 1, 18, 24);
      const frameU0 = 1.5 / 21;
      const frameU1 = 19.5 / 21;
      const frameV0 = 1.43 / 29.7;
      const frameV1 = 28.43 / 29.7;
      const rowFractions = Array.from({ length: rows + 1 }, (_, index) => index / rows);
      if (frame.box?.support >= 0.25) rowFractions.push(1 - 1 / 27, 1 - 0.5 / 27);
      rowFractions.sort((first, second) => first - second);
      const uniqueRows = rowFractions.filter((value, index, values) => !index || Math.abs(value - values[index - 1]) > 1e-5);
      for (let row = 0; row < uniqueRows.length - 1; row += 1) {
        const v0 = frameV0 + (frameV1 - frameV0) * uniqueRows[row];
        const v1 = frameV0 + (frameV1 - frameV0) * uniqueRows[row + 1];
        const y0 = Math.floor(v0 * output.height) - 0.35;
        const y1 = Math.ceil(v1 * output.height) + 0.35;
        for (let column = 0; column < columns; column += 1) {
          const u0 = frameU0 + (frameU1 - frameU0) * (column / columns);
          const u1 = frameU0 + (frameU1 - frameU0) * ((column + 1) / columns);
          const sourceTopLeft = mapFramePoint(frame, u0, v0);
          const sourceBottomLeft = mapFramePoint(frame, u0, v1);
          const sourceTopRight = mapFramePoint(frame, u1, v0);
          const sourceBottomRight = mapFramePoint(frame, u1, v1);
          const x0 = Math.floor(u0 * output.width) - 0.35;
          const x1 = Math.ceil(u1 * output.width) + 0.35;
          const destinationTopLeft = { x: x0, y: y0 };
          const destinationBottomLeft = { x: x0, y: y1 };
          const destinationTopRight = { x: x1, y: y0 };
          const destinationBottomRight = { x: x1, y: y1 };
          drawTriangle(context, source,
            [sourceTopLeft, sourceBottomLeft, sourceTopRight],
            [destinationTopLeft, destinationBottomLeft, destinationTopRight]);
          drawTriangle(context, source,
            [sourceTopRight, sourceBottomLeft, sourceBottomRight],
            [destinationTopRight, destinationBottomLeft, destinationBottomRight]);
        }
        if (row % 3 === 2) await yieldToBrowser();
      }
      context.setTransform(1, 0, 0, 1, 0, 0);
      return output;
    }

    const strips = 28;
    for (let strip = 0; strip < strips; strip += 1) {
      const u0 = strip / strips;
      const u1 = (strip + 1) / strips;
      const sourceTopLeft = bilinear(pageQuad, u0, 0);
      const sourceBottomLeft = bilinear(pageQuad, u0, 1);
      const sourceTopRight = bilinear(pageQuad, u1, 0);
      const sourceBottomRight = bilinear(pageQuad, u1, 1);
      const x0 = Math.floor(u0 * output.width) - 0.35;
      const x1 = Math.ceil(u1 * output.width) + 0.35;
      const destinationTopLeft = { x: x0, y: -0.35 };
      const destinationBottomLeft = { x: x0, y: output.height + 0.35 };
      const destinationTopRight = { x: x1, y: -0.35 };
      const destinationBottomRight = { x: x1, y: output.height + 0.35 };
      drawTriangle(context, source,
        [sourceTopLeft, sourceBottomLeft, sourceTopRight],
        [destinationTopLeft, destinationBottomLeft, destinationTopRight]);
      drawTriangle(context, source,
        [sourceTopRight, sourceBottomLeft, sourceBottomRight],
        [destinationTopRight, destinationBottomLeft, destinationBottomRight]);
      if (strip % 7 === 6) await yieldToBrowser();
    }
    context.setTransform(1, 0, 0, 1, 0, 0);
    return output;
  };

  function medianSample(data, width, height, centerX, centerY, radius, predicate = null) {
    const values = [[], [], []];
    const minX = clamp(Math.floor(centerX - radius), 0, width - 1);
    const maxX = clamp(Math.ceil(centerX + radius), 0, width - 1);
    const minY = clamp(Math.floor(centerY - radius), 0, height - 1);
    const maxY = clamp(Math.ceil(centerY + radius), 0, height - 1);
    for (let y = minY; y <= maxY; y += 1) {
      for (let x = minX; x <= maxX; x += 1) {
        const offset = (y * width + x) * 4;
        const r = data[offset], g = data[offset + 1], b = data[offset + 2];
        if (predicate && !predicate(r, g, b)) continue;
        values[0].push(r); values[1].push(g); values[2].push(b);
      }
    }
    if (values[0].length < 3) return null;
    return values.map(channel => {
      channel.sort((a, b) => a - b);
      return channel[Math.floor(channel.length / 2)] / 255;
    });
  }

  function sampleCalibrationMarker(data, width, height, centerX, centerY, radius, name) {
    const candidates = [];
    const minimumX = clamp(Math.floor(centerX - radius), 0, width - 1);
    const maximumX = clamp(Math.ceil(centerX + radius), 0, width - 1);
    const minimumY = clamp(Math.floor(centerY - radius), 0, height - 1);
    const maximumY = clamp(Math.ceil(centerY + radius), 0, height - 1);
    for (let y = minimumY; y <= maximumY; y += 1) {
      for (let x = minimumX; x <= maximumX; x += 1) {
        const offset = (y * width + x) * 4;
        const r = data[offset], g = data[offset + 1], b = data[offset + 2];
        const luminance = r * 0.2126 + g * 0.7152 + b * 0.0722;
        const chroma = Math.max(r, g, b) - Math.min(r, g, b);
        let colourScore = -Infinity;
        let valid = false;
        if (name === "red") {
          // Warm photos can make red look orange, so G and B need not match.
          // The red-vs-green opponent difference still grows much faster for
          // ink than for the surrounding yellow ring.
          valid = r >= b + 12
            && r - g >= Math.max(7, (r - b) * 0.42);
          colourScore = r * 2 - g - b - Math.abs(g - b) * 0.9;
        } else if (name === "blue") {
          valid = b >= r + 2 && b >= g + 2;
          colourScore = b * 2 - r - g;
        } else if (name === "green") {
          valid = g >= r + 2 && g >= b + 4;
          colourScore = g * 2 - r - b;
        } else {
          valid = luminance < 165 && chroma <= 72;
          colourScore = 255 - luminance - chroma * 0.72;
        }
        if (!valid) continue;
        const distance = Math.hypot(x - centerX, y - centerY) / Math.max(1, radius);
        candidates.push({ r, g, b, score: colourScore - distance * 18 });
      }
    }
    if (candidates.length < 3) return null;
    candidates.sort((first, second) => second.score - first.score);
    const selected = candidates.slice(0, clamp(Math.ceil(candidates.length * 0.28), 5, 72));
    return ["r", "g", "b"].map(channel => {
      const values = selected.map(candidate => candidate[channel]).sort((a, b) => a - b);
      return values[Math.floor(values.length / 2)] / 255;
    });
  }

  function solve4(matrix, vector) {
    const augmented = matrix.map((row, index) => [...row, vector[index]]);
    for (let column = 0; column < 4; column += 1) {
      let pivot = column;
      for (let row = column + 1; row < 4; row += 1) {
        if (Math.abs(augmented[row][column]) > Math.abs(augmented[pivot][column])) pivot = row;
      }
      if (Math.abs(augmented[pivot][column]) < 1e-7) return null;
      [augmented[column], augmented[pivot]] = [augmented[pivot], augmented[column]];
      const divisor = augmented[column][column];
      for (let j = column; j < 5; j += 1) augmented[column][j] /= divisor;
      for (let row = 0; row < 4; row += 1) {
        if (row === column) continue;
        const factor = augmented[row][column];
        for (let j = column; j < 5; j += 1) augmented[row][j] -= factor * augmented[column][j];
      }
    }
    return augmented.map(row => row[4]);
  }

  function buildColorMatrix(samples) {
    const identity = [
      [1, 0, 0, 0],
      [0, 1, 0, 0],
      [0, 0, 1, 0]
    ];
    const xtx = Array.from({ length: 4 }, () => [0, 0, 0, 0]);
    const xty = Array.from({ length: 3 }, () => [0, 0, 0, 0]);
    for (const sample of samples) {
      if (!sample.source) continue;
      const feature = [...sample.source, 1];
      const weight = sample.weight || 1;
      for (let row = 0; row < 4; row += 1) {
        for (let column = 0; column < 4; column += 1) {
          xtx[row][column] += feature[row] * feature[column] * weight;
        }
        for (let channel = 0; channel < 3; channel += 1) {
          xty[channel][row] += feature[row] * sample.target[channel] * weight;
        }
      }
    }
    const ridge = 0.42;
    for (let i = 0; i < 4; i += 1) xtx[i][i] += ridge;
    return identity.map((fallback, channel) => {
      const rhs = xty[channel].map((value, index) => value + ridge * fallback[index]);
      const solved = solve4(xtx, rhs);
      if (!solved || solved.some(value => !Number.isFinite(value) || Math.abs(value) > 3.5)) return fallback;
      return solved;
    });
  }

  function percentileFromHistogram(histogram, total, percentile) {
    if (!total) return 255;
    const target = Math.max(1, Math.ceil(total * percentile));
    let seen = 0;
    for (let value = 0; value < histogram.length; value += 1) {
      seen += histogram[value];
      if (seen >= target) return value;
    }
    return 255;
  }

  function estimatePaperProfile(data, width, height, stencilAware = false) {
    const histograms = [new Uint32Array(256), new Uint32Array(256), new Uint32Array(256)];
    const luminanceHistogram = new Uint32Array(256);
    const stride = Math.max(3, Math.ceil(Math.max(width, height) / 520));
    const marginX = Math.floor(width * (stencilAware ? 0.12 : 0.035));
    const marginY = Math.floor(height * (stencilAware ? 0.08 : 0.035));
    const maximumY = Math.floor(height * (stencilAware ? 0.9 : 0.965));
    let neutralCount = 0;
    let luminanceCount = 0;
    for (let y = marginY; y < maximumY; y += stride) {
      for (let x = marginX; x < width - marginX; x += stride) {
        const offset = (y * width + x) * 4;
        const r = data[offset], g = data[offset + 1], b = data[offset + 2];
        const max = Math.max(r, g, b);
        const min = Math.min(r, g, b);
        const luminance = Math.round(r * 0.2126 + g * 0.7152 + b * 0.0722);
        luminanceHistogram[luminance] += 1;
        luminanceCount += 1;
        const saturation = max ? (max - min) / max : 1;
        // Paper may be grey because of shadows, but it remains considerably
        // more neutral than desks, folders and coloured handwriting.
        if (luminance >= 72 && saturation <= 0.19) {
          histograms[0][r] += 1;
          histograms[1][g] += 1;
          histograms[2][b] += 1;
          neutralCount += 1;
        }
      }
    }
    const sourcePaper = neutralCount >= 80
      ? histograms.map(histogram => percentileFromHistogram(histogram, neutralCount, 0.78))
      : [225, 225, 225];
    const paperLuminance = sourcePaper[0] * 0.2126 + sourcePaper[1] * 0.7152 + sourcePaper[2] * 0.0722;
    const shadowPoint = percentileFromHistogram(luminanceHistogram, luminanceCount, 0.04);
    // Keep the paper target neutral while limiting the global gain. Fine grid
    // and pencil detail are protected below by the local illumination test,
    // instead of lowering every page or clipping every bright pixel globally.
    const targetPaper = 255;
    const gains = sourcePaper.map(channel => clamp(targetPaper / Math.max(104, channel), 0.92, 1.3));
    // Limit channel-to-channel differences: this removes a colour cast while
    // never turning blue/red handwriting into another colour.
    const meanGain = (gains[0] + gains[1] + gains[2]) / 3;
    for (let index = 0; index < gains.length; index += 1) {
      gains[index] = clamp(gains[index], meanGain - 0.13, meanGain + 0.13);
    }
    return { gains, sourcePaper, paperLuminance, shadowPoint, targetPaper };
  }

  function medianColorSamples(samples) {
    const valid = samples.filter(Boolean);
    if (!valid.length) return null;
    return [0, 1, 2].map(channel => {
      const values = valid.map(sample => sample[channel]).sort((a, b) => a - b);
      return values[Math.floor(values.length / 2)];
    });
  }

  function referenceKindForPixel(r, g, b, luminance, saturation, paperLuminance,
    localPaperLuminance = paperLuminance) {
    if (isYellow(r, g, b)) return "yellow";
    if (r > 48 && r > g * 1.18 && r > b * 1.14) return "red";
    if (b > 42 && b > r * 1.13 && b > g * 1.06) return "blue";
    if (g > 48 && g > r * 1.1 && g > b * 1.16) return "green";
    if (saturation <= 0.2
      && luminance < paperLuminance * 0.82
      && (luminance < 72
        || luminance < localPaperLuminance - Math.max(22, localPaperLuminance * 0.13))) return "black";
    return null;
  }

  function calibrateFromReference(original, sample, sourcePaper, targetPaper) {
    if (!sample?.source || !sample?.target) return null;
    const source = sample.source.map(value => value * 255);
    const target = sample.target.map(value => value * 255);
    const inkVector = source.map((value, channel) => value - sourcePaper[channel]);
    const pixelVector = original.map((value, channel) => value - sourcePaper[channel]);
    const denominator = inkVector.reduce((sum, value) => sum + value * value, 0);
    if (denominator < 64) return null;
    const projection = pixelVector.reduce((sum, value, channel) => sum + value * inkVector[channel], 0) / denominator;
    if (projection <= 0.025 || projection > 2.1) return null;
    const residualSquared = pixelVector.reduce((sum, value, channel) => {
      const residual = value - inkVector[channel] * projection;
      return sum + residual * residual;
    }, 0);
    const pixelMagnitude = Math.sqrt(pixelVector.reduce((sum, value) => sum + value * value, 0));
    const residualRatio = Math.sqrt(residualSquared) / Math.max(18, pixelMagnitude);
    if (residualRatio > 0.5) return null;
    const amount = clamp(projection, 0, 1.08);
    const mapped = target.map(value => targetPaper + (value - targetPaper) * amount);
    return {
      mapped,
      mix: clamp(1 - residualRatio / 0.5, 0.58, 1)
    };
  }

  function markerLooksValid(name, source) {
    if (!source) return false;
    const [r, g, b] = source;
    if (name === "black") return Math.max(r, g, b) < 0.72
      && r * 0.2126 + g * 0.7152 + b * 0.0722 < 0.58;
    if (name === "red") return r > 0.18
      && r - b > 0.04
      && r - g >= Math.max(0.025, (r - b) * 0.4);
    if (name === "blue") return b > 0.16 && b >= r + 0.008 && b >= g + 0.008;
    if (name === "green") return g > 0.18 && g >= r + 0.008 && g >= b + 0.015;
    return false;
  }

  Light.correctColors = async function correctColors(canvas, options = {}) {
    const settings = typeof options === "boolean"
      ? { useStencil: options, preciseStencil: options }
      : options;
    const useStencil = !!settings.useStencil;
    const preciseStencil = !!settings.preciseStencil;
    const context = canvas.getContext("2d", { willReadFrequently: true });
    const image = context.getImageData(0, 0, canvas.width, canvas.height);
    const data = image.data;
    // A tiny blurred copy estimates illumination, not content. Comparing a
    // neutral pixel with this smooth local field separates shadowed paper
    // from ink without creating hard, patchy thresholds in uneven photos.
    const illuminationCanvas = document.createElement("canvas");
    illuminationCanvas.width = Math.min(192, canvas.width);
    illuminationCanvas.height = Math.max(1, Math.round(
      canvas.height * illuminationCanvas.width / Math.max(1, canvas.width)
    ));
    const illuminationContext = illuminationCanvas.getContext("2d", { willReadFrequently: true });
    illuminationContext.imageSmoothingEnabled = true;
    illuminationContext.imageSmoothingQuality = "high";
    illuminationContext.filter = "blur(2.4px)";
    illuminationContext.drawImage(canvas, 0, 0, illuminationCanvas.width, illuminationCanvas.height);
    illuminationContext.filter = "none";
    const illuminationPixels = illuminationContext.getImageData(
      0, 0, illuminationCanvas.width, illuminationCanvas.height
    ).data;
    const illumination = new Float32Array(illuminationCanvas.width * illuminationCanvas.height);
    for (let index = 0, offset = 0; index < illumination.length; index += 1, offset += 4) {
      illumination[index] = illuminationPixels[offset] * 0.2126
        + illuminationPixels[offset + 1] * 0.7152
        + illuminationPixels[offset + 2] * 0.0722;
    }
    const illuminationXScale = (illuminationCanvas.width - 1) / Math.max(1, canvas.width - 1);
    const illuminationYScale = (illuminationCanvas.height - 1) / Math.max(1, canvas.height - 1);
    const pxPerCm = canvas.width / 21;
    const radius = Math.max(5, pxPerCm * 0.28);
    const config = SP.Config || {};
    const targets = config.CALIBRATION_TARGETS || {};
    const profile = estimatePaperProfile(data, canvas.width, canvas.height, preciseStencil);
    const markerDefinitions = [
      [10.125, "red", [1, 0, 0]],
      [10.375, "black", [0, 0, 0]],
      [10.625, "blue", [0, 0, 1]],
      [10.875, "green", [110 / 255, 1, 18 / 255]]
    ];
    const samples = preciseStencil
      ? markerDefinitions.map(([x, name, fallback]) => ({
        name,
        source: sampleCalibrationMarker(
          data,
          canvas.width,
          canvas.height,
          x * pxPerCm,
          27.68 * pxPerCm,
          radius,
          name
        ),
        target: (targets[name] || fallback.map(value => value * 255)).map(value => value / 255),
        weight: name === "black" ? 1.4 : 1
      }))
      : [];
    if (preciseStencil) {
      // A single side can be shaded or partially hidden. Sample several parts
      // of the printed frame and use their median as the page's yellow ink.
      // Some cameras bleach that ink until it is effectively grey. The frame
      // geometry is already known after warping, so a darker neutral ridge at
      // these exact coordinates is still a valid fifth calibration reference.
      const stencilReference = (r, g, b) => {
        if (isYellow(r, g, b)) return true;
        const maximum = Math.max(r, g, b);
        const minimum = Math.min(r, g, b);
        const saturation = maximum ? (maximum - minimum) / maximum : 0;
        const luminance = r * 0.2126 + g * 0.7152 + b * 0.0722;
        return saturation <= 0.14
          && luminance >= 45
          && luminance <= profile.paperLuminance - 4.5;
      };
      const yellowSource = medianColorSamples([
        [1.5, 6], [1.5, 14.8], [1.5, 23],
        [19.5, 6], [19.5, 14.8], [19.5, 23],
        [6, 1.43], [10.5, 1.43], [15, 1.43]
      ].map(([x, y]) => medianSample(
        data, canvas.width, canvas.height,
        x * pxPerCm, y * pxPerCm, pxPerCm * 0.19,
        stencilReference
      )));
      const targetYellow = config.TARGET_YELLOW || { R: 240, G: 219, B: 76 };
      samples.push({
        name: "yellow",
        source: yellowSource,
        target: [targetYellow.R / 255, targetYellow.G / 255, targetYellow.B / 255],
        weight: 0.75
      });
    }
    const validMarkerSamples = samples.filter(sample => sample.name !== "yellow" && markerLooksValid(sample.name, sample.source));
    const yellowSample = samples.find(sample => sample.name === "yellow");
    const canCalibrate = preciseStencil
      && validMarkerSamples.length === markerDefinitions.length
      && !!yellowSample?.source;
    const referenceByName = Object.fromEntries(
      [...validMarkerSamples, ...(yellowSample?.source ? [yellowSample] : [])]
        .map(sample => [sample.name, sample])
    );
    const outerBand = pxPerCm * 0.12;
    const yellowX1 = 1.5 * pxPerCm;
    const yellowX2 = 19.5 * pxPerCm;
    const yellowY1 = 1.43 * pxPerCm;
    const yellowY2 = 28.43 * pxPerCm;
    const calibrationY1 = 27.28 * pxPerCm;
    const calibrationY2 = 28.05 * pxPerCm;
    const colorMarkerX1 = 9.92 * pxPerCm;
    const colorMarkerX2 = 11.08 * pxPerCm;
    const colorMarkerY1 = 27.48 * pxPerCm;
    const colorMarkerY2 = 27.90 * pxPerCm;
    for (let y = 0; y < canvas.height; y += 1) {
      for (let x = 0; x < canvas.width; x += 1) {
        const offset = (y * canvas.width + x) * 4;
        const r = data[offset], g = data[offset + 1], b = data[offset + 2];
        const inOuterBand = Math.abs(x - yellowX1) < outerBand
          || Math.abs(x - yellowX2) < outerBand
          || Math.abs(y - yellowY1) < outerBand
          || Math.abs(y - yellowY2) < outerBand;
        const inPageMargin = x < yellowX1 - outerBand
          || x > yellowX2 + outerBand
          || y < yellowY1 - outerBand
          || y > yellowY2 + outerBand;
        const inColorMarkers = x >= colorMarkerX1 && x <= colorMarkerX2
          && y >= colorMarkerY1 && y <= colorMarkerY2;
        const max = Math.max(r, g, b);
        const min = Math.min(r, g, b);
        const saturation = max ? (max - min) / max : 0;
        const luminance = r * 0.2126 + g * 0.7152 + b * 0.0722;
        // The template guarantees blank paper outside its yellow frame. When
        // the photographed sheet is clipped, extrapolation can otherwise pull
        // a dark folder or desk into this narrow margin. Clean only obvious
        // non-paper pixels; pale shadows and the physical paper edge remain.
        const outsidePaperArtifact = preciseStencil && inPageMargin
          && (luminance < 105 || (luminance < 178 && saturation > 0.22));
        if (outsidePaperArtifact) {
          data[offset] = profile.targetPaper;
          data[offset + 1] = profile.targetPaper;
          data[offset + 2] = profile.targetPaper;
          continue;
        }
        const shouldNeutralize = useStencil && (
          ((inOuterBand || (y >= calibrationY1 && y <= calibrationY2)) && isYellow(r, g, b))
          || (inColorMarkers && saturation > 0.24)
        );
        if (shouldNeutralize) {
          data[offset] = 255; data[offset + 1] = 255; data[offset + 2] = 255;
          continue;
        }
        const original = [r, g, b];
        const illuminationX = Math.round(x * illuminationXScale);
        const illuminationY = Math.round(y * illuminationYScale);
        const localBright = illumination[illuminationY * illuminationCanvas.width + illuminationX];
        const automatic = original.map((value, channel) => clamp(value * profile.gains[channel], 0, 255));
        const kind = canCalibrate
          ? referenceKindForPixel(
            r, g, b, luminance, saturation, profile.paperLuminance, localBright
          )
          : null;
        const calibrated = kind
          ? calibrateFromReference(original, referenceByName[kind], profile.sourcePaper, profile.targetPaper)
          : null;
        let output = automatic;
        if (calibrated) {
          output = automatic.map((value, channel) => (
            value * (1 - calibrated.mix) + calibrated.mapped[channel] * calibrated.mix
          ));
        } else if (saturation <= 0.18) {
          // The paper is the fifth calibration reference. Push neutral bright
          // pixels to true white while leaving pencil and black writing intact.
          const whiteStart = Math.max(145, profile.shadowPoint + 34, profile.paperLuminance * 0.7);
          const whiteEnd = Math.max(whiteStart + 22, profile.paperLuminance * 0.98);
          const position = clamp((luminance - whiteStart) / Math.max(1, whiteEnd - whiteStart), 0, 1);
          let whiteMix = position * position * (3 - 2 * position);
          // The blurred illumination value follows smooth shadows but not thin
          // writing. A soft relative threshold avoids both posterisation and
          // accidental removal of pencil/grid details.
          const localDifference = localBright - luminance;
          const localPaperFloor = Math.max(70, profile.shadowPoint + 16, profile.paperLuminance * 0.5);
          const localPaperAmount = luminance >= localPaperFloor
            ? clamp((12 - localDifference) / 8, 0, 1)
            : 0;
          const smoothLocalPaper = localPaperAmount * localPaperAmount * (3 - 2 * localPaperAmount);
          whiteMix = Math.min(0.98, Math.max(whiteMix * 0.8, smoothLocalPaper * 0.98));
          output = automatic.map(value => value * (1 - whiteMix) + profile.targetPaper * whiteMix);
        } else {
          // Preserve colours that are not one of the printed references, but
          // restore a small amount of saturation lost through camera exposure.
          const balancedLuminance = automatic[0] * 0.2126 + automatic[1] * 0.7152 + automatic[2] * 0.0722;
          const vibrance = 1.08;
          output = automatic.map(value => balancedLuminance + (value - balancedLuminance) * vibrance);
        }
        for (let channel = 0; channel < 3; channel += 1) {
          data[offset + channel] = Math.round(clamp(output[channel], 0, 255));
        }
      }
      if (y % 96 === 95) await yieldToBrowser();
    }
    context.putImageData(image, 0, 0);
    illuminationCanvas.width = 0;
    illuminationCanvas.height = 0;
    return {
      canvas,
      calibrated: canCalibrate,
      samples: validMarkerSamples.length,
      paper: Math.round(profile.paperLuminance),
      references: Object.fromEntries(samples.map(sample => [sample.name, {
        source: sample.source?.map(value => Math.round(value * 255)) || null,
        valid: sample.name === "yellow"
          ? !!sample.source
          : markerLooksValid(sample.name, sample.source)
      }]))
    };
  };

  Light.createDetectionPreview = function createDetectionPreview(source, pageQuad, confidence) {
    const preview = document.createElement("canvas");
    preview.width = source.width;
    preview.height = source.height;
    const context = preview.getContext("2d");
    context.drawImage(source, 0, 0);
    context.strokeStyle = confidence > 0 ? "#f0db4c" : "#4f8cff";
    context.lineWidth = Math.max(3, Math.min(source.width, source.height) / 220);
    context.beginPath();
    pageQuad.forEach((point, index) => index ? context.lineTo(point.x, point.y) : context.moveTo(point.x, point.y));
    context.closePath();
    context.stroke();
    return preview;
  };

  SP.detectPageEdges = source => Light.detectPage(source).pageQuad;
})();
