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
  const moduleUrl = typeof document !== "undefined" ? document.currentScript?.src : null;
  const processingWorker = typeof WorkerGlobalScope !== "undefined" && globalThis instanceof WorkerGlobalScope;

  const clamp = (value, min, max) => Math.max(min, Math.min(max, value));
  const yieldToBrowser = () => {
    // Each still has its own disposable worker. No UI events need a turn on
    // that thread, and termination cancels it immediately. Avoid the 4 ms
    // timer clamp hundreds of row-band yields would otherwise accumulate.
    if (processingWorker) return Promise.resolve();
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

  function detectionImage(context, width, height) {
    const image = context.getImageData(0, 0, width, height);
    // All geometry stages must see the same paper-relative ink colours. A
    // cold camera white balance can make actual yellow ink look blue in RGB.
    // This analysis copy never replaces the photographed pixels used by warp.
    const balanced = SP.LiveDetection?.normalizeIllumination?.(image) || image;
    // Geometry thresholds describe ink contrast in an ordinary exposure. A
    // sharp but dim photograph can retain every printed rail while all its
    // pixels sit below those thresholds. Only a measured four-colour footer
    // may authorize lifting this disposable analysis copy. The source photo,
    // focus gate and independent full-source dot certification remain intact.
    const samples = [];
    const stride = Math.max(4, Math.floor(Math.max(width, height) / 40));
    for (let y = 0; y < height; y += stride) for (let x = 0; x < width; x += stride) {
      const at = (y * width + x) * 4;
      samples.push(image.data[at] * .2126 + image.data[at + 1] * .7152 + image.data[at + 2] * .0722);
    }
    samples.sort((a, b) => a - b);
    if (samples[Math.floor(samples.length * .9)] >= 112) return balanced;
    const references = balanced.colourReferences || SP.LiveDetection?.findColourReferences?.(image);
    const white = references?.white;
    const paperLight = white && white[0] * .2126 + white[1] * .7152 + white[2] * .0722;
    if (!Number.isFinite(paperLight) || paperLight < 32 || paperLight >= 112) return balanced;
    const gain = Math.min(3, 152 / paperLight);
    const data = new Uint8ClampedArray(balanced.data.length);
    for (let at = 0; at < data.length; at += 4) {
      data[at] = balanced.data[at] * gain;
      data[at + 1] = balanced.data[at + 1] * gain;
      data[at + 2] = balanced.data[at + 2] * gain;
      data[at + 3] = balanced.data[at + 3];
    }
    return { ...balanced, data, analysisExposureGain: gain };
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

  function hasLocalYellowRidge(index, width, height, pixels) {
    const x = index % width, y = Math.floor(index / width), at = index * 4;
    const center = yellowChroma(pixels[at], pixels[at + 1], pixels[at + 2]);
    const chromaAt = (xx, yy) => {
      const offset = (yy * width + xx) * 4;
      return yellowChroma(pixels[offset], pixels[offset + 1], pixels[offset + 2]);
    };
    // White-balancing a cool-lit page can turn a separately lit neutral desk
    // yellow. A printed rail is a narrow ridge with paper on BOTH sides; a
    // filled desk or its one-sided silhouette is not. Measure perpendicular
    // contrast, retaining oblique/curved rails without accepting flat colour.
    for (const radius of [3, 5]) {
      if (x >= radius && x + radius < width
        && center - Math.max(chromaAt(x - radius, y), chromaAt(x + radius, y)) >= 2) return true;
      if (y >= radius && y + radius < height
        && center - Math.max(chromaAt(x, y - radius), chromaAt(x, y + radius)) >= 2) return true;
    }
    return false;
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
    // Each yellow pixel belongs to at most two overlapping sample windows.
    // Assign it once, retaining source order and the exact distance predicate,
    // instead of scanning the entire page again for every border sample.
    const localPointSets = Array.from({ length: sampleCount }, () => []);
    for (const point of points) {
      if (!(Math.abs(point[perpendicular] - expected) <= maxDeviation)) continue;
      const coordinate = point[axis];
      const nearest = Math.floor(coordinate * (sampleCount - 1));
      for (let sample = Math.max(0, nearest - 1);
        sample <= Math.min(sampleCount - 1, nearest + 1); sample += 1) {
        if (Math.abs(coordinate - sample / (sampleCount - 1)) <= window) {
          localPointSets[sample].push(point);
        }
      }
    }
    for (let sample = 0; sample < sampleCount; sample += 1) {
      const position = sample / (sampleCount - 1);
      const localPoints = localPointSets[sample];
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
    // Acquisition-order bins robustly estimate transverse corner offsets.
    // The actual curve below keeps measured longitudinal coordinates: missing
    // observations are gaps, not shorter paper.
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
      const position = sample / (sampleCount - 1);
      const upper = observed.findIndex(entry => entry.normalized[axis] >= position);
      const index = upper < 0 ? observed.length - 2 : Math.max(0, upper - 1);
      const start = observed[index].normalized[axis], end = observed[index + 1].normalized[axis];
      const mix = (position - start) / Math.max(1e-6, end - start);
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
      const localV = new Float64Array(localPoints.length);
      const localBits = new Uint32Array(localPoints.length);
      const inBoxRail = new Uint8Array(localPoints.length);
      for (let index = 0; index < localPoints.length; index += 1) {
        const point = localPoints[index];
        const bin = clamp(Math.floor((point.u - (position - window)) / (window * 2) * occupancyBins), 0, occupancyBins - 1);
        localV[index] = point.v;
        localBits[index] = 1 << bin;
        inBoxRail[index] = railExistsAt(0, point.u) ? 1 : 0;
      }
      let boxAvailable = 0;
      for (let bin = 0; bin < occupancyBins; bin += 1) {
        const u = position - window + (bin + 0.5) / occupancyBins * window * 2;
        if (railExistsAt(0, u)) boxAvailable += 1;
      }
      const horizontalSupport = (target, rail) => {
        // Twenty occupancy bins fit in one integer. Membership and the bin
        // coordinates are invariant across the 255 trial rail positions.
        let occupied = 0;
        for (let index = 0; index < localV.length; index += 1) {
          if (rail !== 2 && !inBoxRail[index]) continue;
          if (Math.abs(localV[index] - target) <= ridgeTolerance) occupied |= localBits[index];
        }
        let count = 0;
        while (occupied) { occupied &= occupied - 1; count += 1; }
        const available = rail === 2 ? occupancyBins : boxAvailable;
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
    const horizontalPosition = (index, bottomEdge = false) => {
      const amount = index / (count - 1);
      const left = cornersNormalized[bottomEdge ? 3 : 0].u;
      const right = cornersNormalized[bottomEdge ? 2 : 1].u;
      return left + (right - left) * amount;
    };
    const valueAt = (values, amount) => {
      const position = amount * (values.length - 1);
      const index = clamp(Math.floor(position), 0, values.length - 2);
      return values[index] + (values[index + 1] - values[index]) * (position - index);
    };
    if (neutralRecovery) for (let index = 1; index < count - 1; index += 1) {
      // Neutral recovery intentionally avoids iterative reseeding. Its
      // measured corners may nevertheless shift the provisional coordinate
      // origin. Resample the same ridges between those observed endpoints;
      // replacing only the endpoints otherwise stretches the first/last cell
      // and slides every interior calibration square sideways.
      const topU = horizontalPosition(index), bottomU = horizontalPosition(index, true);
      topNormalized[index] = { u: topU, v: valueAt(top.values, topU) };
      bottomNormalized[index] = { u: bottomU, v: valueAt(bottom.values, bottomU) };
    }
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
      const length = Math.hypot(end.x - start.x, end.y - start.y) || 1;
      const tx = (end.x - start.x) / length, ty = (end.y - start.y) / length;
      const startAlong = (start.x - first.x) * tx + (start.y - first.y) * ty;
      const endAlong = (end.x - last.x) * tx + (end.y - last.y) * ty;
      const aligned = sourcePath.map((point, index) => {
        const amount = index / Math.max(1, sourcePath.length - 1);
        const along = startAlong * (1 - amount) + endAlong * amount;
        return {
          x: point.x + tx * along,
          y: point.y + ty * along
        };
      });
      // Endpoint extrapolation can contain an isolated neighbouring ridge.
      // Align the row coordinate along the rail, but never spread that
      // transverse endpoint error across the entire measured side.
      const span = Math.min(3, aligned.length - 1);
      for (let index = 0; index < span; index += 1) {
        const amount = index / span;
        aligned[index] = { x: start.x * (1 - amount) + aligned[span].x * amount,
          y: start.y * (1 - amount) + aligned[span].y * amount };
        const reverse = aligned.length - 1 - index;
        const anchor = aligned[aligned.length - 1 - span];
        aligned[reverse] = { x: end.x * (1 - amount) + anchor.x * amount,
          y: end.y * (1 - amount) + anchor.y * amount };
      }
      return aligned;
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
      u: neutralRecovery ? horizontalPosition(index, true) : index / (count - 1),
      v: neutralRecovery ? valueAt(boxTop.values, horizontalPosition(index, true)) : value
    }));
    const boxBottomPath = boxBottom.values.map((value, index) => toSource({
      u: neutralRecovery ? horizontalPosition(index, true) : index / (count - 1),
      v: neutralRecovery ? valueAt(boxBottom.values, horizontalPosition(index, true)) : value
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
    if ((scaled < 0 || scaled > curve.length - 1) && curve.length >= 4) {
      // The physical blank margin lies outside the measured frame. A single
      // antialiased endpoint interval is too noisy to determine its tangent;
      // fit the four interior samples independently of that noisy corner,
      // then anchor the extrapolation at the corner to preserve continuity.
      const end = scaled < 0 ? 0 : curve.length - 1;
      const direction = scaled < 0 ? 1 : -1;
      const count = Math.min(4, curve.length - 1);
      let dx = 0, dy = 0, weight = 0, sumDistance = 0, sumX = 0, sumY = 0;
      for (let step = 1; step <= count; step += 1) {
        const point = curve[end + direction * step];
        const distance = direction * step;
        dx += distance * (point.x - curve[end].x);
        dy += distance * (point.y - curve[end].y);
        weight += distance * distance;
        sumDistance += distance;
        sumX += point.x - curve[end].x;
        sumY += point.y - curve[end].y;
      }
      dx -= sumDistance * sumX / count;
      dy -= sumDistance * sumY / count;
      weight -= sumDistance * sumDistance / count;
      return { x: curve[end].x + dx / weight * (scaled - end),
        y: curve[end].y + dy / weight * (scaled - end) };
    }
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
    if (frame.metricSurface && SP.MetricSurface?.map) {
      return SP.MetricSurface.map(frame.metricSurface, pageU, pageV);
    }
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
    // RANSAC revisits these coordinates hundreds of times. Cache the numeric
    // columns once and reuse scratch storage without changing hypotheses,
    // sample order or scoring. Only a winning model needs point objects.
    const independent = new Float64Array(points.length);
    const dependent = new Float64Array(points.length);
    const strengths = new Float64Array(points.length);
    for (let index = 0; index < points.length; index += 1) {
      independent[index] = points[index][independentKey];
      dependent[index] = points[index][dependentKey];
      strengths[index] = Math.min(3, points[index].strength || 1);
    }
    // Points arrive in image-scan order. Most contiguous blocks lie well
    // outside a particular line's tolerance; their bounding box can reject
    // that hypothesis without visiting every point. Surviving points retain
    // their original order and the exact residual/weight calculations.
    const blockSize = 32;
    const blockCount = Math.ceil(points.length / blockSize);
    const bounds = new Float64Array(blockCount * 4);
    for (let block = 0; block < blockCount; block += 1) {
      let minimumX = Infinity, maximumX = -Infinity;
      let minimumY = Infinity, maximumY = -Infinity;
      const end = Math.min(points.length, (block + 1) * blockSize);
      for (let index = block * blockSize; index < end; index += 1) {
        minimumX = Math.min(minimumX, independent[index]);
        maximumX = Math.max(maximumX, independent[index]);
        minimumY = Math.min(minimumY, dependent[index]);
        maximumY = Math.max(maximumY, dependent[index]);
      }
      bounds.set([minimumX, maximumX, minimumY, maximumY], block * 4);
    }
    const inlierIndices = new Uint32Array(points.length);
    const occupied = new Uint8Array(120);
    const continuityWeight = clamp(options.continuityWeight || 0, 0, 0.95);
    const measureContinuity = (minimum, span, inlierCount) => {
      const continuityBins = Math.min(120, Math.max(36,
        Math.ceil(span / Math.max(0.006, options.tolerance * 0.7))));
      occupied.fill(0, 0, continuityBins);
      let occupiedCount = 0;
      for (let index = 0; index < inlierCount; index += 1) {
        const bin = clamp(Math.floor((independent[inlierIndices[index]] - minimum)
          / Math.max(1e-6, span) * continuityBins), 0, continuityBins - 1);
        if (!occupied[bin]) occupiedCount += 1;
        occupied[bin] = 1;
      }
      return occupiedCount / continuityBins;
    };
    let best = null;
    let bestScore = 0;
    const hypotheses = Math.min(360, Math.max(140, points.length * 2));
    for (let attempt = 0; attempt < hypotheses; attempt += 1) {
      const first = randomIndex();
      const second = randomIndex();
      const delta = independent[second] - independent[first];
      if (Math.abs(delta) < options.minimumSpan * 0.34) continue;
      const a = (dependent[second] - dependent[first]) / delta;
      // Coordinates are already normalized through the detected paper quad;
      // stencil borders must therefore stay close to horizontal/vertical.
      // A steeper candidate is almost always handwriting or a neighbouring
      // sheet crossing the expected band (not a perspective effect).
      if (Math.abs(a) > (options.maximumSlope || 0.12)) continue;
      const b = dependent[first] - a * independent[first];
      let inlierCount = 0;
      let weightedSupport = 0;
      let minimum = Infinity;
      let maximum = -Infinity;
      for (let block = 0; block < blockCount; block += 1) {
        const bound = block * 4;
        const low = a * bounds[bound + (a < 0 ? 1 : 0)] + b;
        const high = a * bounds[bound + (a < 0 ? 0 : 1)] + b;
        // A tiny guard only admits extra work at floating-point boundaries;
        // it never discards an inlier accepted by the original residual test.
        if (bounds[bound + 2] - high > options.tolerance + 1e-12
          || low - bounds[bound + 3] > options.tolerance + 1e-12) continue;
        const end = Math.min(points.length, (block + 1) * blockSize);
        for (let index = block * blockSize; index < end; index += 1) {
          const coordinate = independent[index];
          if (Math.abs(dependent[index] - (a * coordinate + b)) > options.tolerance) continue;
          inlierIndices[inlierCount++] = index;
          weightedSupport += strengths[index];
          minimum = Math.min(minimum, coordinate);
          maximum = Math.max(maximum, coordinate);
        }
      }
      const span = maximum - minimum;
      if (span < options.minimumSpan || inlierCount < 12) continue;
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
      // Local ridge fits do not weight continuity. Its value still belongs in
      // the winning model, but cannot change the score of discarded models.
      const continuity = continuityWeight ? measureContinuity(minimum, span, inlierCount) : 0;
      const continuityFactor = (1 - continuityWeight)
        + continuityWeight * Math.pow(continuity, 1.35);
      const score = weightedSupport * continuityFactor * (1 + Math.min(1, span / 0.82))
        / (expectedPenalty * slopePenalty * anchorPenalty * boundaryPenalty);
      if (score > bestScore) {
        bestScore = score;
        const inliers = Array.from(inlierIndices.subarray(0, inlierCount), index => points[index]);
        best = { inliers, atCenter, span, minimum, maximum,
          continuity: continuityWeight ? continuity : measureContinuity(minimum, span, inlierCount) };
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

  // A pale book can merge with the sheet in a paper silhouette. Preserve the
  // independently observed live contour, but remeasure every rail on this
  // photo before allowing it to anchor still-image reconstruction.
  function observedLiveFrame(seed, width, height, pixels) {
    if (seed?.source !== "yellow" || seed.quad?.length !== 4
      || seed.frameEdges?.length !== 4 || seed.confidence < 0.72) return null;
    const edges = [], supports = [];
    for (const edge of seed.frameEdges) {
      if (!Array.isArray(edge) || edge.length < 8
        || edge.some(point => !Number.isFinite(point.x) || !Number.isFinite(point.y)
          || point.x < 0 || point.x > 1 || point.y < 0 || point.y > 1)) return null;
      const points = edge.map(point => ({ x: point.x * width, y: point.y * height }));
      let hits = 0;
      const measured = points.map((point, index) => {
        const before = points[Math.max(0, index - 1)], after = points[Math.min(points.length - 1, index + 1)];
        const length = Math.hypot(after.x - before.x, after.y - before.y) || 1;
        const nx = -(after.y - before.y) / length, ny = (after.x - before.x) / length;
        const chromaAt = (x, y) => {
          if (x < 1 || y < 1 || x >= width - 1 || y >= height - 1) return -100;
          const at = (Math.round(y) * width + Math.round(x)) * 4;
          const r = pixels[at], g = pixels[at + 1], b = pixels[at + 2];
          return g >= r * 0.7 && r >= g * 0.82 && b < g * 0.96
            ? yellowChroma(r, g, b) : -100;
        };
        let best = null;
        // Six analysis pixels allow sub-frame motion, not a jump to another
        // sheet. A narrow peak must have paper on both sides of the printed ink.
        for (let step = -12; step <= 12; step += 1) {
          const offset = step * 0.5;
          const x = point.x + nx * offset, y = point.y + ny * offset;
          const center = chromaAt(x, y);
          const contrast = center - Math.max(chromaAt(x + nx * 3.5, y + ny * 3.5),
            chromaAt(x - nx * 3.5, y - ny * 3.5));
          if (center < 9 || contrast < 3) continue;
          const score = contrast - Math.abs(offset) * 0.65;
          if (!best || score > best.score) best = { x, y, score };
        }
        if (best) hits += 1;
        return best ? { x: best.x, y: best.y } : point;
      });
      edges.push(measured);
      supports.push(hits / points.length);
    }
    if (Math.min(...supports) < 0.58 || supports.filter(value => value >= 0.78).length < 3) return null;
    // Shared endpoints keep a watertight surface; their correction is the
    // average of the two independently measured arms at each corner.
    const quad = edges.map((edge, index) => {
      const previous = edges[(index + 3) % 4].at(-1);
      return { x: (edge[0].x + previous.x) / 2, y: (edge[0].y + previous.y) / 2 };
    });
    if (!isPlausibleQuad(quad, width, height, { minAreaRatio: 0.12 })) return null;
    edges.forEach((edge, index) => { edge[0] = quad[index]; edge[edge.length - 1] = quad[(index + 1) % 4]; });
    return { quad, edges, supports };
  }

  function contourDistance(frame, observed, width, height) {
    if (!frame?.paths || !observed) return { maximum: Infinity, mean: Infinity };
    const reference = observed.edges.flat();
    let maximum = 0, sum = 0, count = 0;
    for (const path of Object.values(frame.paths)) for (const point of path) {
      let nearest = Infinity;
      for (let index = 0; index < reference.length - 1; index += 1) {
        const a = reference[index], b = reference[index + 1];
        const dx = b.x - a.x, dy = b.y - a.y;
        const t = clamp(((point.x - a.x) * dx + (point.y - a.y) * dy) / (dx * dx + dy * dy || 1), 0, 1);
        nearest = Math.min(nearest, Math.hypot(point.x - a.x - dx * t, point.y - a.y - dy * t));
      }
      maximum = Math.max(maximum, nearest); sum += nearest; count += 1;
    }
    return { maximum: maximum / Math.hypot(width, height), mean: sum / Math.max(1, count) / Math.hypot(width, height) };
  }

  function detectPageAtScale(source, analysisMaximum, options = {}) {
    const profile = {}; let checkpoint = performance.now();
    const measured = name => { const now=performance.now(); profile[name]=now-checkpoint; checkpoint=now; };
    const analysis = createScaledCanvas(source, analysisMaximum);
    const { canvas, context, scale } = analysis;
    const { width, height } = canvas;
    const pixels = detectionImage(context, width, height).data;
    const liveFrame = observedLiveFrame(options.liveDetection, width, height, pixels);
    const paperQuad = detectPaperQuad(width, height, pixels);
    const paperYellowBaseline = estimatePaperYellowBaseline(width, height, pixels, paperQuad);
    measured('paperMs');
    const mask = new Uint8Array(width * height);
    const traceMask = new Uint8Array(width * height);
    let yellowPixels = 0;
    for (let i = 0, p = 0; i < pixels.length; i += 4, p += 1) {
      const strictYellow = isYellow(pixels[i], pixels[i + 1], pixels[i + 2])
        && hasLocalYellowRidge(p, width, height, pixels);
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
    measured('initialMaskAndMarkersMs');
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

    measured('refinedMaskAndMarkersMs');
    // The unmarked-frame fallback must use rails on the observed sheet.
    // Strong yellow material outside the paper otherwise supplies the first
    // and last pixels on every column, replacing the real top/bottom border
    // and making the fitted side curves turn back on themselves. Keep the
    // full mask for independent marker candidates (overlapping sheets), but
    // exclude exterior distractors from this silhouette-based line seed.
    const boundaryMask = paperQuad ? mask.slice() : mask;
    if (paperQuad) {
      for (let index = 0; index < boundaryMask.length; index += 1) {
        if (!boundaryMask[index]) continue;
        const location = invertBilinear(paperQuad, { x: index % width, y: Math.floor(index / width) });
        if (!location || location.u < 0.008 || location.u > 0.992
          || location.v < 0.008 || location.v > 0.992) boundaryMask[index] = 0;
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
        if (!boundaryMask[offset + x]) continue;
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
        if (!boundaryMask[y * width + x]) continue;
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
    if (liveFrame) {
      const paper = extrapolateStencil(liveFrame.quad);
      const guided = detectMarkerGuidedStencil(paper, width, height, pixels, mask, { trustedAxis: true });
      // The asymmetric lower strip determines physical orientation; the live
      // contour itself stays tied to its measured pixels in all orientations.
      let rotation = 0, bestDistance = Infinity;
      if (guided) for (let turn = 0; turn < 4; turn += 1) {
        const distance = liveFrame.quad.reduce((sum, _, index) => sum + Math.hypot(
          liveFrame.quad[(index + turn) % 4].x - guided.stencilQuad[index].x,
          liveFrame.quad[(index + turn) % 4].y - guided.stencilQuad[index].y), 0);
        if (distance < bestDistance) { bestDistance = distance; rotation = turn; }
      }
      const quad = liveFrame.quad.map((_, index) => liveFrame.quad[(index + rotation) % 4]);
      const edges = liveFrame.edges.map((_, index) => liveFrame.edges[(index + rotation) % 4]);
      const frame = traceYellowFrame(quad, width, height, mask, false, true, traceMask);
      if (frame) {
        const resample = edge => Array.from({ length: 25 }, (_, index) => sampleCurve(edge, index / 24));
        frame.paths = { top: resample(edges[0]), right: resample(edges[1]),
          bottom: resample(edges[2].slice().reverse()), left: resample(edges[3].slice().reverse()) };
        frame.corners = frame.refinedStencilQuad = quad;
        frame.sideSupports = liveFrame.supports.map((_, index) => liveFrame.supports[(index + rotation) % 4]);
        frame.support = frame.sideSupports.reduce((sum, value) => sum + value, 0) / 4;
        frame.confidence = clamp((frame.support - 0.16) / 0.68, 0, 1);
        frame.box.outer = frame.paths.bottom;
        const confidence = Math.min(0.99, frame.support);
        resolvedMarkerAttempts.push({ paper: extrapolateStencil(quad), source: "observed-contour",
          diagnostics: { measuredSideSupports: frame.sideSupports },
          guided: { ...(guided || {}), stencilQuad: quad, confidence }, frame,
          score: frame.support * 2 + frame.box.support * 1.25 + confidence * 0.45 });
      }
      for (const attempt of resolvedMarkerAttempts) {
        attempt.seedAgreement = contourDistance(attempt.frame, liveFrame, width, height);
        // A high aggregate score cannot compensate for following a different
        // sheet. Compare complete curved rails, not their straight chords.
        if (attempt.seedAgreement.maximum > 0.025 || attempt.seedAgreement.mean > 0.009) attempt.score -= 4;
      }
      resolvedMarkerAttempts.sort((first, second) => second.score - first.score);
    }
    // A colour-anchored coordinate system is independent of the pale-paper
    // segmentation and therefore remains trustworthy when several sheets
    // touch. Prefer it whenever it can explain a substantial part of both the
    // outer frame and the calibration boxes; the paper fit is only a fallback
    // for photos where one of the coloured references is genuinely lost.
    const reliableAttempt = attempt => {
      const frame = attempt.frame;
      if (!frame) return false;
      if (liveFrame && (attempt.seedAgreement?.maximum > 0.025 || attempt.seedAgreement?.mean > 0.009)) return false;
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
      measured('fitFramesMs'); result.analysisTimings = profile;
      if (options.liveDetection?.source === "yellow" && !liveFrame) {
        // A thin, antialiased yellow line may fail the stricter seed ridge
        // test while an independent marker/rail reconstruction observes it.
        // Accept that reconstruction only on the same complete contour and
        // with separately measured support on this photograph's pixels.
        const seed = options.liveDetection;
        const validEdges = seed.frameEdges?.length === 4 && seed.frameEdges.every(edge => edge?.length >= 8
          && edge.every(point => Number.isFinite(point.x) && Number.isFinite(point.y)
            && point.x >= 0 && point.x <= 1 && point.y >= 0 && point.y <= 1));
        const agreement = validEdges && result.frame && contourDistance(result.frame,
          { edges: seed.frameEdges.map(edge => edge.map(point => ({x:point.x*source.width,y:point.y*source.height}))) },
          source.width, source.height);
        const evidence = result.quality?.valid && result.method === 'marker-guided' && agreement
          && agreement.maximum <= .025 && agreement.mean <= .009
          ? measureFrameEvidence(source, [result])[0] : null;
        const recovered = evidence && Math.min(...evidence.sideSupports) >= .58
          && evidence.sideSupports.filter(value => value >= .78).length >= 3;
        result.quality = recovered
          ? { ...result.quality, valid:true, reason:'independently-observed-contour', seedAgreement:agreement,
            measuredSideSupports:evidence.sideSupports }
          : { valid:false, reason:'live-contour-not-observed', seedAgreement:agreement || null };
      } else if (!result.quality) {
        result.quality = { valid: false, reason: "page-frame-not-verified", seedAgreement: null };
      }
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
        quality: { valid: true, reason: liveFrame ? "observed-contour-agreement" : "frame-and-calibration",
          seedAgreement: markerResolved.seedAgreement || null, sideSupports: tracedFrame.sideSupports },
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
        quality: { valid: quadFitsObservedImage(extrapolateStencil(resolvedStencil), width, height)
          && (!liveFrame || (contourDistance(tracedFrame, liveFrame, width, height).maximum <= 0.025
          && contourDistance(tracedFrame, liveFrame, width, height).mean <= 0.009)),
          reason: liveFrame ? "contour-consistency" : "frame-only",
          seedAgreement: liveFrame ? contourDistance(tracedFrame, liveFrame, width, height) : null },
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

  function refineObservedFrame(source, detection) {
    const frame = detection.frame;
    if (!frame?.paths || frame.confidence < 0.6) return detection;
    const analysis = createScaledCanvas(source, 1200);
    const { width, height } = analysis.canvas;
    const pixels = detectionImage(analysis.context, width, height).data;
    const scoreAt = (x, y) => {
      const ix = Math.round(x), iy = Math.round(y);
      if (ix < 1 || iy < 1 || ix >= width - 1 || iy >= height - 1) return 0;
      const offset = (iy * width + ix) * 4;
      const r = pixels[offset], g = pixels[offset + 1], b = pixels[offset + 2];
      const saturation = (Math.max(r, g, b) - Math.min(r, g, b)) / Math.max(1, r, g, b);
      return isYellow(r, g, b) ? Math.max(0, yellowChroma(r, g, b))
        : saturation < 0.16 ? Math.max(0, 255 - (r + g + b) / 3) * 0.22 : 0;
    };
    const refine = (path, radius) => path.map((point, index) => {
      if (index < 2 || index >= path.length - 2) return point;
      const before = path[index - 1], after = path[index + 1];
      const length = Math.max(1, Math.hypot(after.x - before.x, after.y - before.y));
      const tx = (after.x - before.x) / length, ty = (after.y - before.y) / length;
      const nx = -ty, ny = tx;
      const x = point.x * analysis.scale, y = point.y * analysis.scale;
      let best = null;
      const candidates = [];
      for (let shift = -radius; shift <= radius; shift += 0.5) {
        let score = 0;
        for (let along = -5; along <= 5; along += 1) {
          const cx = x + nx * shift + tx * along;
          const cy = y + ny * shift + ty * along;
          score += Math.max(0, scoreAt(cx, cy)
            - (scoreAt(cx + nx * 4, cy + ny * 4) + scoreAt(cx - nx * 4, cy - ny * 4)) * 0.5);
        }
        const candidate = { shift, score };
        candidates.push(candidate);
        if (!best || score > best.score) best = candidate;
      }
      if (!best || best.score < 20) return point;
      // Centre the observed ridge rather than its brightest camera pixel.
      // This prevents integer downsampling from shifting a thin frame by
      // several pixels after enlarging the final page.
      let weight = 0, weightedShift = 0;
      for (const candidate of candidates) {
        if (Math.abs(candidate.shift - best.shift) > 2.5) continue;
        const amount = Math.max(0, candidate.score - best.score * 0.35);
        weight += amount;
        weightedShift += candidate.shift * amount;
      }
      const shift = weight ? weightedShift / weight : best.shift;
      return { x: point.x + nx * shift / analysis.scale, y: point.y + ny * shift / analysis.scale };
    });
    for (const name of ["top", "right", "bottom", "left"]) frame.paths[name] = refine(frame.paths[name], 6);
    if (frame.box?.support >= 0.5) {
      frame.box.top = refine(frame.box.top, 4);
      frame.box.bottom = refine(frame.box.bottom, 4);
      frame.box.outer = frame.paths.bottom;
    }
    frame.refinement = "source-ridge";
    analysis.canvas.width = analysis.canvas.height = 0;
    return detection;
  }

  function measureFrameEvidence(source, detections) {
    // Candidate support is measured on different downsampled masks. Compare
    // both reconstructions against the same source pixels before accepting a
    // finer pass: JPEG and one-pixel motion must not replace an observed rail
    // with a smoother but unsupported bend.
    const analysis = createScaledCanvas(source, 1200);
    const { width, height } = analysis.canvas;
    const pixels = detectionImage(analysis.context, width, height).data;
    const colourAt = (x, y) => {
      const ix = Math.round(x), iy = Math.round(y);
      if (ix < 1 || iy < 1 || ix >= width - 1 || iy >= height - 1) return null;
      const offset = (iy * width + ix) * 4;
      const r = pixels[offset], g = pixels[offset + 1], b = pixels[offset + 2];
      const maximum = Math.max(r, g, b);
      return {
        luminance: (r + g + b) / 3,
        chroma: (r + g) * 0.5 - b - Math.abs(r - g) * 0.34,
        saturation: (maximum - Math.min(r, g, b)) / Math.max(1, maximum)
      };
    };
    const pointAt = (path, amount) => {
      const point = sampleCurve(path, amount);
      return { x: point.x * analysis.scale, y: point.y * analysis.scale };
    };
    const traceEvidence = path => {
      if (!path?.length || path.some(point => !Number.isFinite(point?.x) || !Number.isFinite(point?.y))) {
        return { support: 0, strength: 0 };
      }
      let hits = 0, total = 0;
      for (let index = 2; index < 63; index += 1) {
        const point = pointAt(path, index / 64);
        const before = pointAt(path, (index - 1) / 64);
        const after = pointAt(path, (index + 1) / 64);
        const length = Math.hypot(after.x - before.x, after.y - before.y) || 1;
        const nx = -(after.y - before.y) / length, ny = (after.x - before.x) / length;
        let best = 0;
        for (let shift = -3; shift <= 3; shift += 0.75) {
          const center = colourAt(point.x + nx * shift, point.y + ny * shift);
          const first = colourAt(point.x + nx * (shift - 4), point.y + ny * (shift - 4));
          const second = colourAt(point.x + nx * (shift + 4), point.y + ny * (shift + 4));
          if (!center || !first || !second) continue;
          const chromaticContrast = center.chroma - Math.max(first.chroma, second.chroma);
          const neutralContrast = center.saturation < 0.2
            ? Math.min(first.luminance, second.luminance) - center.luminance : 0;
          best = Math.max(best, Math.max(chromaticContrast, neutralContrast * 0.7) - Math.abs(shift) * 0.35);
        }
        if (best >= 3) hits += 1;
        total += Math.min(1, best / 12);
      }
      return { support: hits / 61, strength: total / 61 };
    };
    const results = detections.map(detection => {
      const sides = ["top", "right", "bottom", "left"]
        .map(name => traceEvidence(detection.frame?.paths?.[name]));
      const rails = ["top", "bottom"].map(name => traceEvidence(detection.frame?.box?.[name]));
      return {
        sideSupports: sides.map(side => side.support),
        score: Math.min(...sides.map(side => side.support)) * 2
          + sides.reduce((sum, side) => sum + side.strength, 0) / 4
          + rails.reduce((sum, rail) => sum + rail.strength, 0) * 0.2
      };
    });
    analysis.canvas.width = analysis.canvas.height = 0;
    return results;
  }

  Light.detectPage = function detectPage(source, options = {}) {
    if (!options.liveDetection && SP.LiveDetection?.detect) {
      const preview = createScaledCanvas(source, 480);
      const liveDetection = SP.LiveDetection.detect(preview.context.getImageData(0, 0, preview.canvas.width, preview.canvas.height));
      preview.canvas.width = preview.canvas.height = 0;
      options = { ...options, liveDetection };
    }
    const quick = detectPageAtScale(source, options.forceDetailed ? ANALYSIS_MAX : ANALYSIS_FAST_MAX, options);
    const frame = quick.frame;
    const finiteQuickQuad = quick.pageQuad?.length === 4
      && quick.pageQuad.every(point => Number.isFinite(point?.x) && Number.isFinite(point?.y));
    let needsDetailedPass = !finiteQuickQuad
      || quick.method === "fallback"
      || (quick.method === "paper" && quick.confidence > 0)
      || (quick.method === "marker-guided"
        && (quick.confidence < 0.96
          || (frame?.support || 0) < 0.42
          || (frame?.box?.support || 0) < 0.42
          // Strong paper curl is precisely the case where the extra 160 px
          // of analysis materially improves the physical corner fit. Keep
          // ordinary scans on the fast path, but never accept a coarse trace
          // whose measured bend is already this large.
          || (frame?.curvature || 0) > 0.03
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
    if (options.forceDetailed) {
      needsDetailedPass = false;
      quick.scaleSelection = {resolution:ANALYSIS_MAX,reason:'raster-recovery'};
    }
    const finalize = selected => {
      let detection = refineObservedFrame(source, selected);
      detection = SP.SurfaceParameterization?.refine(source, detection) || detection;
      if (detection.quality?.valid && options.liveDetection?.frameEdges?.length === 4) {
        const width = source.naturalWidth || source.videoWidth || source.width;
        const height = source.naturalHeight || source.videoHeight || source.height;
        const observed = { edges: options.liveDetection.frameEdges.map(edge => edge.map(point => ({
          x: point.x * width, y: point.y * height
        }))) };
        const agreement = contourDistance(detection.frame, observed, width, height);
        detection.quality.seedAgreement = agreement;
        if (agreement.maximum > 0.025 || agreement.mean > 0.009) {
          detection.quality.valid = false;
          detection.quality.reason = 'refined-contour-disagreement';
        }
      }
      if (detection.quality?.valid && detection.frame && SP.MetricSurface?.fit) {
        const surfaceDiagnostics = {};
        const surface = SP.MetricSurface.fit(source, detection, {
          diagnostics: surfaceDiagnostics,
          budgetMs: options.surfaceBudgetMs,
          onObservations: detail => {
            options.onProgress?.('dots', { ...detail, space:'source' });
            options.onProgress?.('surface', { fitting:true });
          },
          onRegistration: detail => options.onProgress?.('surface', { ...detail, space:'source', registered:true })
        });
        if (surface) detection.frame.metricSurface = surface;
        else detection.frame.surfaceDiagnostics = surfaceDiagnostics;
      }
      return detection;
    };
    let selected = quick, completed = null;
    if (needsDetailedPass && quick.quality?.valid && quick.method === 'marker-guided' && quick.confidence >= .98
      && quick.frame?.support >= .88 && quick.frame?.box?.support >= .72
      && quick.frame.sideSupports?.every(support => support >= .7)) {
      // The finer border pass is a recovery path. If the smaller analysis
      // already explains the independently measured full-source dots and
      // perimeter with a validated 3D surface, don't repeat the whole search.
      const candidate = finalize(JSON.parse(JSON.stringify(quick)));
      if (candidate.frame?.metricSurface) {
        completed = candidate;
        completed.scaleSelection = {resolution:ANALYSIS_FAST_MAX,reason:'verified-full-source-surface'};
        needsDetailedPass = false;
      }
    }
    if (needsDetailedPass) {
      const detailed = detectPageAtScale(source, ANALYSIS_MAX, options);
      const weakest = result => Math.min(...(result.frame?.sideSupports || [0]));
      const coarseHasCompleteEvidence = quick.method === "marker-guided"
        && quick.frame?.support >= 0.9 && quick.frame?.box?.support >= 0.85
        && (quick.quality?.valid !== false || detailed.quality?.valid !== true);
      const [coarseEvidence, detailedEvidence] = measureFrameEvidence(source, [quick, detailed]);
      // Extra pixels are another observation, not proof of a better fit.
      // Raster rounding or JPEG can break a fine-scale rail and make RANSAC
      // invent a corner even while the coarse pass explains all four sides.
      const degraded = coarseHasCompleteEvidence && (
        (detailed.method !== "marker-guided" && detailed.frame?.support <= quick.frame.support + 0.03)
        || (weakest(quick) >= 0.94 && weakest(detailed) < weakest(quick) - 0.04)
        || detailedEvidence.score < coarseEvidence.score - 0.06
        || (quick.quality?.valid === true && detailed.quality?.valid === false));
      if (!degraded) selected = detailed;
      selected.scaleSelection = { resolution: degraded ? ANALYSIS_FAST_MAX : ANALYSIS_MAX,
        coarseEvidence, detailedEvidence };
    }
    return completed || finalize(selected);
  };

  Light.detectPageAsync = async function detectPageAsync(source, options = {}) {
    checkProcessingCancelled(options);
    const fallback = async () => {
      await globalThis.InhouseRust?.ready;
      await yieldToBrowser();
      checkProcessingCancelled(options);
      return Light.detectPage(source, options);
    };
    if (!moduleUrl || typeof Worker !== "function" || typeof createImageBitmap !== "function"
      || typeof OffscreenCanvas !== "function") return fallback();
    const workerUrl = new URL("detection-worker.js", moduleUrl);
    workerUrl.search = new URL(moduleUrl).search;
    let bitmap;
    try { bitmap = await createImageBitmap(source); }
    catch (_) { return fallback(); }
    if (options.signal?.aborted) {
      bitmap.close();
      checkProcessingCancelled(options);
    }
    return new Promise((resolve, reject) => {
      let worker;
      let settled = false;
      let timer;
      const finish = (callback, value) => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        options.signal?.removeEventListener("abort", abort);
        worker?.terminate();
        callback(value);
      };
      const abort = () => finish(reject, new DOMException("Scan processing cancelled", "AbortError"));
      const unavailable = () => finish(resolve, fallback());
      try {
        worker = new Worker(workerUrl);
        worker.onmessage = event => {
          if (settled) return;
          const data = event.data;
          if (data.progress) { options.onProgress?.(data.progress, data.detail); return; }
          if (data.error) finish(reject, new Error(data.error));
          else finish(resolve, data.detection);
        };
        worker.onerror = unavailable;
        options.signal?.addEventListener("abort", abort, { once: true });
        timer = setTimeout(() => finish(reject, new Error("Page analysis took too long. Try another frame.")),
          Math.max(1000, Math.min(60000, options.timeoutMs || 20000)));
        worker.postMessage({ bitmap, config: SP.Config || {}, liveDetection: options.liveDetection || null,
          reportProgress: !!options.onProgress, surfaceBudgetMs: options.surfaceBudgetMs }, [bitmap]);
      } catch (_) {
        bitmap.close();
        unavailable();
      }
    });
  };

  async function decodeScanBlob(blob, options) {
    checkProcessingCancelled(options);
    if (typeof createImageBitmap === "function") {
      try {
        const bitmap = await createImageBitmap(blob);
        if (options.signal?.aborted) {
          bitmap.close();
          checkProcessingCancelled(options);
        }
        return bitmap;
      } catch (error) {
        checkProcessingCancelled(options);
        if (typeof Image !== "function") {
          // Some supported imports (notably SVG in Chromium) require the
          // document's image decoder even when JPEG works in the worker.
          error.needsImageDecoder = true;
          throw error;
        }
      }
    }
    return new Promise((resolve, reject) => {
      const image = new Image();
      const url = URL.createObjectURL(blob);
      const finish = (callback, value) => {
        image.onload = image.onerror = null;
        options.signal?.removeEventListener("abort", abort);
        URL.revokeObjectURL(url);
        callback(value);
      };
      const abort = () => {
        finish(reject, new DOMException("Scan processing cancelled", "AbortError"));
        image.src = "";
      };
      image.onload = () => finish(resolve, image);
      image.onerror = () => finish(reject, new Error("This image could not be opened"));
      options.signal?.addEventListener("abort", abort, { once: true });
      if (options.signal?.aborted) { abort(); return; }
      image.src = url;
    });
  }

  function warpedObservations(observed, mesh, width, height) {
    if (!observed?.points?.length || !mesh) return null;
    const { us, vs, grid } = mesh;
    const cellAt = (values, at) => {
      let lo = 0, hi = values.length - 1;
      while (hi - lo > 1) {
        const mid = (lo + hi) >> 1;
        if (at < values[mid]) hi = mid; else lo = mid;
      }
      return Math.min(values.length - 2, lo);
    };
    const points = [], indices = new Map();
    observed.points.forEach((point, index) => {
      let u = point.u, v = point.v, error = Infinity;
      // Invert the exact bilinear cells that sampled the photograph, rather
      // than snapping the detected dots to their ideal template positions.
      for (let iteration = 0; iteration < 7; iteration++) {
        const column = cellAt(us, u), row = cellAt(vs, v);
        const du = us[column + 1] - us[column], dv = vs[row + 1] - vs[row];
        const s = (u - us[column]) / du, t = (v - vs[row]) / dv;
        const a = grid[row][column], b = grid[row][column + 1];
        const c = grid[row + 1][column], d = grid[row + 1][column + 1];
        const x = (a.x * (1 - s) + b.x * s) * (1 - t) + (c.x * (1 - s) + d.x * s) * t;
        const y = (a.y * (1 - s) + b.y * s) * (1 - t) + (c.y * (1 - s) + d.y * s) * t;
        const ex = x - point.x, ey = y - point.y;
        error = Math.hypot(ex, ey);
        if (error < .01) break;
        const xu = ((b.x - a.x) * (1 - t) + (d.x - c.x) * t) / du;
        const yu = ((b.y - a.y) * (1 - t) + (d.y - c.y) * t) / du;
        const xv = ((c.x - a.x) * (1 - s) + (d.x - b.x) * s) / dv;
        const yv = ((c.y - a.y) * (1 - s) + (d.y - b.y) * s) / dv;
        const determinant = xu * yv - xv * yu;
        if (!Number.isFinite(determinant) || Math.abs(determinant) < 1e-8) break;
        u = clamp(u - (ex * yv - ey * xv) / determinant, 0, 1);
        v = clamp(v - (ey * xu - ex * yu) / determinant, 0, 1);
      }
      // Uninvertible observations are omitted, never fabricated. Line runs
      // below split at those gaps instead of drawing across missing evidence.
      if (!Number.isFinite(u + v) || error > .1) return;
      indices.set(index, points.length);
      points.push({ ...point, x: u * width, y: v * height,
        targetX: point.u * width, targetY: point.v * height });
    });
    const remapRuns = runs => (runs || []).flatMap(run => {
      const result = []; let group = [];
      for (const index of run.points) {
        if (indices.has(index)) group.push(indices.get(index));
        else { if (group.length > 1) result.push({ index: run.index, points: group }); group = []; }
      }
      if (group.length > 1) result.push({ index: run.index, points: group });
      return result;
    });
    return { width, height, pitch: width * 5 / 210, space: "corrected", points,
      rows: remapRuns(observed.rows), columns: remapRuns(observed.columns) };
  }

  async function scanPreviewBlob(output, options) {
    const preview = createScaledCanvas(output, 720).canvas;
    try {
      checkProcessingCancelled(options);
      // Keep the native paper references visible through every corrected
      // progress image. Never show the temporary template-free ink buffer.
      if (options.precise) SP.ScanTemplate.render(preview.getContext("2d"), preview.width, preview.height, { background: false });
      return typeof preview.convertToBlob === "function"
        ? await preview.convertToBlob({ type: "image/jpeg", quality: .9 })
        : await new Promise((resolve, reject) => preview.toBlob(value => value?.size
          ? resolve(value) : reject(new Error("Could not preview this scan")), "image/jpeg", .9));
    } finally { preview.width = preview.height = 0; }
  }

  async function encodeScanCanvas(canvas, type = "image/png") {
    const blob = typeof canvas.convertToBlob === "function"
      ? await canvas.convertToBlob({ type, quality: .94 })
      : await new Promise((resolve, reject) => canvas.toBlob(value => value?.size
        ? resolve(value) : reject(new Error("Could not encode this scan")), type, .94));
    if (!blob?.size) throw new Error("Could not encode this scan");
    return blob;
  }

  function separateScanInk(canvas) {
    const context = canvas.getContext("2d", { willReadFrequently: true });
    const pixels = context.getImageData(0, 0, canvas.width, canvas.height);
    const data = pixels.data;
    // Reconstruct the cleaned photograph exactly over white, but keep paper
    // transparent. Neutral pencil becomes black with proportional opacity;
    // coloured ink retains its hue. The editor can therefore draw its own
    // uncompressed native template beneath the ink, just like a normal page.
    if (!globalThis.InhouseRust?.separateInk?.(data)) for (let i = 0; i < data.length; i += 4) {
      const paper = Math.min(data[i], data[i + 1], data[i + 2]);
      const alpha = 255 - paper;
      if (!alpha) { data[i] = data[i + 1] = data[i + 2] = data[i + 3] = 0; continue; }
      data[i] = Math.round((data[i] - paper) * 255 / alpha);
      data[i + 1] = Math.round((data[i + 1] - paper) * 255 / alpha);
      data[i + 2] = Math.round((data[i + 2] - paper) * 255 / alpha);
      data[i + 3] = alpha;
    }
    context.putImageData(pixels, 0, 0);
  }

  async function processScanBlob(blob, options) {
    const started = performance.now();
    const timings = { worker: processingWorker, decodeMs: 0, detectionMs: 0, warpMs: 0,
      alignmentMs: 0, previewMs: 0, colorMs: 0, templateMs: 0, encodeMs: 0 };
    let decoded, source, output, preview, observed, mesh;
    let templateApplied = false, stencilCleanup, outputQuality, dotAlignment, editableInk;
    const progress = (stage, detail) => { checkProcessingCancelled(options); options.onProgress?.(stage, detail); };
    try {
      progress("decoding");
      decoded = await decodeScanBlob(blob, options);
      const width = decoded.naturalWidth || decoded.width;
      const height = decoded.naturalHeight || decoded.height;
      if (!width || !height) throw new Error("This image is empty");
      source = createScaledCanvas(decoded, 2560).canvas;
      decoded.close?.(); decoded = null;
      timings.decodeMs = performance.now() - started;
      progress("detection");
      const detectionStarted = performance.now();
      // The same yielding fallback also supports older WebViews without
      // OffscreenCanvas; current Android performs all of these stages in a
      // dedicated worker without sending full-size pixel buffers to the UI.
      const detection = await Light.detectPageAsync(source, {
        ...options,
        // A three-photo burst can share one CPU core. Preserve the same
        // available fitting work per photograph instead of dropping its 3D
        // surface simply because two other workers used the wall clock.
        surfaceBudgetMs: processingWorker ? 7800 : undefined,
        onProgress(stage, detail) {
          if (stage === "dots" || (stage === "surface" && detail?.registered)) observed = detail;
          progress(stage, detail);
        }
      });
      timings.detectionMs = performance.now() - detectionStarted;
      checkProcessingCancelled(options);
      const precise = ["stencil", "marker-guided"].includes(detection.method) && detection.confidence >= .55;
      const quad = precise ? detection.pageQuad : options.liveDetection?.pageQuad?.map(point => ({
        x: point.x * source.width, y: point.y * source.height
      }));
      if (detection.quality?.valid === false || !quad) {
        timings.totalMs = performance.now() - started;
        return { blob, width, height, corrected: false, templateApplied, detection, timings,
          failureReason: detection.quality?.reason || "Page edges could not be verified" };
      }
      progress("surface", { verified: !!detection.frame?.metricSurface,
        diagnostics: detection.frame?.metricSurface?.diagnostics });
      progress("warp");
      const warpStarted = performance.now();
      output = await Light.warp(source, quad, precise ? detection.frame : null,
        { signal: options.signal, maxDimension: options.maxDimension, onMesh: value => { mesh = value; },
          onRegistration: detail => {
            observed = detail;
            progress("surface", { ...detail, space: "source", registered: true });
          } });
      timings.warpMs = performance.now() - warpStarted;
      source.width = source.height = 0; source = null;
      if (precise && SP.MetricSurface?.certifyRaster) {
        const alignmentStarted = performance.now();
        dotAlignment = SP.MetricSurface.certifyRaster(output);
        timings.alignmentMs = performance.now() - alignmentStarted;
        checkProcessingCancelled(options);
        if (dotAlignment.status === "failed") {
          if (!options.forceDetailed && detection.scaleSelection?.reason === 'verified-full-source-surface') {
            // A valid physical fit still has to explain the final pixels.
            // Retry the finer observation before rejecting this photograph.
            const recoveryMs = performance.now()-started;
            output.width=output.height=0; output=null; mesh=null; observed=null;
            progress('detection',{retry:true});
            const recovered = await processScanBlob(blob,{...options,forceDetailed:true});
            recovered.timings = {...recovered.timings,recoveryMs,totalMs:performance.now()-started};
            return recovered;
          }
          // A convincing yellow perimeter does not certify the interior of
          // a bent sheet. Keep the original instead of hiding a failed warp
          // beneath a straight replacement grid. Other burst frames can win.
          detection.quality = { ...detection.quality, valid: false, reason: "interior-dots-misaligned" };
          timings.totalMs = performance.now() - started;
          return { blob, width, height, corrected: false, templateApplied: false,
            dotAlignment, detection, timings, failureReason: "The page dots could not be aligned. Try another frame." };
        }
      }
      const previewStarted = performance.now();
      const correctedDots = warpedObservations(observed, mesh, output.width, output.height);
      mesh = null;
      if (options.previewProgress && options.onProgress) {
        try {
          const previewBlob = await scanPreviewBlob(output, { ...options, precise });
          progress("warp", { ...correctedDots, width: output.width, height: output.height,
            space: "corrected", previewBlob });
        } catch (_) {
          // Diagnostic thumbnails are optional. A preview encoder failure
          // must not discard a valid full-resolution correction.
          checkProcessingCancelled(options);
        }
      }
      timings.previewMs = performance.now() - previewStarted;
      if (precise) {
        progress("colors");
        const colorsStarted = performance.now();
        const colors = await Light.correctColors(output, { useStencil: true, preciseStencil: true,
          dotObservations: correctedDots?.points, signal: options.signal });
        timings.colors = colors?.pixelProcessing;
        stencilCleanup = colors?.stencilCleanup;
        outputQuality = colors?.outputQuality;
        timings.colorMs = performance.now() - colorsStarted;
        separateScanInk(output);
        progress("vectors");
        const vectorStarted = performance.now();
        const inkContext = output.getContext("2d", { willReadFrequently: true });
        editableInk = await SP.InkVectorizer.fromPixels(inkContext.getImageData(0, 0, output.width, output.height),
          { signal: options.signal });
        // Review exactly the same opaque native pens that will be inserted.
        // The photograph is never left behind underneath editable strokes.
        inkContext.clearRect(0, 0, output.width, output.height);
        SP.InkVectorizer.render(inkContext, editableInk);
        timings.vectorMs = performance.now() - vectorStarted;
        progress("template");
        const templateStarted = performance.now();
        preview = document.createElement("canvas");
        preview.width = output.width; preview.height = output.height;
        const previewContext = preview.getContext("2d");
        SP.ScanTemplate.render(previewContext, preview.width, preview.height, { background: true });
        previewContext.drawImage(output, 0, 0);
        templateApplied = true;
        timings.templateMs = performance.now() - templateStarted;
      }
      progress("encoding");
      const encodeStarted = performance.now();
      const [encoded, previewBlob] = await Promise.all([
        encodeScanCanvas(output, precise ? "image/png" : "image/jpeg"),
        preview ? encodeScanCanvas(preview) : undefined
      ]);
      checkProcessingCancelled(options);
      if (!encoded?.size) throw new Error("Could not encode this scan");
      // Encoding and decoding happen on the worker as well. A malformed image
      // must never be advertised as the winning corrected capture.
      decoded = await decodeScanBlob(encoded, options);
      if ((decoded.naturalWidth || decoded.width) !== output.width
        || (decoded.naturalHeight || decoded.height) !== output.height) throw new Error("The scan could not be verified");
      decoded.close?.(); decoded = null;
      timings.encodeMs = performance.now() - encodeStarted;
      timings.totalMs = performance.now() - started;
      return { blob: encoded, width: output.width, height: output.height, corrected: true,
        previewBlob, editableInk, inkLayer: precise, templateKind: precise ? "default" : undefined,
        templateApplied, stencilCleanup, outputQuality, dotAlignment, detection, timings };
    } finally {
      decoded?.close?.();
      if (source) source.width = source.height = 0;
      if (output) output.width = output.height = 0;
      if (preview) preview.width = preview.height = 0;
    }
  }

  Light.processPageAsync = function processPageAsync(blob, options = {}) {
    return new Promise((resolve, reject) => {
      const controller = new AbortController();
      const settings = { ...options, signal: controller.signal };
      let worker, settled = false, fallbackStarted = false, timer;
      const finish = (callback, value) => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        options.signal?.removeEventListener("abort", abort);
        worker?.terminate();
        controller.abort();
        callback(value);
      };
      const abort = () => finish(reject, new DOMException("Scan processing cancelled", "AbortError"));
      const fallback = () => {
        if (settled || fallbackStarted) return;
        fallbackStarted = true;
        worker?.terminate(); worker = null;
        processScanBlob(blob, settings).then(value => finish(resolve, value), error => finish(reject, error));
      };
      options.signal?.addEventListener("abort", abort, { once: true });
      if (options.signal?.aborted) { abort(); return; }
      timer = setTimeout(() => finish(reject, new Error("Page correction took too long. Try another frame.")),
        Math.max(1000, Math.min(60000, options.timeoutMs || 20000)));
      if (!moduleUrl || typeof Worker !== "function" || typeof createImageBitmap !== "function"
        || typeof OffscreenCanvas !== "function") { fallback(); return; }
      try {
        const workerUrl = new URL("detection-worker.js", moduleUrl);
        workerUrl.search = new URL(moduleUrl).search;
        worker = new Worker(workerUrl);
        worker.onmessage = event => {
          if (settled || fallbackStarted) return;
          const data = event.data;
          if (data.progress) { options.onProgress?.(data.progress, data.detail); return; }
          if (data.unsupported || data.needsImageDecoder) { fallback(); return; }
          if (data.error) {
            const error = new Error(data.error); error.name = data.name || "Error";
            finish(reject, error);
          } else finish(resolve, data.result);
        };
        worker.onerror = fallback;
        worker.postMessage({ operation: "process", blob, config: SP.Config || {},
          liveDetection: options.liveDetection || null, maxDimension: options.maxDimension,
          timeoutMs: options.timeoutMs, previewProgress: options.previewProgress === true });
      } catch (_) { fallback(); }
    });
  };

  function checkProcessingCancelled(settings) {
    if (settings?.signal?.aborted) {
      throw new DOMException("Scan processing cancelled", "AbortError");
    }
  }

  function projectivePageMap(quad) {
    const [p0, p1, p2, p3] = quad;
    const dx1 = p1.x - p2.x, dx2 = p3.x - p2.x;
    const dy1 = p1.y - p2.y, dy2 = p3.y - p2.y;
    const dx3 = p0.x - p1.x + p2.x - p3.x;
    const dy3 = p0.y - p1.y + p2.y - p3.y;
    const determinant = dx1 * dy2 - dx2 * dy1;
    if (Math.abs(determinant) < 1e-8) throw new Error("The page corners are too close together.");
    const g = (dx3 * dy2 - dx2 * dy3) / determinant;
    const h = (dx1 * dy3 - dx3 * dy1) / determinant;
    const a = p1.x - p0.x + g * p1.x, b = p3.x - p0.x + h * p3.x;
    const d = p1.y - p0.y + g * p1.y, e = p3.y - p0.y + h * p3.y;
    return (u, v) => {
      const divisor = g * u + h * v + 1;
      return { x: (a * u + b * v + p0.x) / divisor, y: (d * u + e * v + p0.y) / divisor };
    };
  }

  Light.warp = async function warp(source, pageQuad, frame = null, options = {}) {
    checkProcessingCancelled(options);
    const output = document.createElement("canvas");
    output.height = Math.round(clamp(options.maxDimension || OUTPUT_LONG_EDGE, 256, OUTPUT_LONG_EDGE));
    output.width = Math.round(output.height * A4_RATIO);
    const context = output.getContext("2d", { willReadFrequently: true });
    const input = source.getContext
      ? source.getContext("2d", { willReadFrequently: true }).getImageData(0, 0, source.width, source.height)
      : createScaledCanvas(source, Math.max(source.width, source.height)).context
        .getImageData(0, 0, source.width, source.height);
    const image = context.createImageData(output.width, output.height);
    const pixels = image.data;
    pixels.fill(255);
    const inputPixels = input.data;
    const sourceWidth = input.width;
    const sourceHeight = input.height;
    const curved = !!frame?.paths && frame.confidence >= 0.18;
    // Inverse-map one dense surface instead of repeatedly drawing the entire
    // camera image into 1,200 clipped triangles. The latter made Android GPU
    // drivers process the same large photo for every cell, and overlapping
    // clips softened writing and left seams. Each output pixel is sampled once.
    const columns = 64;
    const rows = 80;
    const fractions = (count, extra) => [...Array.from({ length: count + 1 }, (_, index) => index / count), ...extra]
      .sort((a, b) => a - b)
      .filter((value, index, values) => !index || value - values[index - 1] > 1e-7);
    // A registered bullet must be a mesh vertex. Uniform 64×80 sampling
    // interpolates across those anchors and reintroduces a different error at
    // each printed dot, even when the surface itself maps its centre exactly.
    const registration = curved ? frame.metricSurface?.registration : null;
    const us = fractions(columns, curved
      ? [1.5 / 21, 19.5 / 21, ...(registration?.colPositions || [])] : []);
    const vs = fractions(rows, curved
      ? [1.43 / 29.7, 27.43 / 29.7, 27.93 / 29.7, 28.43 / 29.7,
        ...(registration?.rowPositions || [])] : []);
    if (registration && SP.MetricSurface.refineForRaster) {
      const detail = SP.MetricSurface.refineForRaster(source, frame.metricSurface, {
        width: output.width, height: output.height, imageData: input, mesh: { us, vs }
      });
      if (detail) options.onRegistration?.(detail);
      checkProcessingCancelled(options);
    }
    const map = curved ? (u, v) => mapFramePoint(frame, u, v) : projectivePageMap(pageQuad);
    const grid = vs.map(v => us.map(u => map(u, v)));
    options.onMesh?.({ us, vs, grid });
    await globalThis.InhouseRust?.ready;
    const rustWarp = options.disableRust ? null : globalThis.InhouseRust?.warp(input, output.width, output.height);
    try {
    const orientation = Math.sign(polygonArea(pageQuad));
    const cross = (a, b, c) => (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    for (let row = 0; row < vs.length - 1; row += 1) {
      checkProcessingCancelled(options);
      const topY = vs[row] * output.height;
      const bottomY = vs[row + 1] * output.height;
      const startY = Math.max(0, Math.ceil(topY - 0.5));
      const endY = Math.min(output.height, Math.ceil(bottomY - 0.5));
      for (let column = 0; column < us.length - 1; column += 1) {
        // The printed template has blank margins outside the measured frame.
        // Extrapolating curled guides into those unobserved margins can fold
        // a mesh over itself or import a neighbouring desk. Keep clean paper
        // there, as the earlier scanner did, and validate all observed cells.
        if (curved && (us[column] < 1.5 / 21 - 1e-7 || us[column + 1] > 19.5 / 21 + 1e-7
          || vs[row] < 1.43 / 29.7 - 1e-7 || vs[row + 1] > 28.43 / 29.7 + 1e-7)) continue;
        const leftX = us[column] * output.width;
        const rightX = us[column + 1] * output.width;
        const startX = Math.max(0, Math.ceil(leftX - 0.5));
        const endX = Math.min(output.width, Math.ceil(rightX - 0.5));
        const tl = grid[row][column], tr = grid[row][column + 1];
        const bl = grid[row + 1][column], br = grid[row + 1][column + 1];
        if (![tl, tr, bl, br].every(point => Number.isFinite(point?.x) && Number.isFinite(point?.y))) {
          throw new Error("The page surface could not be reconstructed. Try another frame.");
        }
        if (cross(tl, tr, bl) * orientation <= 1e-7 || cross(br, bl, tr) * orientation <= 1e-7) {
          throw new Error("The page surface overlaps itself. Keep the whole sheet visible and try another frame.");
        }
        if (rustWarp) {
          rustWarp.cell(leftX, rightX, topY, bottomY, tl, tr, bl, br);
          continue;
        }
        const cellWidth = rightX - leftX;
        const firstU = (startX + 0.5 - leftX) / cellWidth;
        for (let y = startY; y < endY; y += 1) {
          const v = (y + 0.5 - topY) / (bottomY - topY);
          const lx = tl.x + (bl.x - tl.x) * v;
          const ly = tl.y + (bl.y - tl.y) * v;
          const rx = tr.x + (br.x - tr.x) * v;
          const ry = tr.y + (br.y - tr.y) * v;
          const dx = (rx - lx) / cellWidth;
          const dy = (ry - ly) / cellWidth;
          let sx = lx + (rx - lx) * firstU - 0.5;
          let sy = ly + (ry - ly) * firstU - 0.5;
          let destination = (y * output.width + startX) * 4;
          for (let x = startX; x < endX; x += 1, destination += 4, sx += dx, sy += dy) {
            if (sx < 0 || sy < 0 || sx >= sourceWidth - 1 || sy >= sourceHeight - 1) {
              pixels[destination] = pixels[destination + 1] = pixels[destination + 2] = 255;
            } else {
              const ix = Math.floor(sx), iy = Math.floor(sy);
              const fx = sx - ix, fy = sy - iy;
              const first = (iy * sourceWidth + ix) * 4;
              const second = first + sourceWidth * 4;
              for (let channel = 0; channel < 3; channel += 1) {
                const upper = inputPixels[first + channel] * (1 - fx) + inputPixels[first + 4 + channel] * fx;
                const lower = inputPixels[second + channel] * (1 - fx) + inputPixels[second + 4 + channel] * fx;
                pixels[destination + channel] = upper * (1 - fy) + lower * fy;
              }
            }
            pixels[destination + 3] = 255;
          }
        }
      }
      options.onProgress?.((row + 1) / (vs.length - 1));
      if (row % 3 === 2) await yieldToBrowser();
    }
    checkProcessingCancelled(options);
    rustWarp?.copyTo(pixels);
    } finally { rustWarp?.dispose(); }
    context.putImageData(image, 0, 0);
    output.scanSurface = { model: curved ? "curved-guided-surface" : "projective",
      columns: us.length - 1, rows: vs.length - 1, registered: !!registration };
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

  function sampleCalibrationMarker(data, width, height, centerX, centerY, radius, name, paper = null) {
    const candidates = [];
    const minimumX = clamp(Math.floor(centerX - radius), 0, width - 1);
    const maximumX = clamp(Math.ceil(centerX + radius), 0, width - 1);
    const minimumY = clamp(Math.floor(centerY - radius), 0, height - 1);
    const maximumY = clamp(Math.ceil(centerY + radius), 0, height - 1);
    for (let y = minimumY; y <= maximumY; y += 1) {
      for (let x = minimumX; x <= maximumX; x += 1) {
        const distance = Math.hypot(x - centerX, y - centerY) / Math.max(1, radius);
        if (distance > 1) continue;
        const offset = (y * width + x) * 4;
        const rawR = data[offset], rawG = data[offset + 1], rawB = data[offset + 2];
        // A printed pigment is relative to nearby paper, not an absolute
        // camera RGB hue. This also keeps a dark blue swatch measurable under
        // a warm lamp without confusing its neighbouring black disc with it.
        const r = paper ? rawR * 255 / Math.max(20, paper[0]) : rawR;
        const g = paper ? rawG * 255 / Math.max(20, paper[1]) : rawG;
        const b = paper ? rawB * 255 / Math.max(20, paper[2]) : rawB;
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
        } else if (name === "yellow") {
          // Grey, bleached printing still has a measured ridge. Unmarked
          // paper at the expected frame coordinate is never a reference.
          valid = luminance < 249 && (isYellow(r,g,b)
            || (luminance > 45 && luminance < 240 && chroma < 15));
          colourScore = yellowChroma(r,g,b) * 1.5 + (255-luminance) * .25;
        } else {
          valid = luminance < 165 && chroma <= 72;
          colourScore = 255 - luminance - chroma * 0.72;
        }
        if (!valid) continue;
        candidates.push({ r: rawR, g: rawG, b: rawB, score: colourScore - distance * 18 });
      }
    }
    if (candidates.length < 3) return null;
    candidates.sort((first, second) => second.score - first.score);
    const selected = candidates.slice(0, clamp(Math.ceil(candidates.length * 0.28), 5, 12));
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
    const ridge = 0.012;
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

  async function removePhotographedStencil(data, width, height, pxPerCm, targetPaper, settings) {
    const setPaper = (x, y) => {
      if (x < 0 || x >= width || y < 0 || y >= height) return;
      const offset = (y * width + x) * 4;
      data[offset] = targetPaper;
      data[offset + 1] = targetPaper;
      data[offset + 2] = targetPaper;
    };
    const metricsAt = (x, y) => {
      const offset = (y * width + x) * 4;
      const r = data[offset], g = data[offset + 1], b = data[offset + 2];
      const maximum = Math.max(r, g, b);
      const minimum = Math.min(r, g, b);
      return {
        luminance: r * 0.2126 + g * 0.7152 + b * 0.0722,
        saturation: maximum ? (maximum - minimum) / maximum : 0
      };
    };
    const removePrintedInk = (x, y, force = false) => {
      if (force) { setPaper(x, y); return; }
      const offset = (y * width + x) * 4;
      const r = data[offset], g = data[offset + 1], b = data[offset + 2];
      const metrics = metricsAt(x, y);
      // Outer rails and marker wells contain no user content and can be
      // cleared unconditionally. In the footer boxes preserve very dark or
      // chromatic handwriting while removing yellow and neutral print residue.
      if (isYellow(r, g, b) || (r >= g * 0.9 && g >= b * 1.08 && r >= b * 1.18)
        || (metrics.saturation <= 0.19 && metrics.luminance >= 72 && metrics.luminance < 246)) {
        setPaper(x, y);
      }
    };
    const paintVerticalBand = (xCm, y1Cm, y2Cm, radiusCm, force = false) => {
      const center = Math.round(xCm * pxPerCm);
      const radius = Math.max(1, Math.ceil(radiusCm * pxPerCm));
      const y1 = Math.max(0, Math.floor(y1Cm * pxPerCm));
      const y2 = Math.min(height - 1, Math.ceil(y2Cm * pxPerCm));
      for (let y = y1; y <= y2; y += 1) {
        for (let x = Math.max(0, center - radius); x <= Math.min(width - 1, center + radius); x += 1) {
          removePrintedInk(x, y, force);
        }
      }
    };
    const paintHorizontalBand = (yCm, x1Cm, x2Cm, radiusCm, force = false) => {
      const center = Math.round(yCm * pxPerCm);
      const radius = Math.max(1, Math.ceil(radiusCm * pxPerCm));
      const x1 = Math.max(0, Math.floor(x1Cm * pxPerCm));
      const x2 = Math.min(width - 1, Math.ceil(x2Cm * pxPerCm));
      for (let y = Math.max(0, center - radius); y <= Math.min(height - 1, center + radius); y += 1) {
        for (let x = x1; x <= x2; x += 1) removePrintedInk(x, y, force);
      }
    };

    // Remove the photographed frame before the vector frame is composited.
    // The generous band also captures grey antialiasing and small residual
    // mesh errors instead of leaving a dirty halo beside the digital line.
    paintVerticalBand(1.5, 1.25, 28.6, 0.13, true);
    paintVerticalBand(19.5, 1.25, 28.6, 0.13, true);
    paintHorizontalBand(1.43, 1.35, 19.65, 0.13, true);
    paintHorizontalBand(28.43, 1.35, 19.65, 0.13, true);

    // Footer boxes and ticks are allowed to contain handwriting, so only
    // their printed yellow/neutral pixels are removed.
    for (const y of [27.43, 27.93]) {
      paintHorizontalBand(y, 1.82, 10.18, 0.18);
      paintHorizontalBand(y, 10.82, 19.18, 0.18);
    }
    for (let x = 2; x <= 10.0001; x += 0.5) paintVerticalBand(x, 27.24, 28.12, 0.13);
    for (let x = 11; x <= 19.0001; x += 0.5) paintVerticalBand(x, 27.24, 28.12, 0.13);

    // The four reference dots are always regenerated digitally.
    for (const xCm of [10.125, 10.375, 10.625, 10.875]) {
      const cx = xCm * pxPerCm;
      const cy = 27.68 * pxPerCm;
      const radius = Math.max(2, Math.ceil(0.19 * pxPerCm));
      for (let y = Math.max(0, Math.floor(cy - radius)); y <= Math.min(height - 1, Math.ceil(cy + radius)); y += 1) {
        for (let x = Math.max(0, Math.floor(cx - radius)); x <= Math.min(width - 1, Math.ceil(cx + radius)); x += 1) {
          if (Math.hypot(x - cx, y - cy) <= radius) setPaper(x, y);
        }
      }
    }

    // Recover one shared lattice phase from hundreds of repeated dots, then
    // clean only a tiny neighbourhood at those coordinates. Searching each
    // whole cell independently can lock onto a faint handwritten fragment;
    // the global periodic vote makes that impossible while still tolerating
    // camera/warp displacement of the printed grid.
    const ringRadius = Math.max(3, Math.ceil(0.095 * pxPerCm));
    // A blurred photographed bullet can be wider than the old inner probe.
    // Check beyond the printed spot, otherwise its own two sides look like
    // a pencil stroke and leave a second lattice beneath the native dots.
    const strokeContinuityRadius = Math.max(ringRadius + 1, Math.round(pxPerCm * 0.14));
    const ringDirections = [
      [1, 0], [-1, 0], [0, 1], [0, -1],
      [Math.SQRT1_2, Math.SQRT1_2], [-Math.SQRT1_2, Math.SQRT1_2],
      [Math.SQRT1_2, -Math.SQRT1_2], [-Math.SQRT1_2, -Math.SQRT1_2]
    ];
    const ringOffsets = ringDirections.map(([ux, uy]) => [
      Math.round(ux * ringRadius), Math.round(uy * ringRadius)
    ]);
    const strokeOffsets = [[1, 0], [0, 1], [Math.SQRT1_2, Math.SQRT1_2], [Math.SQRT1_2, -Math.SQRT1_2]]
      .map(([ux, uy]) => [Math.round(ux * strokeContinuityRadius), Math.round(uy * strokeContinuityRadius)]);
    const dotResponseAt = (x, y, minimumSupports = ringDirections.length) => {
      const center = metricsAt(x, y);
      if (center.saturation > 0.38 || center.luminance < 105 || center.luminance > 238) return null;
      let ringSum = 0;
      let supports = 0;
      for (const [dx, dy] of ringOffsets) {
        const ring = metricsAt(x + dx, y + dy);
        // A continuous stroke necessarily remains dark in at least one radial
        // direction. A printed circular dot is surrounded by lighter paper in
        // all eight directions.
        if (ring.luminance >= center.luminance + 2 && ring.saturation <= 0.48) supports += 1;
        ringSum += ring.luminance;
      }
      if (supports < minimumSupports) return null;
      const contrast = ringSum / ringDirections.length - center.luminance;
      return contrast >= 5 ? { contrast, luminance: center.luminance } : null;
    };
    const scorePhase = (offsetXCm, offsetYCm) => {
      let matches = 0;
      let contrast = 0;
      for (let column = 0; column < 35; column += 2) {
        const x = Math.round((2 + column * 0.5 + offsetXCm) * pxPerCm);
        for (let row = 0; row < 51; row += 2) {
          const y = Math.round((1.93 + row * 0.5 + offsetYCm) * pxPerCm);
          const response = dotResponseAt(x, y, 6);
          if (!response) continue;
          matches += 1;
          contrast += Math.min(30, response.contrast);
        }
      }
      return { matches, contrast };
    };
    const centres = new Array(35 * 51);
    const measuredCentres = new Array(35 * 51);
    const measuredColumns = new Set(), measuredRows = new Set();
    let measuredCount = 0, confirmedCount = 0, geometryMeasuredCount = 0;
    const geometric = Array.isArray(settings.dotObservations) ? settings.dotObservations : [];
    const photographed = Array.isArray(settings.photographedDotObservations) ? settings.photographedDotObservations : [];
    const observed = [...geometric, ...photographed];
    if (observed.length >= 600 && observed.length <= 6000) {
      for (let observationIndex = 0; observationIndex < observed.length; observationIndex++) {
        const point = observed[observationIndex];
        const column = point.col ?? point.column;
        const row = point.row;
        if (!Number.isInteger(column) || !Number.isInteger(row)
          || column < 0 || column >= 35 || row < 0 || row >= 51
          || !Number.isFinite(point.x) || !Number.isFinite(point.y)) continue;
        const index = column * 51 + row;
        if (measuredCentres[index]) continue;
        const nominalX = (2 + column * 0.5) * pxPerCm;
        const nominalY = (1.93 + row * 0.5) * pxPerCm;
        // Inverse-mapped observations belong to this physical lattice cell.
        // Refuse phase-swapped or stale source-image coordinates.
        if (Math.abs(point.x - nominalX) > pxPerCm * 0.24
          || Math.abs(point.y - nominalY) > pxPerCm * 0.24) continue;
        // Paper samples can legitimately land on a nearby handwritten i-dot.
        // They are not source-verified print identities: require agreement
        // with the corrected lattice before using them to remove any ink.
        if (observationIndex >= geometric.length && (Math.abs(point.x - nominalX) > pxPerCm * .045
          || Math.abs(point.y - nominalY) > pxPerCm * .045)) continue;
        // Observations use Canvas edge coordinates; this buffer uses integer
        // pixel indices. Keep the verified geometric centre instead of moving
        // it onto a darker neighbouring pixel from texture or antialiasing.
        const cx = Math.round(point.x - 0.5), cy = Math.round(point.y - 0.5);
        let best = null;
        for (let dy = -2; dy <= 2; dy += 1) {
          for (let dx = -2; dx <= 2; dx += 1) {
            const response = dotResponseAt(cx + dx, cy + dy, 6);
            if (response && (!best || response.contrast > best.contrast)) {
              best = { x: cx + dx, y: cy + dy, contrast: response.contrast };
            }
          }
        }
        // The local search confirms photographic evidence only. Moving the
        // centre here would discard the alignment established by the warp.
        if (best) confirmedCount += 1;
        measuredCentres[index] = { x: cx, y: cy, dx: cx - nominalX, dy: cy - nominalY,
          neutralPrint: point.centerRefined === true && Number.isFinite(point.dotContrast) && point.dotContrast > 0 };
        measuredCount += 1;
        if (observationIndex < geometric.length) geometryMeasuredCount++;
        measuredColumns.add(column); measuredRows.add(row);
        if (measuredCount % 128 === 0) {
          checkProcessingCancelled(settings);
          await yieldToBrowser();
        }
      }
    }
    const useObservedCentres = measuredCount >= 600
      && measuredColumns.size >= 25 && measuredRows.size >= 36
      && confirmedCount >= Math.min(180, measuredCount * 0.18);
    if (useObservedCentres) {
      // Geometry already found these actual ink dots in the source photo.
      // Their inverse-warped positions avoid searching the whole lattice
      // again, and retain local displacement that one global phase misses.
      const measured = measuredCentres.filter(Boolean);
      const globalDX = medianNumber(measured.map(point => point.dx));
      const globalDY = medianNumber(measured.map(point => point.dy));
      for (let column = 0; column < 35; column += 1) {
        checkProcessingCancelled(settings);
        if (column % 7 === 0) await yieldToBrowser();
        for (let row = 0; row < 51; row += 1) {
          const index = column * 51 + row;
          if (measuredCentres[index]) { centres[index] = measuredCentres[index]; continue; }
          let weight = 0, sumX = 0, sumY = 0;
          // A mark hidden by writing is not a new observation. Continue only
          // the smooth displacement of its observed neighbours; the shared
          // glyph subtraction below still protects uncommon ink and strokes.
          for (let dc = -4; dc <= 4; dc += 1) {
            if (column + dc < 0 || column + dc >= 35) continue;
            for (let dr = -4; dr <= 4; dr += 1) {
              if (row + dr < 0 || row + dr >= 51) continue;
              const nearby = measuredCentres[(column + dc) * 51 + row + dr];
              if (!nearby) continue;
              const amount = 1 / (0.5 + dc * dc + dr * dr);
              weight += amount; sumX += nearby.dx * amount; sumY += nearby.dy * amount;
            }
          }
          centres[index] = {
            x: Math.round((2 + column * 0.5) * pxPerCm + (weight ? sumX / weight : globalDX)),
            y: Math.round((1.93 + row * 0.5) * pxPerCm + (weight ? sumY / weight : globalDY))
          };
        }
      }
    } else {
      const nominalPhase = scorePhase(0, 0);
      let bestPhase = { x: 0, y: 0, ...nominalPhase };
      for (let yStep = -10; yStep <= 10; yStep += 1) {
        checkProcessingCancelled(settings);
        await yieldToBrowser();
        for (let xStep = -10; xStep <= 10; xStep += 1) {
          const x = xStep * 0.02, y = yStep * 0.02;
          const score = scorePhase(x, y);
          if (score.matches > bestPhase.matches
            || (score.matches === bestPhase.matches && score.contrast > bestPhase.contrast)) {
            bestPhase = { x, y, ...score };
          }
        }
      }
      // Sparse synthetic/mostly-written pages do not contain enough template
      // evidence to justify moving the lattice away from its canonical phase.
      const gridPhase = bestPhase.matches >= nominalPhase.matches + 5
        ? bestPhase
        : { x: 0, y: 0 };
      // Residual perspective and real paper curl move the printed lattice by a
      // few pixels across the page. One global phase smears the learned glyph
      // over many locations and leaves most photographed dots untouched. Track
      // a phase only where a whole block supplies repeated isolated marks.
      for (let blockColumn = 0; blockColumn < 35; blockColumn += 7) {
        for (let blockRow = 0; blockRow < 51; blockRow += 9) {
          checkProcessingCancelled(settings);
          await yieldToBrowser();
          const endColumn = Math.min(35, blockColumn + 7);
          const endRow = Math.min(51, blockRow + 9);
          let best = { x: gridPhase.x, y: gridPhase.y, matches: 0, contrast: 0 };
          for (let yStep = -5; yStep <= 5; yStep += 1) {
            for (let xStep = -5; xStep <= 5; xStep += 1) {
              const xPhase = gridPhase.x + xStep * 0.04;
              const yPhase = gridPhase.y + yStep * 0.04;
              let matches = 0, contrast = 0;
              for (let column = blockColumn; column < endColumn; column += 1) {
                for (let row = blockRow; row < endRow; row += 1) {
                  const response = dotResponseAt(
                    Math.round((2 + column * 0.5 + xPhase) * pxPerCm),
                    Math.round((1.93 + row * 0.5 + yPhase) * pxPerCm), 6);
                  if (response) { matches += 1; contrast += Math.min(30, response.contrast); }
                }
              }
              if (matches > best.matches || matches === best.matches && contrast > best.contrast) {
                best = { x: xPhase, y: yPhase, matches, contrast };
              }
            }
          }
          const reliable = best.matches >= Math.max(8, (endColumn - blockColumn) * (endRow - blockRow) * 0.22);
          const phase = reliable ? best : gridPhase;
          for (let column = blockColumn; column < endColumn; column += 1) {
            for (let row = blockRow; row < endRow; row += 1) {
              let cx = Math.round((2 + column * 0.5 + phase.x) * pxPerCm);
              let cy = Math.round((1.93 + row * 0.5 + phase.y) * pxPerCm);
              if (reliable) {
                let localBest = null;
                const search = Math.max(2, Math.round(pxPerCm * 0.04));
                for (let dy = -search; dy <= search; dy += 1) {
                  for (let dx = -search; dx <= search; dx += 1) {
                    const response = dotResponseAt(cx + dx, cy + dy, 6);
                    if (response && (!localBest || response.contrast > localBest.contrast)) {
                      localBest = { x: cx + dx, y: cy + dy, contrast: response.contrast };
                    }
                  }
                }
                if (localBest) { cx = localBest.x; cy = localBest.y; }
              }
              centres[column * 51 + row] = { x: cx, y: cy };
            }
          }
        }
      }
    }
    // Learn the photographed dot glyph from the median of all 1,785 cells.
    // Handwriting is an outlier at any one relative pixel, while the printed
    // grid is repeated everywhere. Subtracting this learned darkness preserves
    // ink that crosses a dot instead of deleting the combined pixel.
    const glyphRadius = Math.max(4, Math.ceil(0.22 * pxPerCm));
    const glyphSide = glyphRadius * 2 + 1;
    const glyphPixels = glyphSide * glyphSide;
    // The same disk is sampled in all 1,785 cells. Build its integer support
    // once rather than recomputing a square root for every photographed dot.
    const glyphOffsets = [];
    for (let dy = -glyphRadius; dy <= glyphRadius; dy += 1) {
      for (let dx = -glyphRadius; dx <= glyphRadius; dx += 1) {
        if (Math.hypot(dx, dy) <= glyphRadius) glyphOffsets.push({
          dx, dy, index: (dy + glyphRadius) * glyphSide + dx + glyphRadius
        });
      }
    }
    const glyphHistograms = new Uint16Array(glyphPixels * 256);
    const glyphCounts = new Uint16Array(glyphPixels);
    for (let column = 0; column < 35; column += 1) {
      checkProcessingCancelled(settings);
      await yieldToBrowser();
      for (let row = 0; row < 51; row += 1) {
        const { x: cx, y: cy } = centres[column * 51 + row];
        for (const { dx, dy, index: glyphIndex } of glyphOffsets) {
          const metrics = metricsAt(cx + dx, cy + dy);
          if (metrics.saturation > 0.48) continue;
          const luminance = clamp(Math.round(metrics.luminance), 0, 255);
          glyphHistograms[glyphIndex * 256 + luminance] += 1;
          glyphCounts[glyphIndex] += 1;
        }
      }
    }
    const glyphDarkness = new Uint8Array(glyphPixels);
    let repeatedPeak = 0;
    for (let glyphIndex = 0; glyphIndex < glyphPixels; glyphIndex += 1) {
      const count = glyphCounts[glyphIndex];
      if (!count) continue;
      // Residual mesh curvature moves a photographed dot by a few pixels
      // between cells. A lower robust quantile captures that broadened glyph;
      // handwriting still occupies far fewer than 30% of the same relative
      // position across the full page.
      const target = Math.ceil(count * 0.12);
      let seen = 0;
      let median = targetPaper;
      const histogramOffset = glyphIndex * 256;
      for (let luminance = 0; luminance < 256; luminance += 1) {
        seen += glyphHistograms[histogramOffset + luminance];
        if (seen >= target) {
          median = luminance;
          break;
        }
      }
      // Be conservative: this is a printed dot under handwriting, not a
      // segmentation mask. A lower robust quantile avoids learning ink that
      // happens to recur at similar positions, and the capped correction
      // preserves the darker antialiased edge of a real pen stroke.
      repeatedPeak = Math.max(repeatedPeak, targetPaper - median);
      glyphDarkness[glyphIndex] = Math.max(0, Math.min(112, targetPaper - median));
    }
    const componentSide = glyphSide;
    const componentSeen = new Uint8Array(glyphPixels);
    const componentQueue = new Int32Array(glyphPixels);
    const searchRadius = Math.min(glyphRadius - 2, Math.max(2, Math.ceil(pxPerCm * .12)));
    let isolatedRemoved = 0, isolatedRejected = 0, protectedComponents = 0;
    let protectComponent = false, protectedHalo = 0;
    const removeIsolatedRepeatedGlyph = (cx, cy, neutralPrint) => {
      protectComponent = false; protectedHalo = 0;
      if (repeatedPeak < 5) return false;
      const darknessAt = (dx, dy) => {
        const offset = ((cy + dy) * width + cx + dx) * 4;
        const r = data[offset], g = data[offset + 1], b = data[offset + 2];
        const darkness = targetPaper - (r * .2126 + g * .7152 + b * .0722);
        const chroma = Math.max(r, g, b) - Math.min(r, g, b);
        // Full-resolution source classification already verified this glyph
        // as neutral print. Colour correction can amplify a local camera
        // cast; keep that provenance instead of mistaking it for blue ink.
        // Unverified sites retain the strict neutral test.
        return chroma <= Math.max(5, darkness * .3)
          || neutralPrint && chroma <= Math.max(r, g, b) * .5 ? darkness : 0;
      };
      let peak = 0, peakX = 0, peakY = 0;
      for (let dy = -searchRadius; dy <= searchRadius; dy++) for (let dx = -searchRadius; dx <= searchRadius; dx++) {
        if (dx * dx + dy * dy > searchRadius * searchRadius) continue;
        const value = darknessAt(dx, dy);
        if (value > peak) { peak = value; peakX = dx; peakY = dy; }
      }
      // Punctuation and ink over a bullet are not the repeated printed tone.
      // Do not turn a darker isolated handwritten mark into background.
      if (peak < 5) return false;
      const uncommonTone = peak > Math.max(18, repeatedPeak * 1.4);
      const threshold = Math.max(3, peak * .12);
      const start = (peakY + glyphRadius) * componentSide + peakX + glyphRadius;
      componentSeen.fill(0); componentSeen[start] = 2; componentQueue[0] = start;
      let head = 0, tail = 1, mass = 0, sumX = 0, sumY = 0, sumXX = 0, sumYY = 0, sumXY = 0;
      let minX = peakX, maxX = peakX, minY = peakY, maxY = peakY, touchesEdge = false;
      while (head < tail) {
        const index = componentQueue[head++], dx = index % componentSide - glyphRadius,
          dy = Math.floor(index / componentSide) - glyphRadius, weight = Math.max(0, darknessAt(dx, dy) - threshold);
        mass += weight; sumX += dx * weight; sumY += dy * weight;
        sumXX += dx * dx * weight; sumYY += dy * dy * weight; sumXY += dx * dy * weight;
        minX = Math.min(minX, dx); maxX = Math.max(maxX, dx); minY = Math.min(minY, dy); maxY = Math.max(maxY, dy);
        if (Math.abs(dx) >= glyphRadius - 1 || Math.abs(dy) >= glyphRadius - 1) { touchesEdge = true; continue; }
        for (let oy = -1; oy <= 1; oy++) for (let ox = -1; ox <= 1; ox++) {
          const next = index + oy * componentSide + ox;
          if (componentSeen[next]) continue;
          componentSeen[next] = 1;
          if (darknessAt(dx + ox, dy + oy) > threshold) {
            componentSeen[next] = 2; componentQueue[tail++] = next;
          }
        }
      }
      if (!mass || tail < 2) { isolatedRejected++; return false; }
      if (touchesEdge) {
        protectComponent = true; protectedHalo = 1; protectedComponents++; isolatedRejected++; return false;
      }
      const mx = sumX / mass, my = sumY / mass, vx = sumXX / mass - mx * mx,
        vy = sumYY / mass - my * my, covariance = sumXY / mass - mx * my,
        separation = Math.hypot(vx - vy, 2 * covariance),
        narrow = Math.max(.1, (vx + vy - separation) / 2), wide = (vx + vy + separation) / 2;
      // A compact printed spot ends inside its cell. A faint pencil line,
      // loop or shaded drawing extends to the boundary or is elongated.
      const extent = Math.max(maxX - minX, maxY - minY);
      let occupiesCenter = false;
      for (let dy = -1; dy <= 1 && !occupiesCenter; dy++) for (let dx = -1; dx <= 1; dx++) {
        if (componentSeen[(dy + glyphRadius) * componentSide + dx + glyphRadius] === 2) {
          occupiesCenter = true; break;
        }
      }
      const offCenter = !occupiesCenter || Math.hypot(mx, my) > Math.max(1, pxPerCm * .035);
      if (wide / narrow > 3.2 || extent > glyphRadius * 1.35 || offCenter) {
        // Short asymmetric speckles attached to a repeated glyph can still
        // use the conservative subtraction. A displaced isolated mark or a
        // longer stroke is kept, including its antialiased fringe.
        protectComponent = offCenter || extent > pxPerCm * .12;
        protectedHalo = extent > pxPerCm * .12 ? 1 : 0;
        if (protectComponent) protectedComponents++;
        isolatedRejected++; return false;
      }
      if (uncommonTone) {
        // A darker punctuation core or camera speckle may touch the lighter
        // printed dot. Preserve the uncommon pixels, not the entire shared
        // component; otherwise one speckle protects a second printed glyph.
        const inkThreshold = Math.max(repeatedPeak * 1.2, peak * .6);
        for (let i = 0; i < tail; i++) {
          const index = componentQueue[i], dx = index % componentSide - glyphRadius,
            dy = Math.floor(index / componentSide) - glyphRadius;
          if (darknessAt(dx, dy) < inkThreshold) componentSeen[index] = 1;
        }
        protectComponent = true; protectedComponents++; isolatedRejected++; return false;
      }
      const halo = Math.max(1.5, pxPerCm * .025);
      const rx = (maxX - minX) / 2 + halo, ry = (maxY - minY) / 2 + halo;
      const centerX = (minX + maxX) / 2, centerY = (minY + maxY) / 2;
      for (let dy = Math.floor(centerY - ry); dy <= Math.ceil(centerY + ry); dy++) {
        for (let dx = Math.floor(centerX - rx); dx <= Math.ceil(centerX + rx); dx++) {
          if (((dx - centerX) / rx) ** 2 + ((dy - centerY) / ry) ** 2 > 1) continue;
          const darkness = darknessAt(dx, dy);
          const componentIndex = (dy + glyphRadius) * componentSide + dx + glyphRadius;
          const belongs = Math.abs(dx) <= glyphRadius && Math.abs(dy) <= glyphRadius
            && componentSeen[componentIndex] === 2;
          if (darkness > 0 && darkness <= peak + 1 && (belongs || darkness <= threshold)) setPaper(cx + dx, cy + dy);
        }
      }
      isolatedRemoved++;
      return true;
    };
    for (let column = 0; column < 35; column += 1) {
      checkProcessingCancelled(settings);
      await yieldToBrowser();
      for (let row = 0; row < 51; row += 1) {
        const { x: cx, y: cy, neutralPrint } = centres[column * 51 + row];
        if (removeIsolatedRepeatedGlyph(cx, cy, neutralPrint)) continue;
        for (const { dx, dy, index: glyphIndex } of glyphOffsets) {
          if (protectComponent) {
            let belongsToInk = false;
            for (let oy = -protectedHalo; oy <= protectedHalo && !belongsToInk; oy++) for (let ox = -protectedHalo; ox <= protectedHalo; ox++) {
              if (Math.abs(dx + ox) > glyphRadius || Math.abs(dy + oy) > glyphRadius) continue;
              if (componentSeen[(dy + oy + glyphRadius) * componentSide + dx + ox + glyphRadius] === 2) {
                belongsToInk = true; break;
              }
            }
            if (belongsToInk) continue;
          }
          const lift = glyphDarkness[glyphIndex];
          if (lift < 4) continue;
          const x = cx + dx, y = cy + dy;
          const metrics = metricsAt(x, y);
          // Only consider neutral pixels; dark strokes are protected by the
          // continuity test below, even when their antialiased pixels are
          // as light as the printed dot itself.
          if (metrics.saturation > 0.42 || metrics.luminance < 70) continue;
          // A dot is radially isolated; a pen stroke continues through it.
          // Keep any pixel with dark neighbours on both sides of an axis,
          // including pale/anti-aliased strokes that a simple threshold
          // cannot tell apart from the printed grid.
          let continuesAsStroke = false;
          for (const [sx, sy] of strokeOffsets) {
            const before = metricsAt(x - sx, y - sy);
            const after = metricsAt(x + sx, y + sy);
            if (before.luminance < 245 && after.luminance < 245
              && before.luminance <= metrics.luminance + 28
              && after.luminance <= metrics.luminance + 28) {
              continuesAsStroke = true;
              break;
            }
          }
          if (continuesAsStroke) continue;
          const offset = (y * width + x) * 4;
          for (let channel = 0; channel < 3; channel += 1) {
            data[offset + channel] = Math.min(targetPaper, data[offset + channel] + lift);
          }
        }
      }
    }
    // Compact removal is limited by repeated print tone, shape, connectivity
    // and source-neutral provenance. Darker punctuation, chromatic ink and
    // continuous strokes keep the conservative learned-glyph subtraction;
    // nearby disconnected marks are never included in a dot's halo mask.
    return {
      method: useObservedCentres && geometryMeasuredCount >= 600 ? "observed-dots" : "recovered-dot-phase",
      observations: geometryMeasuredCount,
      photographedObservations: measuredCount - geometryMeasuredCount,
      confirmed: confirmedCount,
      isolatedRemoved,
      isolatedRejected,
      protectedComponents
    };
  }

  function smoothPaperIllumination(values, width, height, options = {}) {
    // A local closing estimates the paper behind thin ink strokes. A simple
    // blur follows dark writing and then mistakes it for a shadow, whitening
    // pencil away. Work on the small illumination map, never on the scan.
    let field = values;
    const radius = options.radius ?? Math.max(2, Math.round(width / 16));
    const extremum = (horizontal, maximum) => {
      const next = new Float32Array(field.length);
      const length = horizontal ? width : height;
      const lines = horizontal ? height : width;
      const queue = new Int32Array(length + radius * 2);
      const indexAt = (line, position) => horizontal
        ? line * width + clamp(position, 0, width - 1)
        : clamp(position, 0, height - 1) * width + line;
      for (let line = 0; line < lines; line += 1) {
        let head = 0, tail = 0;
        for (let position = -radius; position < length + radius; position += 1) {
          while (head < tail && queue[head] < position - radius * 2) head += 1;
          const value = field[indexAt(line, position)];
          while (head < tail && (maximum
            ? field[indexAt(line, queue[tail - 1])] <= value
            : field[indexAt(line, queue[tail - 1])] >= value)) tail -= 1;
          queue[tail++] = position;
          if (position >= radius) next[indexAt(line, position - radius)] = field[indexAt(line, queue[head])];
        }
      }
      field = next;
    };
    // Closing fills ink-sized dark holes but preserves a smooth monotonic
    // shadow. A dilation alone brightens its estimate and leaves grey bands;
    // the erosion restores the actual local paper exposure afterwards.
    extremum(true, true);
    extremum(false, true);
    extremum(true, false);
    extremum(false, false);
    const blurRadius = options.blurRadius ?? 3;
    if (!blurRadius) return field;
    for (const horizontal of [true, false]) {
      const next = new Float32Array(field.length);
      for (let y = 0; y < height; y += 1) for (let x = 0; x < width; x += 1) {
        let sum = 0;
        for (let delta = -blurRadius; delta <= blurRadius; delta += 1) {
          const sx = horizontal ? clamp(x + delta, 0, width - 1) : x;
          const sy = horizontal ? y : clamp(y + delta, 0, height - 1);
          sum += field[sy * width + sx];
        }
        next[y * width + x] = sum / (2 * blurRadius + 1);
      }
      field = next;
    }
    return field;
  }

  function observedPaperIllumination(canvas, data, observations) {
    const anchors = new Float32Array(35 * 51);
    const paperPoints = [];
    const paperNoise = [];
    let count = 0, supplied = 0, recovered = 0;
    const pxPerMM = canvas.width / 210;
    const offsets = [];
    for (const radius of [1, 1.8]) for (let sample = 0; sample < 24; sample += 1) {
      const angle = sample / 24 * Math.PI * 2;
      offsets.push([Math.cos(angle) * radius * pxPerMM, Math.sin(angle) * radius * pxPerMM]);
    }
    const measurePaper = point => {
      const samples = [];
      const positions = count % 4 === 0 ? [] : null;
      for (const sampleOffset of offsets) {
        const [dx, dy] = sampleOffset;
        const x = Math.round(point.x - 0.5 + dx);
        const y = Math.round(point.y - 0.5 + dy);
        if (x < 0 || y < 0 || x >= canvas.width || y >= canvas.height) continue;
        const offset = (y * canvas.width + x) * 4;
        const r = data[offset], g = data[offset + 1], b = data[offset + 2];
        const maximum = Math.max(r, g, b), minimum = Math.min(r, g, b);
        if (maximum - minimum > maximum * 0.24) continue;
        samples.push(r * 0.2126 + g * 0.7152 + b * 0.0722);
        positions?.push(sampleOffset);
      }
      if (samples.length < 16) return 0;
      if (samples.length >= 40 && positions) {
        // Remove the local lighting slope before measuring camera grain.
        // A median across these paper rings ignores occasional ink crossings
        // and shadow boundaries; their intensity range is not a noise floor.
        let sum = 0, sx = 0, sy = 0, sxx = 0, syy = 0, sxy = 0, sxv = 0, syv = 0;
        for (let i = 0; i < samples.length; i += 1) {
          const [dx, dy] = positions[i], value = samples[i];
          sum += value; sx += dx; sy += dy; sxx += dx * dx; syy += dy * dy;
          sxy += dx * dy; sxv += dx * value; syv += dy * value;
        }
        const n = samples.length, mx = sx / n, my = sy / n, mean = sum / n;
        const xx = sxx - sx * mx, yy = syy - sy * my, xy = sxy - sx * my;
        const xv = sxv - sx * mean, yv = syv - sy * mean;
        const determinant = xx * yy - xy * xy;
        const gx = determinant > 1e-6 ? (xv * yy - yv * xy) / determinant : 0;
        const gy = determinant > 1e-6 ? (yv * xx - xv * xy) / determinant : 0;
        const residuals = samples.map((value, i) => value - mean
          - gx * (positions[i][0] - mx) - gy * (positions[i][1] - my));
        const sorted = residuals.slice().sort((a, b) => a - b);
        const median = sorted[Math.floor(sorted.length / 2)];
        const deviations = residuals.map(value => Math.abs(value - median)).sort((a, b) => a - b);
        paperNoise.push(deviations[Math.floor(deviations.length / 2)] * 1.4826);
      }
      samples.sort((a, b) => a - b);
      return samples[Math.floor(samples.length * 0.8)];
    };
    for (const point of Array.isArray(observations) ? observations : []) {
      const column = point.col ?? point.column, row = point.row;
      if (!Number.isInteger(column) || !Number.isInteger(row)
        || column < 0 || column >= 35 || row < 0 || row >= 51
        || !Number.isFinite(point.x + point.y)) continue;
      const index = column * 51 + row;
      if (anchors[index]) continue;
      const paper = measurePaper(point);
      if (!paper) continue;
      anchors[index] = paper;
      paperPoints.push({ x: point.x, y: point.y, col: column, row });
      count += 1; supplied += 1;
    }
    // A dot rejected by the surface fit is not proof that the paper is blank
    // or painted over. Recover exposure evidence from the rectified raster,
    // including when the geometric fit has fewer than 600 accepted points.
    // These observations never move the surface or become geometry anchors.
    const lumaAt = (x, y) => {
      const offset = (y * canvas.width + x) * 4;
      return data[offset] * 0.2126 + data[offset + 1] * 0.7152 + data[offset + 2] * 0.0722;
    };
    const ring = [];
    for (let i = 0; i < 8; i += 1) {
      const angle = i * Math.PI / 4;
      ring.push([Math.round(Math.cos(angle) * pxPerMM), Math.round(Math.sin(angle) * pxPerMM)]);
    }
    const reach = Math.max(2, Math.round(pxPerMM * 0.7));
    const step = Math.max(1, Math.round(pxPerMM * 0.22));
    const core = Math.max(1, Math.round(pxPerMM * 0.12));
    const safe = reach + Math.ceil(pxPerMM) + core + 1;
    for (let column = 0; column < 35; column += 1) for (let row = 0; row < 51; row += 1) {
      const index = column * 51 + row;
      if (anchors[index]) continue;
      const cx = Math.round((20 + column * 5) * pxPerMM - 0.5);
      const cy = Math.round((19.3 + row * 5) * pxPerMM - 0.5);
      if (cx < safe || cy < safe || cx >= canvas.width - safe || cy >= canvas.height - safe) continue;
      let best = null;
      for (let dy = -reach; dy <= reach; dy += step) for (let dx = -reach; dx <= reach; dx += step) {
        const x = cx + dx, y = cy + dy;
        const center = (lumaAt(x, y) * 2 + lumaAt(x - core, y) + lumaAt(x + core, y)
          + lumaAt(x, y - core) + lumaAt(x, y + core)) / 6;
        let minimum = 255, sum = 0;
        for (const [rx, ry] of ring) {
          const value = lumaAt(x + rx, y + ry);
          minimum = Math.min(minimum, value); sum += value;
        }
        const paper = sum / ring.length;
        const contrast = paper - center;
        // Continuous handwriting intersects the ring; an opaque grey drawing
        // has no repeatable central bullet signal. Do not use either as paper.
        if (contrast < Math.max(3, paper * 0.012) || minimum - center < contrast * 0.32
          || center < paper * 0.32) continue;
        const score = contrast - Math.hypot(dx, dy) / Math.max(1, reach) * 0.5;
        if (!best || score > best.score) best = { x: x + 0.5, y: y + 0.5, score };
      }
      if (!best) continue;
      const paper = measurePaper(best);
      if (!paper) continue;
      anchors[index] = paper;
      paperPoints.push({ x: best.x, y: best.y, col: column, row });
      count += 1; recovered += 1;
    }
    // Sparse evidence still improves a photographed template. Generic pages
    // with no repeatable bullet signal retain the conservative local field.
    if (count < 80) return null;
    // Visible printed dots certify actual paper under the illumination. An
    // opaque drawing hides those dots; continue neighbouring paper across the
    // resulting gap instead of mistaking the drawing's grey fill for a lamp.
    const filled = new Float32Array(anchors);
    for (let column = 0; column < 35; column += 1) for (let row = 0; row < 51; row += 1) {
      const index = column * 51 + row;
      if (filled[index]) continue;
      let weighted = 0, weight = 0;
      for (let dc = -5; dc <= 5; dc += 1) for (let dr = -5; dr <= 5; dr += 1) {
        const c = column + dc, r = row + dr;
        if (c < 0 || c >= 35 || r < 0 || r >= 51 || !anchors[c * 51 + r]) continue;
        const amount = 1 / (0.5 + dc * dc + dr * dr);
        weighted += anchors[c * 51 + r] * amount; weight += amount;
      }
      if (!weight) {
        // Detection can miss an entire blurred corner. Never substitute an
        // arbitrary bright paper value there: that invents a rectangular grey
        // patch on a dark or blue-lit sheet. Continue the nearest measured
        // exposure, with distant anchors receiving very little influence.
        for (let c = 0; c < 35; c += 1) for (let r = 0; r < 51; r += 1) {
          const sample = anchors[c * 51 + r];
          if (!sample) continue;
          const distanceSquared = (c - column) ** 2 + (r - row) ** 2;
          const amount = 1 / (distanceSquared * distanceSquared);
          weighted += sample * amount; weight += amount;
        }
      }
      filled[index] = weighted / weight;
    }
    const minimums = new Float32Array(anchors.length);
    for (let column = 0; column < 35; column += 1) for (let row = 0; row < 51; row += 1) {
      let minimum = Infinity;
      for (let dc = -1; dc <= 1; dc += 1) for (let dr = -1; dr <= 1; dr += 1) {
        const c = clamp(column + dc, 0, 34), r = clamp(row + dr, 0, 50);
        if (anchors[c * 51 + r]) minimum = Math.min(minimum, anchors[c * 51 + r]);
      }
      minimums[column * 51 + row] = minimum;
    }
    const width = Math.min(512, canvas.width);
    const height = Math.round(canvas.height * width / canvas.width);
    const reduced = document.createElement("canvas"); reduced.width = width; reduced.height = height;
    const context = reduced.getContext("2d", { willReadFrequently: true });
    context.imageSmoothingQuality = "high"; context.drawImage(canvas, 0, 0, width, height);
    const rgba = context.getImageData(0, 0, width, height).data;
    const values = new Float32Array(width * height);
    const left = Math.ceil(1.78 / 21 * width), right = Math.floor(19.22 / 21 * width);
    const top = Math.ceil(1.71 / 29.7 * height), bottom = Math.floor(28.15 / 29.7 * height);
    for (let y = 0; y < height; y += 1) for (let x = 0; x < width; x += 1) {
      const offset = (clamp(y, top, bottom) * width + clamp(x, left, right)) * 4;
      values[y * width + x] = rgba[offset] * 0.2126 + rgba[offset + 1] * 0.7152 + rgba[offset + 2] * 0.0722;
    }
    const fine = smoothPaperIllumination(values, width, height, { radius: 2, blurRadius: 0 });
    const field = new Float32Array(fine.length);
    const drawing = new Uint8Array(fine.length);
    for (let y = 0; y < height; y += 1) for (let x = 0; x < width; x += 1) {
      const column = clamp(((x + 0.5) / width * 210 - 20) / 5, 0, 34);
      const row = clamp(((y + 0.5) / height * 297 - 19.3) / 5, 0, 50);
      const c0 = Math.floor(column), c1 = Math.min(34, c0 + 1);
      const r0 = Math.floor(row), r1 = Math.min(50, r0 + 1);
      const cx = column - c0, ry = row - r0;
      const paper = (filled[c0 * 51 + r0] * (1 - cx) + filled[c1 * 51 + r0] * cx) * (1 - ry)
        + (filled[c0 * 51 + r1] * (1 - cx) + filled[c1 * 51 + r1] * cx) * ry;
      let minimum = minimums[Math.round(column) * 51 + Math.round(row)];
      if (!Number.isFinite(minimum)) minimum = paper;
      const local = fine[y * width + x];
      const protectDrawing = clamp((minimum - local - 6) / 10, 0, 1);
      field[y * width + x] = local * (1 - protectDrawing) + paper * protectDrawing;
      drawing[y * width + x] = Math.round(protectDrawing * 255);
    }
    reduced.width = reduced.height = 0;
    paperNoise.sort((a, b) => a - b);
    const noiseSigma = paperNoise.length >= 32 ? paperNoise[Math.floor(paperNoise.length / 2)] : 0;
    return { values: field, drawing, width, height, anchors: count, supplied, recovered, paperPoints,
      noiseSigma, noiseFloor: clamp(noiseSigma * 2.2, 2, 6) };
  }

  Light.correctColors = async function correctColors(canvas, options = {}) {
    const settings = typeof options === "boolean"
      ? { useStencil: options, preciseStencil: options }
      : options;
    checkProcessingCancelled(settings);
    const useStencil = !!settings.useStencil;
    const preciseStencil = !!settings.preciseStencil;
    const context = canvas.getContext("2d", { willReadFrequently: true });
    const image = context.getImageData(0, 0, canvas.width, canvas.height);
    const original = image.data;
    // Classify paper and ink in a common white balance before whitening.
    // Otherwise blue-lit paper is treated as coloured writing and the whole
    // page retains its cast. The shared field keeps local luminance/shadows
    // and applies the same bounded channel correction to pencil and ink.
    const balanceOptions = {
      signal: settings.signal,
      // The warp's white surround is generated, not measured paper. Estimate
      // its illuminant from the photographed interior, as the paper profile
      // does, so synthetic margins cannot veto a real camera colour cast.
      region: { x0: .12, x1: .88, y0: .08, y1: .9 }
    };
    const balanced = !processingWorker && SP.LiveDetection?.normalizeIlluminationAsync
      ? await SP.LiveDetection.normalizeIlluminationAsync(image, balanceOptions)
      : SP.LiveDetection?.normalizeIllumination?.(image, balanceOptions);
    const data = balanced?.data || original;
    const observedIllumination = preciseStencil
      ? observedPaperIllumination(canvas, data, settings.dotObservations) : null;
    // Without enough visible dots, a small closing still estimates smooth
    // paper behind thin ink. Reliable observed paper uses the finer field
    // directly; avoid computing and discarding this fallback for every scan.
    const illuminationCanvas = document.createElement("canvas");
    let illumination = observedIllumination?.values;
    if (!observedIllumination) {
      illuminationCanvas.width = Math.min(192, canvas.width);
      illuminationCanvas.height = Math.max(1, Math.round(
        canvas.height * illuminationCanvas.width / Math.max(1, canvas.width)
      ));
      const illuminationContext = illuminationCanvas.getContext("2d", { willReadFrequently: true });
      illuminationContext.imageSmoothingEnabled = true;
      illuminationContext.imageSmoothingQuality = "high";
      illuminationContext.drawImage(canvas, 0, 0, illuminationCanvas.width, illuminationCanvas.height);
      // White balance preserves local luminance. Continue real interior paper
      // into the synthetic white surround, which did not see the room's light.
      const illuminationPixels = illuminationContext.getImageData(
        0, 0, illuminationCanvas.width, illuminationCanvas.height).data;
      illumination = new Float32Array(illuminationCanvas.width * illuminationCanvas.height);
      const fieldLeft = Math.ceil(1.78 / 21 * illuminationCanvas.width);
      const fieldRight = Math.floor(19.22 / 21 * illuminationCanvas.width);
      const fieldTop = Math.ceil(1.71 / 29.7 * illuminationCanvas.height);
      const fieldBottom = Math.floor(28.15 / 29.7 * illuminationCanvas.height);
      for (let index = 0; index < illumination.length; index += 1) {
        const x = index % illuminationCanvas.width;
        const y = Math.floor(index / illuminationCanvas.width);
        const sx = preciseStencil ? clamp(x, fieldLeft, fieldRight) : x;
        const sy = preciseStencil ? clamp(y, fieldTop, fieldBottom) : y;
        const offset = (sy * illuminationCanvas.width + sx) * 4;
        illumination[index] = illuminationPixels[offset] * 0.2126
          + illuminationPixels[offset + 1] * 0.7152
          + illuminationPixels[offset + 2] * 0.0722;
      }
      illumination = smoothPaperIllumination(illumination, illuminationCanvas.width, illuminationCanvas.height);
    }
    const illuminationWidth = observedIllumination?.width || illuminationCanvas.width;
    const illuminationHeight = observedIllumination?.height || illuminationCanvas.height;
    const illuminationXScale = (illuminationWidth - 1) / Math.max(1, canvas.width - 1);
    const illuminationYScale = (illuminationHeight - 1) / Math.max(1, canvas.height - 1);
    const pxPerCm = canvas.width / 21;
    // Stay inside each 1.25 mm printed disc. The former 2.8 mm search
    // reached adjacent markers, letting a misplaced/missing swatch borrow its
    // neighbour's colour instead of measuring the known physical position.
    const radius = Math.max(2, pxPerCm * 0.115);
    const config = SP.Config || {};
    const targets = config.CALIBRATION_TARGETS || {};
    const profile = estimatePaperProfile(data, canvas.width, canvas.height, preciseStencil);
    const localPaperAt = (x, y) => {
      const channels = [[], [], []];
      for (let dy = -3; dy <= 3; dy++) for (let dx = -4; dx <= 4; dx++) {
        const sx = clamp(Math.round((x + dx * .045) * pxPerCm), 0, canvas.width - 1);
        const sy = clamp(Math.round((y + dy * .045) * pxPerCm), 0, canvas.height - 1);
        const offset = (sy * canvas.width + sx) * 4;
        const rgb = [data[offset], data[offset + 1], data[offset + 2]];
        // These are verified blank template positions. Requiring bright,
        // already neutral RGB would reject the very casts being calibrated.
        if (Math.min(...rgb) < 12 || Math.max(...rgb) < 30) continue;
        rgb.forEach((value, channel) => channels[channel].push(value));
      }
      return channels.map((values, channel) => {
        values.sort((a,b) => a-b);
        return values.length >= 16 ? values[Math.floor(values.length * .8)] : profile.sourcePaper[channel];
      });
    };
    const relativeToPaper = (source, paper) => source?.map((value, channel) =>
      clamp(value * 255 / Math.max(20, paper[channel]), 0, 1));
    const markerDefinitions = [
      [10.125, "red", [1, 0, 0]],
      [10.375, "black", [0, 0, 0]],
      [10.625, "blue", [0, 0, 1]],
      [10.875, "green", [110 / 255, 1, 18 / 255]]
    ];
    const samples = preciseStencil ? markerDefinitions.map(([x, name, fallback]) => {
      const paper = localPaperAt(x, 28.15);
      const source = sampleCalibrationMarker(data, canvas.width, canvas.height,
        x * pxPerCm, 27.68 * pxPerCm, radius, name, paper);
      return { name, source, normalizedSource: relativeToPaper(source, paper),
        target: (targets[name] || fallback.map(value => value * 255)).map(value => value / 255),
        weight: name === "black" ? 1.4 : 1 };
    }) : [];
    if (preciseStencil) {
      // Normalize every measured rail by its own adjacent paper before
      // combining them. A shadow on one side must not become a global colour
      // response, and a washed-out/missing rail must not masquerade as yellow.
      const yellowMeasurements = [
        [1.5, 6, 1.85, 6], [1.5, 14.8, 1.85, 14.8], [1.5, 23, 1.85, 23],
        [19.5, 6, 19.15, 6], [19.5, 14.8, 19.15, 14.8], [19.5, 23, 19.15, 23],
        [6, 1.43, 6, 1.85], [10.5, 1.43, 10.5, 1.85], [15, 1.43, 15, 1.85]
      ].map(([x, y, paperX, paperY]) => {
        const paper = localPaperAt(paperX, paperY);
        const source = sampleCalibrationMarker(data, canvas.width, canvas.height,
          x * pxPerCm, y * pxPerCm, pxPerCm * .19, "yellow", paper);
        return source && { source, normalizedSource: relativeToPaper(source, paper) };
      }).filter(Boolean);
      samples.push({ name: "yellow",
        source: yellowMeasurements.length >= 3 ? medianColorSamples(yellowMeasurements.map(sample => sample.source)) : null,
        normalizedSource: yellowMeasurements.length >= 3
          ? medianColorSamples(yellowMeasurements.map(sample => sample.normalizedSource)) : null,
        target: [240 / 255, 219 / 255, 76 / 255], weight: .75 });
    }
    const validMarkerSamples = samples.filter(sample => sample.name !== "yellow"
      && markerLooksValid(sample.name, sample.normalizedSource));
    const yellowSample = samples.find(sample => sample.name === "yellow");
    // All four independently located pigments are required for a fitted
    // camera response. Yellow improves the fit when measurable, but losing
    // that faint rail no longer discards four strong colour references.
    const canCalibrate = preciseStencil && validMarkerSamples.length === markerDefinitions.length;
    const normalizedReferences = validMarkerSamples.map(sample => ({
      ...sample, source: sample.normalizedSource, weight: 2
    }));
    normalizedReferences.push({ name: 'paper', source: [1,1,1], target: [1,1,1], weight: 8 });
    const markerMatrix = canCalibrate ? buildColorMatrix(normalizedReferences) : null;
    const yellowError = markerMatrix && yellowSample?.normalizedSource
      ? Math.sqrt(yellowSample.target.reduce((sum, target, channel) => sum
        + (markerMatrix[channel].slice(0, 3).reduce((value, coefficient, input) =>
          value + coefficient * yellowSample.normalizedSource[input], markerMatrix[channel][3]) - target) ** 2, 0) / 3)
      : null;
    // A neutral shadow edge at the frame position is useful geometric
    // evidence, but cannot redefine yellow when the four pigments disagree.
    // Check that extra reference independently before letting it affect ink.
    const yellowVerified = !!yellowSample?.normalizedSource && (yellowError == null || yellowError < .2);
    if (yellowVerified) normalizedReferences.splice(normalizedReferences.length - 1, 0, {
      ...yellowSample, source: yellowSample.normalizedSource, weight: 1.2
    });
    const referenceByName = Object.fromEntries(
      [...validMarkerSamples, ...(yellowVerified ? [yellowSample] : [])]
        .map(sample => [sample.name, sample])
    );
    const colorMatrix = canCalibrate ? buildColorMatrix(normalizedReferences) : null;
    const matrixError = colorMatrix ? Math.sqrt(normalizedReferences.reduce((total, sample) => total
      + sample.target.reduce((sum, target, c) => sum + (colorMatrix[c][0] * sample.source[0]
        + colorMatrix[c][1] * sample.source[1] + colorMatrix[c][2] * sample.source[2] + colorMatrix[c][3] - target) ** 2, 0), 0)
      / (normalizedReferences.length * 3)) : null;
    const calibratedMatrix = colorMatrix && matrixError < .14 ? colorMatrix : null;
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
    const columnFlags = new Uint8Array(canvas.width);
    const illuminationColumns = new Uint16Array(canvas.width);
    const illuminationColumnMix = observedIllumination ? new Float32Array(canvas.width) : null;
    for (let x = 0; x < canvas.width; x += 1) {
      columnFlags[x] = (Math.abs(x - yellowX1) < outerBand || Math.abs(x - yellowX2) < outerBand ? 1 : 0)
        | (x < yellowX1 - outerBand || x > yellowX2 + outerBand ? 2 : 0)
        | (x >= colorMarkerX1 && x <= colorMarkerX2 ? 4 : 0)
        | (Math.abs((x / pxPerCm * 2) - Math.round(x / pxPerCm * 2)) < 0.24 ? 8 : 0);
      if (observedIllumination) {
        const at = clamp((x + 0.5) * illuminationWidth / canvas.width - 0.5, 0, illuminationWidth - 1);
        illuminationColumns[x] = Math.floor(at);
        illuminationColumnMix[x] = at - illuminationColumns[x];
      } else illuminationColumns[x] = Math.round(x * illuminationXScale);
    }
    const colorParameters = new Float64Array(61);
    colorParameters.set([+preciseStencil, +useStencil, +!!observedIllumination, observedIllumination?.noiseFloor || 2,
      ...profile.gains, ...profile.sourcePaper, profile.paperLuminance, profile.targetPaper, +canCalibrate, +!!calibratedMatrix]);
    if (calibratedMatrix) colorParameters.set(calibratedMatrix.flat(), 14);
    ['red','black','blue','green','yellow'].forEach((name,index) => {
      const sample = referenceByName[name];
      if (sample?.source) colorParameters.set([1, ...sample.source, ...sample.target], 26 + index * 7);
    });
    await globalThis.InhouseRust?.ready;
    checkProcessingCancelled(settings);
    const pixelsStarted = performance.now();
    const rustColors = settings.disableRust ? null : globalThis.InhouseRust?.scanColors?.(data, original, canvas.width, canvas.height,
      illumination, observedIllumination?.drawing, illuminationWidth, illuminationHeight, colorParameters);
    if (rustColors) {
      try {
        for (let from = 0; from < canvas.height; from += processingWorker ? canvas.height : 96) {
          checkProcessingCancelled(settings);
          rustColors.rows(from, Math.min(canvas.height, from + (processingWorker ? canvas.height : 96)));
          if (!processingWorker) await yieldToBrowser();
        }
        rustColors.copyTo(data);
      } finally { rustColors.dispose(); }
    } else for (let y = 0; y < canvas.height; y += 1) {
      const outerRow = Math.abs(y - yellowY1) < outerBand || Math.abs(y - yellowY2) < outerBand;
      const marginRow = y < yellowY1 - outerBand || y > yellowY2 + outerBand;
      const markerRow = y >= colorMarkerY1 && y <= colorMarkerY2;
      const inCalibrationBand = y >= calibrationY1 && y <= calibrationY2;
      const printedRailRow = Math.abs(y - 27.43 * pxPerCm) <= outerBand
        || Math.abs(y - 27.93 * pxPerCm) <= outerBand;
      const illuminationAtY = observedIllumination
        ? clamp((y + 0.5) * illuminationHeight / canvas.height - 0.5, 0, illuminationHeight - 1)
        : Math.round(y * illuminationYScale);
      const illuminationRow = Math.floor(illuminationAtY) * illuminationWidth;
      const illuminationNextRow = Math.min(illuminationHeight - 1, Math.floor(illuminationAtY) + 1) * illuminationWidth;
      const illuminationRowMix = illuminationAtY - Math.floor(illuminationAtY);
      for (let x = 0; x < canvas.width; x += 1) {
        const offset = (y * canvas.width + x) * 4;
        const r = data[offset], g = data[offset + 1], b = data[offset + 2];
        const flags = columnFlags[x];
        const inOuterBand = !!(flags & 1) || outerRow;
        const inPageMargin = !!(flags & 2) || marginRow;
        const inColorMarkers = !!(flags & 4) && markerRow;
        const max = Math.max(r, g, b);
        const min = Math.min(r, g, b);
        const saturation = max ? (max - min) / max : 0;
        const luminance = r * 0.2126 + g * 0.7152 + b * 0.0722;
        // The template guarantees blank paper outside its yellow frame. When
        // the photographed sheet is clipped, extrapolation can otherwise pull
        // a dark folder or desk into this narrow margin. Clean only obvious
        // non-paper pixels; pale shadows and the physical paper edge remain.
        // Warp fills unobserved margins with neutral white. Those synthetic
        // pixels never saw the lamp, so an inverse blue cast must not turn
        // them yellow. Keep the original working buffer for this distinction.
        const syntheticWhiteMargin = original[offset] >= 254
          && original[offset + 1] >= 254 && original[offset + 2] >= 254;
        const outsidePaperArtifact = preciseStencil && inPageMargin
          && (syntheticWhiteMargin || luminance < 105 || (luminance < 178 && saturation > 0.22));
        if (outsidePaperArtifact) {
          data[offset] = profile.targetPaper;
          data[offset + 1] = profile.targetPaper;
          data[offset + 2] = profile.targetPaper;
          continue;
        }
        const warmPrintedYellow = r >= g * 0.85 && g >= r * 0.78
          && g >= b * 1.06 && r >= b * 1.12 && yellowChroma(r, g, b) >= 6;
        const onPrintedRail = inOuterBand || (inCalibrationBand && (printedRailRow || !!(flags & 8)));
        const shouldNeutralize = useStencil && (
          ((inOuterBand || inCalibrationBand) && isYellow(r, g, b))
          // Warm camera casts can move the printed yellow outside the strict
          // hue interval. Remove it on verified template rails before colour
          // calibration can amplify those harmless remnants into red specks.
          || (onPrintedRail && warmPrintedYellow)
          || (inColorMarkers && saturation > 0.24)
        );
        if (shouldNeutralize) {
          data[offset] = 255; data[offset + 1] = 255; data[offset + 2] = 255;
          continue;
        }
        const fieldColumn = illuminationColumns[x];
        let localBright = illumination[illuminationRow + fieldColumn];
        let localDrawing = 0;
        if (observedIllumination) {
          const mix = illuminationColumnMix[x], next = Math.min(illuminationWidth - 1, fieldColumn + 1);
          const upper = localBright * (1 - mix) + illumination[illuminationRow + next] * mix;
          const lower = illumination[illuminationNextRow + fieldColumn] * (1 - mix)
            + illumination[illuminationNextRow + next] * mix;
          localBright = upper * (1 - illuminationRowMix) + lower * illuminationRowMix;
          const mask = observedIllumination.drawing;
          const upperDrawing = mask[illuminationRow + fieldColumn] * (1 - mix) + mask[illuminationRow + next] * mix;
          const lowerDrawing = mask[illuminationNextRow + fieldColumn] * (1 - mix)
            + mask[illuminationNextRow + next] * mix;
          localDrawing = (upperDrawing * (1 - illuminationRowMix) + lowerDrawing * illuminationRowMix) / 255;
        }
        // Measured neutral paper uses only relative luminance. Avoid colour
        // reference projection and three discarded channel gains for the
        // millions of neutral pixels making up most scanned pages.
        const inkDeficit = localBright - luminance;
        // Camera JPEG chroma spreads beyond the sharp luminance edge. Colour
        // alone must not turn those otherwise paper-bright pixels into ink.
        const hasInkContrast = !observedIllumination
          || inkDeficit > Math.max(observedIllumination.noiseFloor * 1.5,localBright * .025);
        const colouredStroke = hasInkContrast && (saturation > 0.18 || (saturation > 0.105
          && inkDeficit > Math.max(6, localBright * 0.035)));
        const measuredNeutral = !!observedIllumination && !colouredStroke;
        const measuredBlack = measuredNeutral && (luminance < localBright * 0.16
          || (localDrawing > 0.85 && luminance < localBright * 0.26));
        const automaticR = measuredNeutral ? 0 : clamp(r * profile.gains[0], 0, 255);
        const automaticG = measuredNeutral ? 0 : clamp(g * profile.gains[1], 0, 255);
        const automaticB = measuredNeutral ? 0 : clamp(b * profile.gains[2], 0, 255);
        const kind = canCalibrate && (!measuredNeutral || measuredBlack)
          ? referenceKindForPixel(
            r, g, b, luminance, saturation, profile.paperLuminance, localBright
          )
          : null;
        const calibrated = kind
          ? calibrateFromReference([r, g, b], referenceByName[kind], profile.sourcePaper, profile.targetPaper)
          : null;
        let outputR = automaticR, outputG = automaticG, outputB = automaticB;
        if (!colouredStroke && (observedIllumination || !calibrated)) {
          // Recover ink contrast relative to the paper at this pixel. Blending
          // with a global white target washed faint strokes away even when the
          // image contained enough local contrast to read them clearly.
          const paper = Math.max(40, localBright);
          const noiseFloor = Math.max(observedIllumination?.noiseFloor || 2, paper * 0.0075);
          const reflectance = clamp(1 - (paper - luminance - noiseFloor) / paper, 0, 1);
          // Strengthen thin pencil independently of broad shaded drawings.
          // The dark transition retains already-dark ink's measured tone;
          // light stroke coverage gains contrast without crushing grey fills.
          const transition = clamp((reflectance - 0.12) / 0.45, 0, 1);
          const strength = observedIllumination
            ? (1.25 - localDrawing * 0.6) * transition * transition * (3 - 2 * transition) : 0.65;
          let neutral = profile.targetPaper * clamp(
            reflectance - strength * reflectance * (1 - reflectance), 0, 1);
          if (measuredBlack && calibrated && kind === "black") {
            // Deep camera blacks include a camera contrast/offset response,
            // not only exposure. Retain a little measured black-reference
            // calibration without applying its grey lift to faint writing.
            const reference = calibrated.mapped[0] * 0.2126 + calibrated.mapped[1] * 0.7152
              + calibrated.mapped[2] * 0.0722;
            neutral += (reference - neutral) * (localDrawing > 0.85 ? 1 : 0.25) * calibrated.mix;
          }
          outputR = outputG = outputB = neutral;
        } else if (calibratedMatrix && colouredStroke) {
          const paper = Math.max(40, localBright);
          const nr = clamp(r / paper * profile.paperLuminance / Math.max(40, profile.sourcePaper[0]), 0, 1.2), ng = clamp(g / paper * profile.paperLuminance / Math.max(40, profile.sourcePaper[1]), 0, 1.2), nb = clamp(b / paper * profile.paperLuminance / Math.max(40, profile.sourcePaper[2]), 0, 1.2);
          outputR = 255 * (calibratedMatrix[0][0] * nr + calibratedMatrix[0][1] * ng + calibratedMatrix[0][2] * nb + calibratedMatrix[0][3]);
          outputG = 255 * (calibratedMatrix[1][0] * nr + calibratedMatrix[1][1] * ng + calibratedMatrix[1][2] * nb + calibratedMatrix[1][3]);
          outputB = 255 * (calibratedMatrix[2][0] * nr + calibratedMatrix[2][1] * ng + calibratedMatrix[2][2] * nb + calibratedMatrix[2][3]);
        } else if (calibrated) {
          // Without measured paper anchors, retain the printed reference's
          // tonal calibration rather than relying on a broad local estimate.
          outputR = automaticR * (1 - calibrated.mix) + calibrated.mapped[0] * calibrated.mix;
          outputG = automaticG * (1 - calibrated.mix) + calibrated.mapped[1] * calibrated.mix;
          outputB = automaticB * (1 - calibrated.mix) + calibrated.mapped[2] * calibrated.mix;
        } else {
          // Preserve colours that are not one of the printed references, but
          // restore a small amount of saturation lost through camera exposure.
          const balancedLuminance = automaticR * 0.2126 + automaticG * 0.7152 + automaticB * 0.0722;
          const vibrance = 1.08;
          outputR = balancedLuminance + (automaticR - balancedLuminance) * vibrance;
          outputG = balancedLuminance + (automaticG - balancedLuminance) * vibrance;
          outputB = balancedLuminance + (automaticB - balancedLuminance) * vibrance;
        }
        if (observedIllumination && colouredStroke) {
          // Thin pen lines lose chroma through camera/JPEG subsampling more
          // than the large reference circles do. Recover that extra dilution
          // around their luminance; preserve hue and leave solid fills alone.
          const strongest = Math.max(outputR, outputG, outputB);
          const weakest = Math.min(outputR, outputG, outputB);
          const dilution = 1 - clamp((strongest-weakest) / Math.max(1,strongest),0,1);
          const edge = clamp((inkDeficit / Math.max(40,localBright) - .025) / .16,0,1);
          const edgeStrength = edge * edge * (3 - 2 * edge);
          const vibrance = 1 + .6 * (1-localDrawing) * dilution * edgeStrength;
          const tone = outputR*.2126 + outputG*.7152 + outputB*.0722;
          outputR = tone + (outputR-tone)*vibrance;
          outputG = tone + (outputG-tone)*vibrance;
          outputB = tone + (outputB-tone)*vibrance;
          // Increase coloured stroke coverage against white, preserving hue
          // instead of desaturating pale blue/red handwriting into grey.
          // Solid fills and already-saturated reference colours retain their
          // calibrated palette; the extra coverage is for pastel strokes.
          const coverage = 1 + 0.38 * (1 - localDrawing) * edgeStrength
            * clamp((Math.min(outputR, outputG, outputB) - 40) / 90, 0, 1);
          outputR = profile.targetPaper - (profile.targetPaper - outputR) * coverage;
          outputG = profile.targetPaper - (profile.targetPaper - outputG) * coverage;
          outputB = profile.targetPaper - (profile.targetPaper - outputB) * coverage;
        }
        // Near-black JPEG channels may be clipped under a warm lamp. A colour
        // matrix cannot recover that missing signal; preserve their dark tone
        // rather than lifting dark writing into a mustard/pastel fill.
        if (colouredStroke && max < 100 && luminance < 80 && r > g && g > b && g >= r * .35) {
          const limit = (automaticR * .2126 + automaticG * .7152 + automaticB * .0722) * 1.1 + 3;
          const mappedLuminance = outputR * .2126 + outputG * .7152 + outputB * .0722;
          if (mappedLuminance > limit) {
            const scale = limit / mappedLuminance;
            outputR *= scale; outputG *= scale; outputB *= scale;
          }
        }
        data[offset] = Math.round(clamp(outputR, 0, 255));
        data[offset + 1] = Math.round(clamp(outputG, 0, 255));
        data[offset + 2] = Math.round(clamp(outputB, 0, 255));
      }
      if (y % 48 === 47) {
        checkProcessingCancelled(settings);
        settings.onProgress?.((y + 1) / canvas.height);
        await yieldToBrowser();
      }
    }
    const pixelsMs = performance.now() - pixelsStarted;
    const cleanupStarted = performance.now();
    const stencilCleanup = preciseStencil && !settings.skipStencilCleanup
      ? await removePhotographedStencil(data, canvas.width, canvas.height, pxPerCm, profile.targetPaper,
        observedIllumination ? { ...settings, photographedDotObservations: observedIllumination.paperPoints } : settings)
      : null;
    const paperOutput = [];
    const coveredRegions = new Set();
    const qualityOffsets = Array.from({ length: 16 }, (_, sample) => {
      const angle = sample / 16 * Math.PI * 2;
      return [Math.cos(angle) * pxPerCm * 0.15, Math.sin(angle) * pxPerCm * 0.15];
    });
    for (const point of observedIllumination?.paperPoints || []) {
      const samples = [];
      // Score actual cleaned paper surrounding an evidenced printed point.
      // A high quantile ignores a pen crossing the ring without penalising a
      // page for containing handwriting; a remaining shadow affects the ring.
      for (const [dx, dy] of qualityOffsets) {
        const x = Math.round(point.x - 0.5 + dx);
        const y = Math.round(point.y - 0.5 + dy);
        if (x < 0 || y < 0 || x >= canvas.width || y >= canvas.height) continue;
        const offset = (y * canvas.width + x) * 4;
        const r = data[offset], g = data[offset + 1], b = data[offset + 2];
        if (Math.max(r, g, b) - Math.min(r, g, b) > 26) continue;
        samples.push(r * 0.2126 + g * 0.7152 + b * 0.0722);
      }
      if (samples.length < 8) continue;
      samples.sort((a, b) => a - b);
      paperOutput.push(samples[Math.floor(samples.length * 0.8)]);
      coveredRegions.add(Math.floor(point.col / 5) * 9 + Math.floor(point.row / 6));
    }
    paperOutput.sort((a, b) => a - b);
    const outputQuality = {
      method: "verified-dot-surrounds",
      paperSamples: paperOutput.length,
      paperResidualFraction: paperOutput.length
        ? paperOutput.filter(value => value < 235).length / paperOutput.length : null,
      paperP10: paperOutput.length ? paperOutput[Math.floor(paperOutput.length * 0.1)] : null,
      paperMedian: paperOutput.length ? paperOutput[Math.floor(paperOutput.length * 0.5)] : null,
      confidence: Math.min(1, paperOutput.length / 600) * Math.min(1, coveredRegions.size / 50)
    };
    if (data !== original) original.set(data);
    context.putImageData(image, 0, 0);
    illuminationCanvas.width = 0;
    illuminationCanvas.height = 0;
    return {
      canvas,
      pixelProcessing: { backend: rustColors ? "rust" : "javascript", pixelsMs, cleanupMs: performance.now() - cleanupStarted },
      calibrated: canCalibrate,
      calibration: { method: calibratedMatrix ? (yellowVerified ? "paper-relative-six-reference" : "paper-relative-four-markers") : "bounded-reference-fallback", matrixError, yellowError, matrix: calibratedMatrix, sourcePaper: profile.sourcePaper, normalizedReferences, references: normalizedReferences.map(sample => sample.name) },
      stencilCleanup,
      outputQuality,
      illumination: observedIllumination ? { method: "observed-paper", anchors: observedIllumination.anchors,
        supplied: observedIllumination.supplied, recovered: observedIllumination.recovered,
        noiseSigma: observedIllumination.noiseSigma, noiseFloor: observedIllumination.noiseFloor } : { method: "local-paper" },
      samples: validMarkerSamples.length,
      paper: Math.round(profile.paperLuminance),
      references: Object.fromEntries(samples.map(sample => [sample.name, {
        source: sample.source?.map(value => Math.round(value * 255)) || null,
        valid: sample.name === "yellow"
          ? yellowVerified
          : markerLooksValid(sample.name, sample.normalizedSource)
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
