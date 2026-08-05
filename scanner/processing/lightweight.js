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
    const low = Math.min(r, g);
    return r > 92 && g > 78 && b < 178 && low - b > 25 && Math.abs(r - g) < 125;
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
      if (count >= 2 && last - first > width * 0.46) {
        if (first < width * 0.42) leftPoints.push({ x: first, y });
        if (last > width * 0.58) rightPoints.push({ x: last, y });
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
      if (count >= 2 && last - first > height * 0.46) {
        if (first < height * 0.42) topPoints.push({ x, y: first });
        if (last > height * 0.58) bottomPoints.push({ x, y: last });
      }
    }

    const leftLine = robustLine(leftPoints, "y", "x");
    const rightLine = robustLine(rightPoints, "y", "x");
    const topLine = robustLine(topPoints, "x", "y");
    const bottomLine = robustLine(bottomPoints, "x", "y");
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
      const plausible = Math.min(topWidth, bottomWidth) > width * 0.45
        && Math.min(leftHeight, rightHeight) > height * 0.45;
      if (plausible) {
        confidence = Math.min(1, yellowPixels / Math.max(1, width * height * 0.004));
      }
    }

    canvas.width = 0;
    canvas.height = 0;
    if (confidence < 0.24) {
      return { pageQuad: fallbackPageQuad(source), stencilQuad: null, confidence: 0 };
    }
    stencilQuad = stencilQuad.map(point => ({ x: point.x / scale, y: point.y / scale }));
    return {
      pageQuad: extrapolateStencil(stencilQuad),
      stencilQuad,
      confidence
    };
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

  Light.correctColors = async function correctColors(canvas, useStencil) {
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
    const samples = useStencil
      ? markerDefinitions.map(([x, name, fallback]) => ({
        source: medianSample(data, canvas.width, canvas.height, x * pxPerCm, 27.68 * pxPerCm, radius),
        target: (targets[name] || fallback.map(value => value * 255)).map(value => value / 255),
        weight: name === "black" ? 1.4 : 1
      }))
      : [];
    if (useStencil) {
      const yellowSource = medianSample(
        data, canvas.width, canvas.height,
        1.5 * pxPerCm, 14.8 * pxPerCm, pxPerCm * 0.16,
        isYellow
      );
      const targetYellow = config.TARGET_YELLOW || { R: 240, G: 219, B: 76 };
      samples.push({
        source: yellowSource,
        target: [targetYellow.R / 255, targetYellow.G / 255, targetYellow.B / 255],
        weight: 0.75
      });
    }
    const whiteLocations = [[0.7, 0.7], [20.3, 0.7], [0.7, 29], [20.3, 29]];
    const whites = whiteLocations
      .map(([x, y]) => medianSample(data, canvas.width, canvas.height, x * pxPerCm, y * pxPerCm, pxPerCm * 0.2))
      .filter(Boolean)
      .sort((a, b) => (b[0] + b[1] + b[2]) - (a[0] + a[1] + a[2]));
    samples.push({ source: whites[0] || [1, 1, 1], target: [1, 1, 1], weight: 1.6 });
    const matrix = buildColorMatrix(samples);
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
        if (useStencil && (inColorMarkers
          || (inOuterBand && isYellow(r, g, b))
          || (y >= calibrationY1 && y <= calibrationY2 && isYellow(r, g, b)))) {
          data[offset] = 255; data[offset + 1] = 255; data[offset + 2] = 255;
          continue;
        }
        const nr = r / 255, ng = g / 255, nb = b / 255;
        data[offset] = clamp(Math.round((matrix[0][0] * nr + matrix[0][1] * ng + matrix[0][2] * nb + matrix[0][3]) * 255), 0, 255);
        data[offset + 1] = clamp(Math.round((matrix[1][0] * nr + matrix[1][1] * ng + matrix[1][2] * nb + matrix[1][3]) * 255), 0, 255);
        data[offset + 2] = clamp(Math.round((matrix[2][0] * nr + matrix[2][1] * ng + matrix[2][2] * nb + matrix[2][3]) * 255), 0, 255);
      }
      if (y % 96 === 95) await yieldToBrowser();
    }
    context.putImageData(image, 0, 0);
    return { canvas, samples: samples.filter(sample => sample.source).length };
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
