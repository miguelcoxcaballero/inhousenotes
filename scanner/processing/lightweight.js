(() => {
  "use strict";

  const SP = (window.ScannerPro = window.ScannerPro || {});
  const Light = (SP.Lightweight = {});
  const A4_RATIO = 21 / 29.7;
  const ANALYSIS_MAX = 640;
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
    if (r - g > 10 && r - b > 8 && r > g * 1.08) return "red";
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
      const normalized = invertBilinear(paperQuad, source);
      if (!normalized || normalized.u <= -0.18 || normalized.u >= 1.18
        || normalized.v <= -0.18 || normalized.v >= 1.18) continue;
      const neighborhoodRadius = Math.max(4, Math.ceil(Math.max(boxWidth, boxHeight) * 0.75) + 3);
      let yellowNearby = 0;
      for (let y = Math.max(0, Math.floor(source.y - neighborhoodRadius)); y <= Math.min(height - 1, Math.ceil(source.y + neighborhoodRadius)); y += 1) {
        for (let x = Math.max(0, Math.floor(source.x - neighborhoodRadius)); x <= Math.min(width - 1, Math.ceil(source.x + neighborhoodRadius)); x += 1) {
          if (yellowMask[y * width + x]) yellowNearby += 1;
        }
      }
      components.push({
        ...normalized,
        kind: names[category],
        count: tail,
        compactness: tail / (boxWidth * boxHeight),
        yellowNearby
      });
    }
    return components;
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
      if (Math.abs(a) > 0.72) continue;
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
      const expectedPenalty = 1 + Math.abs(atCenter - options.expected) / 0.1;
      const slopePenalty = 1 + Math.abs(a) / 0.075;
      const anchorPenalty = Number.isFinite(options.anchorIndependent)
        ? 1 + Math.abs(a * options.anchorIndependent + b - options.anchorDependent) / 0.025
        : 1;
      const score = inliers.length * (1 + Math.min(1, span / 0.82))
        / (expectedPenalty * slopePenalty * anchorPenalty);
      if (score > bestScore) {
        bestScore = score;
        best = { inliers, atCenter, span, minimum, maximum };
      }
    }
    if (!best) return null;
    const fitted = robustLine(best.inliers, independentKey, dependentKey);
    return fitted ? {
      ...fitted,
      support: best.inliers.length,
      span: best.span,
      independentMinimum: best.minimum,
      independentMaximum: best.maximum
    } : null;
  }

  function medianNumber(values) {
    if (!values.length) return null;
    const sorted = values.slice().sort((a, b) => a - b);
    return sorted[Math.floor(sorted.length / 2)];
  }

  function estimateCalibrationStrip(yellowPoints, stripLine) {
    const minimumU = -0.12;
    const maximumU = 1.12;
    const binCount = 180;
    const counts = new Uint16Array(binCount);
    for (const point of yellowPoints) {
      if (point.u < minimumU || point.u > maximumU
        || Math.abs(point.v - (stripLine.a * point.u + stripLine.b)) > 0.05) continue;
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

  function detectMarkerGuidedStencil(paperQuad, width, height, pixels, yellowMask) {
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
        && normalized.v > -0.18 && normalized.v < 1.18) yellowPoints.push(normalized);
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

    const oriented = yellowPoints.map(point => rotateNormalized(point, bestRotation));
    const orientedChromatic = chromaticPoints.map(point => ({ ...point, ...rotateNormalized(point, bestRotation) }));
    const bottomPoints = oriented.filter(point => point.v >= 0.72 && point.v <= 1.08 && point.u > 0.02 && point.u < 0.98);
    const common = { minimumSpan: 0.42, tolerance: 0.009 };
    const stripLine = guidedRansacLine(bottomPoints, "u", "v", {
      ...common,
      expected: 27.68 / 29.7
    });
    if (!stripLine) return null;
    const stripInterval = estimateCalibrationStrip(oriented, stripLine);
    if (!stripInterval) return null;
    const markerAxis = fitMarkerAxis(orientedChromatic, stripLine, stripInterval);
    if (!markerAxis) return null;
    const blackMarker = blackMarkerIsPresent(markerAxis, bestRotation, paperQuad, pixels, width, height);
    const leftAnchor = stripInterval.minimum;
    const rightAnchor = stripInterval.maximum;
    const geometryScale = stripInterval.span / (18 / 21);
    const stripAtCenter = stripLine.a * 0.5 + stripLine.b;
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
    const topPoints = oriented.filter(point => Math.abs(point.v - topExpected) < 0.23
      && point.u > leftAnchor - 0.08 && point.u < rightAnchor + 0.08);
    let leftLine = guidedRansacLine(leftPoints, "v", "u", {
      ...common,
      minimumSpan: Math.max(0.36, geometryScale * 0.56),
      expected: leftAnchor,
      anchorIndependent: bottomAtCenter,
      anchorDependent: leftAnchor
    });
    let rightLine = guidedRansacLine(rightPoints, "v", "u", {
      ...common,
      minimumSpan: Math.max(0.36, geometryScale * 0.56),
      expected: rightAnchor,
      anchorIndependent: bottomAtCenter,
      anchorDependent: rightAnchor
    });
    let topLine = guidedRansacLine(topPoints, "u", "v", {
      ...common,
      minimumSpan: Math.max(0.36, geometryScale * 0.5),
      expected: topExpected
    });
    // The printed marker pattern fully determines scale and orientation. When a
    // border is hidden by another sheet, synthesize only that missing line from
    // the calibrated geometry instead of falling back to the surrounding desk.
    leftLine ||= { a: 0, b: leftAnchor, count: 0, support: 0, span: 0 };
    rightLine ||= { a: 0, b: rightAnchor, count: 0, support: 0, span: 0 };
    topLine ||= { a: stripLine.a, b: topExpected - stripLine.a * 0.5, count: 0, support: 0, span: 0 };
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
      minAreaRatio: 0.48,
      maxOppositeRatio: 1.4
    })) return null;
    const centerValues = {
      left: leftLine.a * 0.5 + leftLine.b,
      right: rightLine.a * 0.5 + rightLine.b,
      top: topLine.a * 0.5 + topLine.b,
      bottom: bottomLine.a * 0.5 + bottomLine.b
    };
    if (centerValues.left < -0.04 || centerValues.left > 0.25
      || centerValues.right < 0.75 || centerValues.right > 1.04
      || centerValues.top < -0.04 || centerValues.top > 0.22
      || centerValues.bottom < 0.8 || centerValues.bottom > 1.06) return null;

    const stencilQuad = normalizedQuad.map(point => {
      const base = unrotateNormalized(point, bestRotation);
      return bilinear(paperQuad, base.u, base.v);
    });
    const support = leftLine.support + rightLine.support + topLine.support + stripLine.support;
    return {
      stencilQuad,
      confidence: clamp(0.58 + support / Math.max(900, yellowPoints.length * 2.2)
        + Math.min(0.14, bestMarkerHits * 0.01) + (blackMarker ? 0.04 : 0), 0, 1),
      rotation: bestRotation,
      markerHits: bestMarkerHits
    };
  }

  Light.detectPage = function detectPage(source) {
    const analysis = createScaledCanvas(source, ANALYSIS_MAX);
    const { canvas, context, scale } = analysis;
    const { width, height } = canvas;
    const pixels = context.getImageData(0, 0, width, height).data;
    const mask = new Uint8Array(width * height);
    let yellowPixels = 0;
    for (let i = 0, p = 0; i < pixels.length; i += 4, p += 1) {
      if (isYellow(pixels[i], pixels[i + 1], pixels[i + 2])) {
        mask[p] = 1;
        yellowPixels += 1;
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
    const paperQuad = detectPaperQuad(width, height, pixels);
    const markerGuided = detectMarkerGuidedStencil(paperQuad, width, height, pixels, mask);
    canvas.width = 0;
    canvas.height = 0;
    if (markerGuided) {
      stencilQuad = markerGuided.stencilQuad.map(point => ({ x: point.x / scale, y: point.y / scale }));
      return {
        pageQuad: extrapolateStencil(stencilQuad),
        stencilQuad,
        confidence: markerGuided.confidence,
        method: "marker-guided"
      };
    }
    if (confidence >= 0.24) {
      stencilQuad = stencilQuad.map(point => ({ x: point.x / scale, y: point.y / scale }));
      return {
        pageQuad: extrapolateStencil(stencilQuad),
        stencilQuad,
        confidence,
        method: "stencil"
      };
    }
    if (paperQuad) {
      const hasStencilSignal = yellowPixels >= width * height * 0.00045;
      return {
        pageQuad: paperQuad.map(point => ({ x: point.x / scale, y: point.y / scale })),
        stencilQuad: null,
        confidence: hasStencilSignal ? 0.35 : 0,
        method: "paper"
      };
    }
    return { pageQuad: fallbackPageQuad(source), stencilQuad: null, confidence: 0, method: "fallback" };
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
    context.save();
    context.beginPath();
    context.moveTo(d0.x, d0.y);
    context.lineTo(d1.x, d1.y);
    context.lineTo(d2.x, d2.y);
    context.closePath();
    context.clip();
    context.setTransform(a, b, c, d, e, f);
    context.drawImage(source, 0, 0);
    context.restore();
  }

  Light.warp = async function warp(source, pageQuad) {
    const output = document.createElement("canvas");
    output.height = OUTPUT_LONG_EDGE;
    output.width = Math.round(OUTPUT_LONG_EDGE * A4_RATIO);
    const context = output.getContext("2d", { willReadFrequently: true });
    context.fillStyle = "#fff";
    context.fillRect(0, 0, output.width, output.height);
    context.imageSmoothingEnabled = true;
    context.imageSmoothingQuality = "high";
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

  function estimatePaperProfile(data, width, height) {
    const histograms = [new Uint32Array(256), new Uint32Array(256), new Uint32Array(256)];
    const luminanceHistogram = new Uint32Array(256);
    const stride = Math.max(3, Math.ceil(Math.max(width, height) / 520));
    const marginX = Math.floor(width * 0.035);
    const marginY = Math.floor(height * 0.035);
    let neutralCount = 0;
    let luminanceCount = 0;
    for (let y = marginY; y < height - marginY; y += stride) {
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
      ? histograms.map(histogram => percentileFromHistogram(histogram, neutralCount, 0.86))
      : [225, 225, 225];
    const paperLuminance = sourcePaper[0] * 0.2126 + sourcePaper[1] * 0.7152 + sourcePaper[2] * 0.0722;
    const shadowPoint = percentileFromHistogram(luminanceHistogram, luminanceCount, 0.04);
    const targetPaper = 242;
    const gains = sourcePaper.map(channel => clamp(targetPaper / Math.max(96, channel), 0.9, 1.3));
    // Limit channel-to-channel differences: this removes a colour cast while
    // never turning blue/red handwriting into another colour.
    const meanGain = (gains[0] + gains[1] + gains[2]) / 3;
    for (let index = 0; index < gains.length; index += 1) {
      gains[index] = clamp(gains[index], meanGain - 0.13, meanGain + 0.13);
    }
    return { gains, paperLuminance, shadowPoint, targetPaper };
  }

  function markerLooksValid(name, source) {
    if (!source) return false;
    const [r, g, b] = source;
    if (name === "black") return Math.max(r, g, b) < 0.54 && r * 0.2126 + g * 0.7152 + b * 0.0722 < 0.38;
    if (name === "red") return r > 0.28 && r > g * 1.2 && r > b * 1.16;
    if (name === "blue") return b > 0.25 && b > r * 1.16 && b > g * 1.08;
    if (name === "green") return g > 0.3 && g > r * 1.1 && g > b * 1.2;
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
    const pxPerCm = canvas.width / 21;
    const radius = Math.max(3, pxPerCm * 0.075);
    const config = SP.Config || {};
    const targets = config.CALIBRATION_TARGETS || {};
    const markerDefinitions = [
      [10.125, "red", [1, 0, 0]],
      [10.375, "black", [0, 0, 0]],
      [10.625, "blue", [0, 0, 1]],
      [10.875, "green", [110 / 255, 1, 18 / 255]]
    ];
    const samples = preciseStencil
      ? markerDefinitions.map(([x, name, fallback]) => ({
        name,
        source: medianSample(data, canvas.width, canvas.height, x * pxPerCm, 27.68 * pxPerCm, radius),
        target: (targets[name] || fallback.map(value => value * 255)).map(value => value / 255),
        weight: name === "black" ? 1.4 : 1
      }))
      : [];
    if (preciseStencil) {
      const yellowSource = medianSample(
        data, canvas.width, canvas.height,
        1.5 * pxPerCm, 14.8 * pxPerCm, pxPerCm * 0.16,
        isYellow
      );
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
    const profile = estimatePaperProfile(data, canvas.width, canvas.height);
    const paperSource = profile.paperLuminance / 255;
    const matrixSamples = canCalibrate
      ? [...validMarkerSamples, yellowSample, {
        name: "white",
        source: [paperSource, paperSource, paperSource],
        target: [profile.targetPaper / 255, profile.targetPaper / 255, profile.targetPaper / 255],
        weight: 1.8
      }]
      : [];
    const matrix = canCalibrate ? buildColorMatrix(matrixSamples) : null;
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
        const inColorMarkers = x >= colorMarkerX1 && x <= colorMarkerX2
          && y >= colorMarkerY1 && y <= colorMarkerY2;
        const max = Math.max(r, g, b);
        const min = Math.min(r, g, b);
        const saturation = max ? (max - min) / max : 0;
        const shouldNeutralize = useStencil && (
          ((inOuterBand || (y >= calibrationY1 && y <= calibrationY2)) && isYellow(r, g, b))
          || (inColorMarkers && saturation > 0.24)
        );
        if (shouldNeutralize) {
          const luminance = r * 0.2126 + g * 0.7152 + b * 0.0722;
          const shade = clamp(luminance / Math.max(1, profile.paperLuminance), 0.74, 1.04);
          const neutral = clamp(Math.round(profile.targetPaper * shade), 178, 248);
          data[offset] = neutral; data[offset + 1] = neutral; data[offset + 2] = neutral;
          continue;
        }
        const original = [r, g, b];
        for (let channel = 0; channel < 3; channel += 1) {
          const automatic = clamp(original[channel] * profile.gains[channel], 0, 255);
          if (!matrix) {
            data[offset + channel] = Math.round(automatic);
            continue;
          }
          const nr = r / 255, ng = g / 255, nb = b / 255;
          const calibrated = clamp((matrix[channel][0] * nr + matrix[channel][1] * ng
            + matrix[channel][2] * nb + matrix[channel][3]) * 255, 0, 255);
          // The printed targets refine hue, but automatic paper balance remains
          // dominant so a damaged/dirty marker cannot wash out a whole scan.
          const boundedCalibration = clamp(calibrated, automatic - 30, automatic + 30);
          data[offset + channel] = Math.round(automatic * 0.76 + boundedCalibration * 0.24);
        }
      }
      if (y % 96 === 95) await yieldToBrowser();
    }
    context.putImageData(image, 0, 0);
    return {
      canvas,
      calibrated: canCalibrate,
      samples: validMarkerSamples.length,
      paper: Math.round(profile.paperLuminance)
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
