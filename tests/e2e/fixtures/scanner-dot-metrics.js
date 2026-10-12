(() => {
  'use strict';
  const quantile = (values, fraction) => [...values].sort((a, b) => a - b)[Math.min(values.length - 1, Math.floor(values.length * fraction))];
  function measure(canvas, options = {}) {
    const { width, height } = canvas;
    const data = canvas.getContext('2d', { willReadFrequently: true }).getImageData(0, 0, width, height).data;
    const luminance = (x, y) => {
      const index = (Math.max(0, Math.min(height - 1, y)) * width + Math.max(0, Math.min(width - 1, x))) * 4;
      return .2126 * data[index] + .7152 * data[index + 1] + .0722 * data[index + 2];
    };
    const pitch = width * 5 / 210;
    const search = Math.max(2, Math.floor(pitch * .28));
    const radius = Math.max(2, Math.ceil(pitch * (options.radiusFraction || .14)));
    const found = [];
    const sites = options.sites ? new Set(options.sites.map(point => `${point.row}:${point.col}`)) : null;
    for (let row = 0; row < 51; row++) for (let col = 0; col < 35; col++) {
      if (sites && !sites.has(`${row}:${col}`)) continue;
      const xMm = 20 + col * 5, yMm = 19.3 + row * 5;
      if (options.excludeWriting && (
        [35, 75, 115, 155, 195, 235].some(y => Math.abs(yMm - y) < 2) ||
        [35, 75, 115, 155, 180].some(x => Math.abs(xMm - x) < 2) ||
        (yMm > 49 && yMm < 57) || (yMm > 129 && yMm < 137))) continue;
      if (options.missingGrid && ((col * 17 + row * 7) % 13 < 2 ||
        (col > 14 && col < 20 && row > 22 && row < 30))) continue;
      const targetX = xMm / 210 * width, targetY = yMm / 297 * height;
      const ring = [];
      for (let i = 0; i < 24; i++) {
        const angle = i * Math.PI / 12;
        ring.push(luminance(Math.round(targetX - .5 + Math.cos(angle) * pitch * .36),
          Math.round(targetY - .5 + Math.sin(angle) * pitch * .36)));
      }
      const paper = quantile(ring, .65);
      let best = -Infinity, peakX, peakY;
      const centerX = Math.round(targetX - .5), centerY = Math.round(targetY - .5);
      for (let y = centerY - search; y <= centerY + search; y++) {
        for (let x = centerX - search; x <= centerX + search; x++) {
          const darkness = paper - luminance(x, y);
          if (darkness > best) { best = darkness; peakX = x; peakY = y; }
        }
      }
      if (best < 5) continue;
      let weight = 0, sx = 0, sy = 0;
      for (let dy = -radius; dy <= radius; dy++) for (let dx = -radius; dx <= radius; dx++) {
        if (dx * dx + dy * dy > radius * radius) continue;
        const x = peakX + dx, y = peakY + dy;
        const value = Math.max(0, paper - luminance(x, y) - Math.max(options.noiseFloor ?? 2, best * (options.relativeNoise ?? .035)));
        sx += (x + .5) * value; sy += (y + .5) * value; weight += value;
      }
      if (!weight) continue;
      const x = sx / weight, y = sy / weight;
      found.push({ row, col, x, y, targetX, targetY, dx: x - targetX, dy: y - targetY, strength: best });
    }
    const errors = found.map(point => Math.hypot(point.dx, point.dy));
    const all = new Map(found.map(point => [`${point.row}:${point.col}`, point]));
    const rowZigzag = [], columnZigzag = [];
    for (const point of found) {
      const left = all.get(`${point.row}:${point.col - 1}`), right = all.get(`${point.row}:${point.col + 1}`);
      if (left && right) rowZigzag.push(Math.abs(point.dy - (left.dy + right.dy) / 2));
      const top = all.get(`${point.row - 1}:${point.col}`), bottom = all.get(`${point.row + 1}:${point.col}`);
      if (top && bottom) columnZigzag.push(Math.abs(point.dx - (top.dx + bottom.dx) / 2));
    }
    const stats = values => ({ count: values.length, median: quantile(values, .5), p95: quantile(values, .95), maximum: Math.max(...values) });
    return { count: found.length, error: stats(errors), rowZigzag: stats(rowZigzag), columnZigzag: stats(columnZigzag),
      meanX: found.reduce((sum, point) => sum + point.dx, 0) / found.length,
      meanY: found.reduce((sum, point) => sum + point.dy, 0) / found.length, points: found };
  }
  function measureWriting(canvas) {
    const { width, height } = canvas;
    const pixels = canvas.getContext('2d').getImageData(0, 0, width, height).data;
    const errors = [];
    for (const horizontal of [true, false]) {
      const lines = horizontal ? [35, 75, 115, 155, 195, 235] : [35, 75, 115, 155, 180];
      const samples = horizontal ? [30, 45, 60, 90, 105, 120, 135, 150, 165] : [50, 90, 130, 170, 210, 250];
      const dimension = horizontal ? height : width, pageDimension = horizontal ? 297 : 210;
      for (const line of lines) for (const sample of samples) {
        const at = Math.round(sample / (horizontal ? 210 : 297) * (horizontal ? width : height));
        const expected = line / pageDimension * dimension;
        let weight = 0, position = 0;
        for (let along = Math.floor(expected - dimension * .025); along <= Math.ceil(expected + dimension * .025); along++) {
          for (let side = -1; side <= 1; side++) {
            const x = horizontal ? at + side : along, y = horizontal ? along : at + side;
            const offset = (y * width + x) * 4;
            const strength = horizontal ? pixels[offset + 2] - (pixels[offset] + pixels[offset + 1]) / 2
              : pixels[offset] - (pixels[offset + 1] + pixels[offset + 2]) / 2;
            const value = Math.max(0, strength - 10);
            weight += value; position += (along + .5) * value;
          }
        }
        errors.push(weight ? Math.abs(position / weight - expected) / dimension : 1);
      }
    }
    return { maximum: Math.max(...errors), mean: errors.reduce((sum, value) => sum + value, 0) / errors.length };
  }
  window.ScannerDotMetrics = { measure, measureWriting };
})();
