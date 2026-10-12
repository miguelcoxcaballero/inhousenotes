(() => {
  "use strict";
  // Fit a smooth, developable sheet in millimetres, then project it through
  // a pinhole camera. Its tangent is (cos(theta(s)), sin(theta(s))), so
  // dX²+dZ²=ds²: the model bends paper without stretching its writing.
  // Straight rulings run perpendicular to a freely fitted bend direction.
  // The printed 5 mm lattice supplies metric correspondences; the observed
  // yellow perimeter constrains extrapolation beyond the outermost dots.
  // This is a single-view shape hypothesis, not a calibrated depth reading.
  // Accept it only when held-out dots, perimeter and orientation agree.

  const SP = (window.ScannerPro = window.ScannerPro || {});
  const W = 210,
    H = 297,
    STEPS = 128;
  const clamp = (v, a, b) => Math.max(a, Math.min(b, v));
  const norm = (p) => Math.hypot(...p);
  const dot = (a, b) => a.reduce((sum, v, i) => sum + v * b[i], 0);
  const cross = (a, b) => [
    a[1] * b[2] - a[2] * b[1],
    a[2] * b[0] - a[0] * b[2],
    a[0] * b[1] - a[1] * b[0],
  ];
  const median = (values) => {
    const s = [...values].sort((a, b) => a - b);
    return s[Math.floor(s.length / 2)];
  };
  const percentile = (values, p) => {
    const s = [...values].sort((a, b) => a - b);
    return s[Math.min(s.length - 1, Math.floor(s.length * p))];
  };
  function solve(matrix, values) {
    const n = values.length,
      rows = matrix.map((row, i) => [...row, values[i]]);
    for (let i = 0; i < n; i++) {
      let pivot = i;
      for (let j = i + 1; j < n; j++)
        if (Math.abs(rows[j][i]) > Math.abs(rows[pivot][i])) pivot = j;
      if (Math.abs(rows[pivot][i]) < 1e-12) return null;
      [rows[i], rows[pivot]] = [rows[pivot], rows[i]];
      const divisor = rows[i][i];
      for (let k = i; k <= n; k++) rows[i][k] /= divisor;
      for (let j = 0; j < n; j++)
        if (i !== j) {
          const factor = rows[j][i];
          for (let k = i; k <= n; k++) rows[j][k] -= factor * rows[i][k];
        }
    }
    return rows.map((row) => row[n]);
  }
  function rotation(p) {
    const theta = Math.hypot(p[0], p[1], p[2]);
    if (theta < 1e-10) return [1, 0, 0, 0, 1, 0, 0, 0, 1];
    const [x, y, z] = p.slice(0, 3).map((v) => v / theta),
      c = Math.cos(theta),
      s = Math.sin(theta),
      d = 1 - c;
    return [
      c + x * x * d,
      x * y * d - z * s,
      x * z * d + y * s,
      y * x * d + z * s,
      c + y * y * d,
      y * z * d - x * s,
      z * x * d - y * s,
      z * y * d + x * s,
      c + z * z * d,
    ];
  }
  function prepare(p, width, height) {
    const beta = p[7],
      cb = Math.cos(beta),
      sb = Math.sin(beta),
      length = (Math.abs(cb) * W + Math.abs(sb) * H) / 2;
    const middle = (p.length - 8) / 2,
      angles = [...p.slice(8, 8 + middle), 0, ...p.slice(8 + middle)],
      knots = angles.length,
      xs = [0],
      zs = [0];

    let previous = angles[0];
    for (let i = 1; i <= STEPS; i++) {
      const t = (i / STEPS) * (knots - 1),
        index = Math.min(knots - 2, Math.floor(t));
      const angle =
        angles[index] + (angles[index + 1] - angles[index]) * (t - index);
      xs.push(
        xs[i - 1] + ((Math.cos(previous) + Math.cos(angle)) * length) / STEPS,
      );
      zs.push(
        zs[i - 1] + ((Math.sin(previous) + Math.sin(angle)) * length) / STEPS,
      );
      previous = angle;
    }
    const centerX = xs[STEPS / 2],
      centerZ = zs[STEPS / 2];
    for (let i = 0; i <= STEPS; i++) {
      xs[i] -= centerX;
      zs[i] -= centerZ;
    }
    return {
      p,
      width,
      height,
      cb,
      sb,
      length,
      xs,
      zs,
      rotation: rotation(p),
      depth: Math.exp(p[5]),
      focal: Math.exp(p[6]),
    };
  }
  function project(model, u, v) {
    const { p, cb, sb, length, xs, zs, rotation: r } = model;
    const x = (u - 0.5) * W,
      y = (v - 0.5) * H,
      s = x * cb + y * sb,
      t = -x * sb + y * cb;
    const pos = clamp(((s / length + 1) * STEPS) / 2, 0, STEPS),
      i = Math.min(STEPS - 1, Math.floor(pos)),
      mix = pos - i;
    const curve = xs[i] + (xs[i + 1] - xs[i]) * mix;
    const xx = curve * cb - t * sb,
      yy = curve * sb + t * cb,
      z = zs[i] + (zs[i + 1] - zs[i]) * mix;
    const X = r[0] * xx + r[1] * yy + r[2] * z + p[3] * 100,
      Y = r[3] * xx + r[4] * yy + r[5] * z + p[4] * 100;
    const Z = r[6] * xx + r[7] * yy + r[8] * z + model.depth;
    return {
      x: model.width / 2 + (model.focal * X) / Z,
      y: model.height / 2 + (model.focal * Y) / Z,
    };
  }
  function initial(quad, width, height) {
    const f = Math.max(width, height) * 1.4,
      physical = [
        [15, 14.3],
        [195, 14.3],
        [195, 284.3],
        [15, 284.3],
      ];
    const A = [],
      b = [];
    quad.forEach((point, i) => {
      const u = physical[i][0] - W / 2,
        v = physical[i][1] - H / 2,
        { x, y } = point;
      A.push(
        [u, v, 1, 0, 0, 0, -u * x, -v * x],
        [0, 0, 0, u, v, 1, -u * y, -v * y],
      );
      b.push(x, y);
    });
    const h = solve(A, b);
    if (!h) return null;
    const a = [
      (h[0] - (width / 2) * h[6]) / f,
      (h[3] - (height / 2) * h[6]) / f,
      h[6],
    ];
    const c = [
      (h[1] - (width / 2) * h[7]) / f,
      (h[4] - (height / 2) * h[7]) / f,
      h[7],
    ];
    const translation = [(h[2] - width / 2) / f, (h[5] - height / 2) / f, 1];
    const scale = 2 / (norm(a) + norm(c)),
      r1 = a.map((v) => v / norm(a));
    let r2 = c.map((v, i) => v - r1[i] * dot(c, r1));
    r2 = r2.map((v) => v / norm(r2));
    const r3 = cross(r1, r2);
    const theta = Math.acos(clamp((r1[0] + r2[1] + r3[2] - 1) / 2, -1, 1)),
      factor = theta < 1e-6 ? 0.5 : theta / (2 * Math.sin(theta));
    const rot = [r2[2] - r3[1], r3[0] - r1[2], r1[1] - r2[0]].map(
      (v) => v * factor,
    );
    return [
      ...rot,
      (translation[0] * scale) / 100,
      (translation[1] * scale) / 100,
      Math.log(scale),
      Math.log(f),
      0,
      ...Array(8).fill(0),
    ];
  }
  function blur(input, width, height, sigma) {
    const radius = Math.ceil(sigma * 3),
      kernel = [];
    let total = 0;
    for (let i = -radius; i <= radius; i++) {
      const weight = Math.exp((-i * i) / (2 * sigma * sigma));
      kernel.push(weight);
      total += weight;
    }
    for (let i = 0; i < kernel.length; i++) kernel[i] /= total;
    const temp = new Float32Array(input.length),
      out = new Float32Array(input.length);
    for (let y = 0; y < height; y++)
      for (let x = 0; x < width; x++) {
        let value = 0;
        for (let k = -radius; k <= radius; k++)
          value +=
            input[y * width + clamp(x + k, 0, width - 1)] * kernel[k + radius];
        temp[y * width + x] = value;
      }
    for (let y = 0; y < height; y++)
      for (let x = 0; x < width; x++) {
        let value = 0;
        for (let k = -radius; k <= radius; k++)
          value +=
            temp[clamp(y + k, 0, height - 1) * width + x] * kernel[k + radius];
        out[y * width + x] = value;
      }
    return out;
  }
  function broadBlur(input, width, height) {
    // Two radius-two box convolutions have variance four (sigma two).
    // Sliding sums keep this background estimate linear in pixel count.
    let current = input;
    for (let pass = 0; pass < 2; pass++) {
      const temp = new Float32Array(input.length),
        out = new Float32Array(input.length);
      for (let y = 0; y < height; y++) {
        let sum = 0;
        for (let k = -2; k <= 2; k++)
          sum += current[y * width + clamp(k, 0, width - 1)];
        for (let x = 0; x < width; x++) {
          temp[y * width + x] = sum / 5;
          sum +=
            current[y * width + clamp(x + 3, 0, width - 1)] -
            current[y * width + clamp(x - 2, 0, width - 1)];
        }
      }
      for (let x = 0; x < width; x++) {
        let sum = 0;
        for (let k = -2; k <= 2; k++)
          sum += temp[clamp(k, 0, height - 1) * width + x];
        for (let y = 0; y < height; y++) {
          out[y * width + x] = sum / 5;
          sum +=
            temp[clamp(y + 3, 0, height - 1) * width + x] -
            temp[clamp(y - 2, 0, height - 1) * width + x];
        }
      }
      current = out;
    }
    return current;
  }
  function lineDistance(point, path) {
    let best = Infinity;
    for (let i = 0; i < path.length - 1; i++) {
      const a = path[i],
        b = path[i + 1],
        dx = b.x - a.x,
        dy = b.y - a.y;
      const t = clamp(
        ((point.x - a.x) * dx + (point.y - a.y) * dy) /
          (dx * dx + dy * dy || 1),
        0,
        1,
      );
      best = Math.min(
        best,
        Math.hypot(point.x - a.x - t * dx, point.y - a.y - t * dy),
      );
    }
    return best;
  }
  function inside(point, polygon) {
    let result = false;
    for (let i = 0, j = polygon.length - 1; i < polygon.length; j = i++) {
      const a = polygon[i],
        b = polygon[j];
      if (
        a.y > point.y !== b.y > point.y &&
        point.x < ((b.x - a.x) * (point.y - a.y)) / (b.y - a.y) + a.x
      )
        result = !result;
    }
    return result;
  }
  function coons(paths, u, v) {
    const sample = (path, t) => {
      const at = clamp(t, 0, 1) * (path.length - 1),
        i = Math.min(path.length - 2, Math.floor(at)),
        f = at - i;
      return {
        x: path[i].x + (path[i + 1].x - path[i].x) * f,
        y: path[i].y + (path[i + 1].y - path[i].y) * f,
      };
    };
    const top = sample(paths.top, u),
      bottom = sample(paths.bottom, u),
      left = sample(paths.left, v),
      right = sample(paths.right, v);
    const a = paths.top[0],
      b = paths.top.at(-1),
      c = paths.bottom.at(-1),
      d = paths.bottom[0];
    const coordinate = (key) =>
      top[key] * (1 - v) +
      bottom[key] * v +
      left[key] * (1 - u) +
      right[key] * u -
      (a[key] * (1 - u) * (1 - v) +
        b[key] * u * (1 - v) +
        c[key] * u * v +
        d[key] * (1 - u) * v);
    return { x: coordinate("x"), y: coordinate("y") };
  }
  function observe(source, frame, diagnostics = {}) {
    if (!frame?.paths) return null;
    const scale = Math.min(1, 1000 / Math.max(source.width, source.height)),
      width = Math.round(source.width * scale),
      height = Math.round(source.height * scale);
    const canvas =
      typeof OffscreenCanvas === "function"
        ? new OffscreenCanvas(width, height)
        : document.createElement("canvas");
    canvas.width = width;
    canvas.height = height;
    const ctx = canvas.getContext("2d", { willReadFrequently: true });
    ctx.drawImage(source, 0, 0, width, height);
    const rgba = ctx.getImageData(0, 0, width, height).data,
      luma = new Float32Array(width * height);
    for (let i = 0; i < luma.length; i++)
      luma[i] =
        0.2126 * rgba[i * 4] +
        0.7152 * rgba[i * 4 + 1] +
        0.0722 * rgba[i * 4 + 2];
    const smooth = broadBlur(luma, width, height),
      sharp = blur(luma, width, height, 0.55),
      response = new Float32Array(width * height);
    for (let i = 0; i < response.length; i++)
      response[i] = smooth[i] - sharp[i];
    const paths = {};
    for (const name of ["top", "right", "bottom", "left"])
      paths[name] = frame.paths[name].map((p) => ({
        x: p.x * scale,
        y: p.y * scale,
      }));
    let calibrationTop =
      frame.box?.support >= 0.5 &&
      frame.box.top?.length >= 7 &&
      frame.box.top?.every((p) => Number.isFinite(p.x) && Number.isFinite(p.y))
        ? frame.box.top.map((p) => ({ x: p.x * scale, y: p.y * scale }))
        : null;
    if (calibrationTop?.length >= 3) {
      let hits = 0,
        checked = 0;
      const pixel = (x, y, c) =>
        rgba[
          (clamp(Math.round(y), 0, height - 1) * width +
            clamp(Math.round(x), 0, width - 1)) *
            4 +
            c
        ];
      for (let i = 1; i < calibrationTop.length - 1; i++) {
        const a = calibrationTop[i - 1],
          point = calibrationTop[i],
          b = calibrationTop[i + 1],
          length = Math.hypot(b.x - a.x, b.y - a.y);
        if (length < 1) continue;
        const nx = -(b.y - a.y) / length,
          ny = (b.x - a.x) / length,
          background = [0, 1, 2].map(
            (c) =>
              (pixel(point.x + nx * 4, point.y + ny * 4, c) +
                pixel(point.x - nx * 4, point.y - ny * 4, c)) /
              2,
          );
        let supported = false;
        for (let offset = -2; offset <= 2; offset++) {
          const darkness = background.map(
            (value, c) =>
              (value - pixel(point.x + nx * offset, point.y + ny * offset, c)) /
              Math.max(40, value),
          );
          if (
            darkness[2] > 0.02 &&
            darkness[2] - (darkness[0] + darkness[1]) / 2 > 0.015
          )
            supported = true;
        }
        checked++;
        if (supported) hits++;
      }
      // Some frame hypotheses infer the box position from page proportions.
      // Only an actually visible coloured rail may censor grid observations.
      if (checked < 5 || hits / checked < 0.35) calibrationTop = null;
    }
    const aboveCalibration = (point) => {
      if (!calibrationTop || calibrationTop.length < 2) return true;
      let closest = Infinity,
        side = 0;
      for (let i = 1; i < calibrationTop.length; i++) {
        const a = calibrationTop[i - 1],
          b = calibrationTop[i],
          dx = b.x - a.x,
          dy = b.y - a.y,
          length = Math.hypot(dx, dy);
        if (!length) continue;
        const t = clamp(
            ((point.x - a.x) * dx + (point.y - a.y) * dy) / (length * length),
            0,
            1,
          ),
          distance = Math.hypot(point.x - a.x - t * dx, point.y - a.y - t * dy);
        if (distance < closest) {
          closest = distance;
          side = ((point.x - a.x) * dy - (point.y - a.y) * dx) / length;
        }
      }
      // The calibration boxes contain sharp corners at the same 5 mm pitch
      // as the dots. JPEG ringing can make those corners look like extra grid
      // rows. The measured upper box rail is a physical exclusion boundary;
      // the last real dot row sits one full pitch above it.
      return side > pitch * 0.25;
    };
    const polygon = [
      ...paths.top,
      ...paths.right,
      ...[...paths.bottom].reverse(),
      ...[...paths.left].reverse(),
    ];
    const quad = [
      paths.top[0],
      paths.top.at(-1),
      paths.bottom.at(-1),
      paths.bottom[0],
    ];
    const hu = {
      x: (quad[1].x - quad[0].x + quad[2].x - quad[3].x) / 72,
      y: (quad[1].y - quad[0].y + quad[2].y - quad[3].y) / 72,
    };
    const hv = {
      x: (quad[3].x - quad[0].x + quad[2].x - quad[1].x) / 108,
      y: (quad[3].y - quad[0].y + quad[2].y - quad[1].y) / 108,
    };
    const pitch = (Math.hypot(hu.x, hu.y) + Math.hypot(hv.x, hv.y)) / 2;
    if (pitch < 5 || pitch > 30) return null;
    let points = [];
    for (let y = 3; y < height - 3; y++)
      for (let x = 3; x < width - 3; x++) {
        const index = y * width + x,
          value = response[index],
          offset = index * 4;
        if (value < 5) continue;
        const channels = [rgba[offset], rgba[offset + 1], rgba[offset + 2]],
          highest = Math.max(...channels);
        const background = [0, 0, 0];
        for (let dy = -3; dy <= 3; dy++)
          for (let dx = -3; dx <= 3; dx++) {
            const at = ((y + dy) * width + x + dx) * 4;
            for (let c = 0; c < 3; c++) background[c] += rgba[at + c] / 49;
          }
        const contrast = background.map((value, c) => value - channels[c]);
        // A neutral printed dot inherits the illuminant of its local paper.
        // Compare fractional darkening of each channel, not absolute RGB
        // neutrality: blue room light can tint white paper by more than 50%.
        const relative = contrast.map(
            (value, c) => value / Math.max(40, background[c]),
          ),
          strongest = Math.max(...relative),
          weakest = Math.min(...relative);
        if (
          highest < 32 ||
          strongest < 0.014 ||
          weakest < strongest * 0.36 - 0.012 ||
          strongest - weakest > strongest * 0.56 + 0.016
        )
          continue;
        let maximum = true;
        for (let dy = -2; dy <= 2 && maximum; dy++)
          for (let dx = -2; dx <= 2; dx++)
            if (response[index + dy * width + dx] > value + 1e-5) {
              maximum = false;
              break;
            }
        if (!maximum) continue;
        const center = luma[index];
        let ringSupports = 0;
        for (let direction = 0; direction < 8; direction++) {
          const angle = (direction * Math.PI) / 4,
            rx = clamp(Math.round(x + Math.cos(angle) * 3), 0, width - 1),
            ry = clamp(Math.round(y + Math.sin(angle) * 3), 0, height - 1),
            ri = ry * width + rx;
          if (luma[ri] - center > value * 0.3) ringSupports++;
        }
        if (ringSupports < 6) continue;
        const p = { x, y };
        if (
          !inside(p, polygon) ||
          !aboveCalibration(p) ||
          Math.min(
            ...Object.values(paths).map((path) => lineDistance(p, path)),
          ) <
            pitch * 0.42
        )
          continue;
        let sx = 0,
          sy = 0,
          weight = 0;
        for (let dy = -1; dy <= 1; dy++)
          for (let dx = -1; dx <= 1; dx++) {
            const w = Math.max(0, response[index + dy * width + dx]);
            sx += (x + dx) * w;
            sy += (y + dy) * w;
            weight += w;
          }
        points.push({ x: sx / weight, y: sy / weight, strength: value });
      }
    const typicalContrast = median(points.map((p) => p.strength));
    points = points.filter((p) => p.strength < typicalContrast * 2.3);
    diagnostics.candidates = points.length;
    if (points.length < 350 || points.length > 4500) return null;
    const hash = new Map(),
      cell = pitch * 1.5,
      key = (x, y) => `${x},${y}`;
    points.forEach((p, i) => {
      const k = key(Math.floor(p.x / cell), Math.floor(p.y / cell));
      if (!hash.has(k)) hash.set(k, []);
      hash.get(k).push(i);
    });
    const determinant = hu.x * hv.y - hu.y * hv.x;
    if (Math.abs(determinant) < 1) return null;
    // The traced rails may be sampled by image distance or by physical
    // distance. Try both direction fields and retain the lattice supported
    // by more reciprocal, cycle-consistent observations. Neither hypothesis
    // determines the final camera or surface geometry.
    const bases = [];
    for (let row = 0; row <= 12; row++)
      for (let col = 0; col <= 8; col++) {
        const u = clamp(col / 8, 0.001, 0.999),
          v = clamp(row / 12, 0.001, 0.999),
          p = coons(paths, u, v),
          a = coons(paths, u + 0.0001, v),
          b = coons(paths, u, v + 0.0001);
        bases.push({
          ...p,
          du: { x: (a.x - p.x) / 0.0036, y: (a.y - p.y) / 0.0036 },
          dv: { x: (b.x - p.x) / 0.0054, y: (b.y - p.y) / 0.0054 },
        });
      }
    const buildGraph = (local) => {
      const links = points.map(() => Array(8).fill(-1)),
        directions = [
          [1, 0],
          [-1, 0],
          [0, 1],
          [0, -1],
          [2, 0],
          [-2, 0],
          [0, 2],
          [0, -2],
        ];
      points.forEach((p, i) => {
        const errors = Array(8).fill(Infinity),
          cx = Math.floor(p.x / cell),
          cy = Math.floor(p.y / cell);
        let du = hu,
          dv = hv;
        if (local) {
          let nearest = null,
            distance = Infinity;
          for (const candidate of bases) {
            const d = (candidate.x - p.x) ** 2 + (candidate.y - p.y) ** 2;
            if (d < distance) {
              nearest = candidate;
              distance = d;
            }
          }
          du = nearest.du;
          dv = nearest.dv;
        }
        const localDet = du.x * dv.y - du.y * dv.x;
        for (let yy = cy - 2; yy <= cy + 2; yy++)
          for (let xx = cx - 2; xx <= cx + 2; xx++)
            for (const j of hash.get(key(xx, yy)) || []) {
              if (i === j) continue;
              const dx = points[j].x - p.x,
                dy = points[j].y - p.y;
              const u = (dx * dv.y - dy * dv.x) / localDet,
                v = (dy * du.x - dx * du.y) / localDet;
              directions.slice(0, 4).forEach(([du, dv], d) => {
                const along = u * du + v * dv,
                  across = u * dv - v * du;
                if (
                  along < 0.5 ||
                  along > 1.5 ||
                  Math.abs(across) > along * 0.3
                )
                  return;
                const error = along + Math.abs(across) * 0.3;
                if (error < errors[d]) {
                  errors[d] = error;
                  links[i][d] = j;
                }
              });
            }
      });
      // Bridge a single missing dot only when the adjacent observed interval
      // establishes its size. Two-step links never compete with an observed
      // one-step neighbour, which would silently change the lattice phase.
      points.forEach((p, i) => {
        for (let d = 0; d < 4; d++) {
          if (links[i][d] >= 0) continue;
          const opposite = links[i][d ^ 1];
          if (opposite < 0) continue;
          const dx = p.x - points[opposite].x,
            dy = p.y - points[opposite].y,
            length = Math.hypot(dx, dy);
          const tx = p.x + dx * 2,
            ty = p.y + dy * 2,
            cx = Math.floor(tx / cell),
            cy = Math.floor(ty / cell);
          let error = length * 0.28,
            best = -1;
          for (let yy = cy - 1; yy <= cy + 1; yy++)
            for (let xx = cx - 1; xx <= cx + 1; xx++)
              for (const j of hash.get(key(xx, yy)) || []) {
                const distance = Math.hypot(points[j].x - tx, points[j].y - ty);
                if (distance < error) {
                  error = distance;
                  best = j;
                }
              }
          links[i][d + 4] = best;
        }
      });
      const visited = new Set(),
        components = [];
      points.forEach((_, seed) => {
        if (visited.has(seed)) return;
        const queue = [seed],
          position = new Map([[seed, [0, 0]]]);
        visited.add(seed);
        let mismatches = 0;
        const conflicts = new Set();
        for (let qi = 0; qi < queue.length; qi++) {
          const i = queue[qi];
          links[i].forEach((j, d) => {
            if (j < 0 || links[j][d ^ 1] !== i) return;
            const n = [
              position.get(i)[0] + directions[d][0],
              position.get(i)[1] + directions[d][1],
            ];
            if (position.has(j)) {
              const old = position.get(j);
              if (old[0] !== n[0] || old[1] !== n[1]) {
                mismatches++;
                conflicts.add(i);
                conflicts.add(j);
              }
            } else if (!visited.has(j)) {
              position.set(j, n);
              visited.add(j);
              queue.push(j);
            }
          });
        }
        components.push({ queue, position, mismatches, conflicts });
      });
      components.sort(
        (a, b) =>
          b.queue.length -
          b.conflicts.size * 4 -
          (a.queue.length - a.conflicts.size * 4),
      );
      return components[0];
    };
    const hypotheses = [buildGraph(false), buildGraph(true)];
    hypotheses.sort(
      (a, b) =>
        b.queue.length -
        b.conflicts.size * 4 -
        (a.queue.length - a.conflicts.size * 4),
    );
    const best = hypotheses[0];
    diagnostics.component = {
      count: best.queue.length,
      mismatches: best.mismatches,
    };
    if (
      best.queue.length < 350 ||
      best.conflicts.size > best.queue.length * 0.06
    )
      return null;
    const entries = best.queue
      .filter((i) => !best.conflicts.has(i))
      .map((i) => ({
        ...points[i],
        col: best.position.get(i)[0],
        row: best.position.get(i)[1],
      }));
    const minC = Math.min(...entries.map((p) => p.col)),
      maxC = Math.max(...entries.map((p) => p.col)),
      minR = Math.min(...entries.map((p) => p.row)),
      maxR = Math.max(...entries.map((p) => p.row));
    diagnostics.extent = [minC, maxC, minR, maxR];
    if (
      maxC - minC < 31 ||
      maxR - minR < 46 ||
      maxC - minC > 39 ||
      maxR - minR > 56
    )
      return null;
    const phases = [];
    for (let c0 = minC - 3; c0 <= minC + 3; c0++)
      for (let r0 = minR - 3; r0 <= minR + 3; r0++) {
        const selected = entries.filter(
          (p) =>
            p.col >= c0 && p.col < c0 + 35 && p.row >= r0 && p.row < r0 + 51,
        );
        const boundary = [[], [], [], []];
        for (const p of selected) {
          const c = p.col - c0,
            r = p.row - r0;
          if (c < 2)
            boundary[0].push(
              Math.abs(
                lineDistance(p, paths.left) / Math.hypot(hu.x, hu.y) - (c + 1),
              ),
            );
          if (c > 32)
            boundary[1].push(
              Math.abs(
                lineDistance(p, paths.right) / Math.hypot(hu.x, hu.y) -
                  (35 - c),
              ),
            );
          if (r < 2)
            boundary[2].push(
              Math.abs(
                lineDistance(p, paths.top) / Math.hypot(hv.x, hv.y) - (r + 1),
              ),
            );
          if (r > 48)
            boundary[3].push(
              Math.abs(
                lineDistance(p, paths.bottom) / Math.hypot(hv.x, hv.y) -
                  (54 - r - 1),
              ),
            );
        }
        if (boundary.some((side) => side.length < 12)) continue;
        phases.push({
          c0,
          r0,
          selected,
          score:
            boundary.reduce((sum, side) => sum + median(side), 0) / 4 +
            (entries.length - selected.length) /
              Math.max(30, entries.length * 0.08),
        });
      }
    phases.sort((a, b) => a.score - b.score);
    const phase = phases[0];
    diagnostics.phases = phases.slice(0, 3).map((p) => ({
      c: p.c0,
      r: p.r0,
      score: p.score,
      count: p.selected.length,
    }));
    if (
      !phase ||
      phase.score > 0.65 ||
      (phases[1] && phases[1].score - phase.score < 0.25)
    )
      return null;
    const measured = phase.selected.map((p) => ({
      col: p.col - phase.c0,
      row: p.row - phase.r0,
      u: (20 + 5 * (p.col - phase.c0)) / W,
      v: (19.3 + 5 * (p.row - phase.r0)) / H,
      x: p.x / scale,
      y: p.y / scale,
    }));
    const positions = new Map(),
      duplicates = new Set();
    for (const point of measured) {
      const key = `${point.col},${point.row}`;
      if (positions.has(key)) duplicates.add(key);
      positions.set(key, point);
    }
    const lattice = measured.filter((point) => {
      if (duplicates.has(`${point.col},${point.row}`)) return false;
      // A thin pencil stroke can have local maxima that imitate dot centers.
      // Require agreement with independently ordered neighboring grid dots,
      // without consulting the fitted 3D surface or modifying any ink pixels.
      for (const [dc, dr] of [
        [1, 0],
        [0, 1],
      ]) {
        const neighbors = [];
        for (const t of [-3, -2, -1, 1, 2, 3]) {
          const key = `${point.col + dc * t},${point.row + dr * t}`;
          if (positions.has(key) && !duplicates.has(key))
            neighbors.push({ ...positions.get(key), t });
        }
        if (
          neighbors.length < 4 ||
          !neighbors.some((p) => p.t < 0) ||
          !neighbors.some((p) => p.t > 0)
        )
          continue;
        const matrix = Array.from({ length: 3 }, () => [0, 0, 0]),
          bx = [0, 0, 0],
          by = [0, 0, 0];
        for (const neighbor of neighbors) {
          const basis = [1, neighbor.t, neighbor.t ** 2];
          for (let a = 0; a < 3; a++) {
            bx[a] += basis[a] * neighbor.x;
            by[a] += basis[a] * neighbor.y;
            for (let b = 0; b < 3; b++) matrix[a][b] += basis[a] * basis[b];
          }
        }
        const x = solve(matrix, bx),
          y = solve(matrix, by);
        if (!x || !y) continue;
        const errors = neighbors.map((p) =>
          Math.hypot(
            x[0] + x[1] * p.t + x[2] * p.t ** 2 - p.x,
            y[0] + y[1] * p.t + y[2] * p.t ** 2 - p.y,
          ),
        );
        if (
          Math.max(...errors) < (pitch / scale) * 0.08 &&
          Math.hypot(x[0] - point.x, y[0] - point.y) > (pitch / scale) * 0.22
        )
          return false;
      }
      return true;
    });
    diagnostics.rejectedGridOutliers = measured.length - lattice.length;
    if (lattice.length < 350 || lattice.length < measured.length * 0.85)
      return null;
    return {
      points: lattice,
      pitch: pitch / scale,
      phaseScore: phase.score,
      coverage: lattice.length / (35 * 51),
    };
  }
  function traceParallels(points, pitch) {
    // Rows and columns are parallel in the *flat sheet*, not in the image.
    // Trace each family in its measured lattice order. A local quadratic
    // estimates its position and tangent without straightening a curved row
    // prematurely. Occluded runs remain gaps, never synthetic observations.
    const curves = [],
      samples = [];
    for (const axis of ["row", "col"]) {
      const along = axis === "row" ? "col" : "row",
        groups = new Map();
      for (const point of points) {
        if (!groups.has(point[axis])) groups.set(point[axis], []);
        groups.get(point[axis]).push(point);
      }
      for (const [index, ordered] of groups) {
        ordered.sort((a, b) => a[along] - b[along]);
        let runs = [],
          run = [];
        for (const point of ordered) {
          if (run.length && point[along] - run.at(-1)[along] > 2) {
            runs.push(run);
            run = [];
          }
          run.push(point);
        }
        runs.push(run);
        for (const run of runs) {
          if (run.length < 7 || run.at(-1)[along] - run[0][along] < 6) continue;
          const curve = {
            axis,
            index,
            from: run[0][along],
            to: run.at(-1)[along],
            count: run.length,
            samples: [],
          };
          for (let i = 3; i < run.length - 3; i += 4) {
            const center = run[i],
              local = run.slice(i - 3, i + 4);
            if (local.at(-1)[along] - local[0][along] > 10) continue;
            let weights = local.map(() => 1),
              cx,
              cy,
              errors;
            for (let iteration = 0; iteration < 3; iteration++) {
              const matrix = Array.from({ length: 3 }, () => [0, 0, 0]),
                bx = [0, 0, 0],
                by = [0, 0, 0];
              local.forEach((point, j) => {
                const t = point[along] - center[along],
                  basis = [1, t, t * t],
                  weight = weights[j];
                for (let a = 0; a < 3; a++) {
                  bx[a] += basis[a] * point.x * weight;
                  by[a] += basis[a] * point.y * weight;
                  for (let b = 0; b < 3; b++)
                    matrix[a][b] += basis[a] * basis[b] * weight;
                }
              });
              cx = solve(matrix, bx);
              cy = solve(matrix, by);
              if (!cx || !cy) break;
              errors = local.map((point) => {
                const t = point[along] - center[along];
                return Math.hypot(
                  cx[0] + cx[1] * t + cx[2] * t * t - point.x,
                  cy[0] + cy[1] * t + cy[2] * t * t - point.y,
                );
              });
              const threshold = Math.max(pitch * 0.035, median(errors) * 2);
              weights = errors.map((error) =>
                Math.min(1, threshold / Math.max(1e-6, error)),
              );
            }
            if (!cx || !cy || percentile(errors, 0.8) > pitch * 0.13) continue;
            const length = Math.hypot(cx[1], cy[1]);
            if (length < pitch * 0.4 || length > pitch * 1.8) continue;
            const sample = {
              u: center.u,
              v: center.v,
              x: cx[0],
              y: cy[0],
              tx: cx[1],
              ty: cy[1],
              du: axis === "row" ? 5 / W : 0,
              dv: axis === "col" ? 5 / H : 0,
            };
            curve.samples.push(sample);
          }
          if (curve.samples.length) curves.push(curve);
        }
      }
    }
    // Balance the two families and the whole page, independent of traversal
    // order or how many dots survived handwriting in one particular row.
    for (const axis of ["row", "col"]) {
      const family = curves
        .filter((curve) => curve.axis === axis)
        .sort((a, b) => a.index - b.index);
      const candidates = family.flatMap((curve) => curve.samples);
      const count = Math.min(40, candidates.length);
      for (let i = 0; i < count; i++)
        samples.push(
          candidates[Math.floor(((i + 0.5) * candidates.length) / count)],
        );
    }
    return { curves, samples };
  }
  function residual(p, points, width, height, frame, guides = []) {
    const middle = (p.length - 8) / 2,
      a = [...p.slice(8, 8 + middle), 0, ...p.slice(8 + middle)],
      smoothCount = a.length - 2;

    const model = prepare(p, width, height),
      out = new Float64Array(
        points.length * 2 + smoothCount + (frame ? 200 : 0) + guides.length * 4,
      );
    points.forEach((point, i) => {
      const predicted = project(model, point.u, point.v);
      out[i * 2] = predicted.x - point.x;
      out[i * 2 + 1] = predicted.y - point.y;
    });
    for (let i = 0; i < smoothCount; i++)
      out[points.length * 2 + i] = (a[i] - 2 * a[i + 1] + a[i + 2]) * 0.5;
    if (frame) {
      let offset = points.length * 2 + smoothCount;
      for (const name of ["top", "right", "bottom", "left"])
        for (let i = 0; i <= 24; i++) {
          const t = i / 24,
            u =
              name === "left"
                ? 15 / W
                : name === "right"
                  ? 195 / W
                  : (15 + 180 * t) / W,
            v =
              name === "top"
                ? 14.3 / H
                : name === "bottom"
                  ? 284.3 / H
                  : (14.3 + 270 * t) / H;
          const point = project(model, u, v),
            path = frame.paths[name];
          let best = Infinity,
            nearest = path[0];
          for (let j = 0; j < path.length - 1; j++) {
            const a = path[j],
              b = path[j + 1],
              dx = b.x - a.x,
              dy = b.y - a.y,
              f = clamp(
                ((point.x - a.x) * dx + (point.y - a.y) * dy) /
                  (dx * dx + dy * dy || 1),
                0,
                1,
              );
            const q = { x: a.x + f * dx, y: a.y + f * dy },
              distance = (q.x - point.x) ** 2 + (q.y - point.y) ** 2;
            if (distance < best) {
              best = distance;
              nearest = q;
            }
          }
          out[offset++] = (point.x - nearest.x) * 0.35;
          out[offset++] = (point.y - nearest.y) * 0.35;
        }
    }
    let offset = points.length * 2 + smoothCount + (frame ? 200 : 0);
    for (const guide of guides) {
      const center = project(model, guide.u, guide.v),
        before = project(
          model,
          guide.u - guide.du * 0.5,
          guide.v - guide.dv * 0.5,
        ),
        after = project(
          model,
          guide.u + guide.du * 0.5,
          guide.v + guide.dv * 0.5,
        );
      // These are 3D camera/surface residuals, not a residual 2D rubber warp.
      // The shared isometric sheet must explain both parallel curve families.
      out[offset++] = (center.x - guide.x) * 0.65;
      out[offset++] = (center.y - guide.y) * 0.65;
      out[offset++] = (after.x - before.x - guide.tx) * 1.7;
      out[offset++] = (after.y - before.y - guide.ty) * 1.7;
    }
    return out;
  }
  const loss = (values) =>
    values.reduce((sum, v) => sum + 2 * (Math.sqrt(1 + v * v) - 1), 0);
  function surfaceErrors(model, points, frame) {
    const errors = points.map((point) => {
        const q = project(model, point.u, point.v);
        return Math.hypot(q.x - point.x, q.y - point.y);
      }),
      borders = [];
    for (const name of ["top", "right", "bottom", "left"])
      for (let i = 0; i <= 24; i++) {
        const t = i / 24,
          u =
            name === "left"
              ? 15 / W
              : name === "right"
                ? 195 / W
                : (15 + 180 * t) / W,
          v =
            name === "top"
              ? 14.3 / H
              : name === "bottom"
                ? 284.3 / H
                : (14.3 + 270 * t) / H;
        borders.push(lineDistance(project(model, u, v), frame.paths[name]));
      }
    return {
      rms: Math.sqrt(errors.reduce((sum, v) => sum + v * v, 0) / errors.length),
      p90: percentile(errors, 0.9),
      p99: percentile(errors, 0.99),
      borderMedian: median(borders),
      borderP95: percentile(borders, 0.95),
    };
  }
  function validErrors(errors, pitch) {
    return (
      errors.rms <= pitch * 0.14 &&
      errors.p90 <= pitch * 0.22 &&
      errors.p99 <= pitch * 0.4 &&
      errors.borderMedian <= pitch * 0.6 &&
      errors.borderP95 <= pitch * 1.15
    );
  }
  function visibleSurface(model) {
    let sign = 0;
    for (let y = 0; y <= 16; y++)
      for (let x = 0; x <= 12; x++) {
        const du = x === 12 ? -0.0001 : 0.0001,
          dv = y === 16 ? -0.0001 : 0.0001;
        const p = project(model, x / 12, y / 16),
          a = project(model, x / 12 + du, y / 16),
          b = project(model, x / 12, y / 16 + dv),
          jac =
            ((a.x - p.x) * (b.y - p.y) - (a.y - p.y) * (b.x - p.x)) / (du * dv);
        if (
          !Number.isFinite(jac) ||
          Math.abs(jac) < 1e-6 ||
          (sign && Math.sign(jac) !== sign)
        )
          return false;
        sign = Math.sign(jac);
      }
    return true;
  }
  function optimize(
    start,
    points,
    width,
    height,
    deadline,
    frame,
    guides = [],
    maxIterations = 48,
    preserveEnds = false,
  ) {
    let p = [...start],
      r = residual(p, points, width, height, frame, guides),
      cost = loss(r),
      lambda = 0.01,
      iterations = 0;
    for (
      ;
      iterations < maxIterations && performance.now() < deadline;
      iterations++
    ) {
      const jacobian = [];
      for (let j = 0; j < p.length; j++) {
        if (preserveEnds && (j === 8 || j === p.length - 1)) {
          jacobian.push(new Float64Array(r.length));
          continue;
        }
        const perturbed = [...p],
          epsilon = 1e-4;
        perturbed[j] += epsilon;
        const next = residual(perturbed, points, width, height, frame, guides);
        jacobian.push(next.map((v, i) => (v - r[i]) / epsilon));
      }
      const a = Array.from({ length: p.length }, () => Array(p.length).fill(0)),
        b = Array(p.length).fill(0);
      for (let i = 0; i < r.length; i++) {
        const weight = 1 / Math.sqrt(1 + r[i] * r[i]);
        for (let j = 0; j < p.length; j++) {
          const v = jacobian[j][i] * weight;
          b[j] -= v * r[i];
          for (let k = 0; k <= j; k++) a[j][k] += v * jacobian[k][i];
        }
      }
      for (let j = 0; j < p.length; j++) {
        for (let k = 0; k < j; k++) a[k][j] = a[j][k];
        a[j][j] += lambda * Math.max(1e-6, a[j][j]);
      }
      const delta = solve(a, b);
      if (!delta) break;
      const trial = p.map((v, i) => v + clamp(delta[i], -0.4, 0.4));
      trial[3] = clamp(trial[3], -5, 5);
      trial[4] = clamp(trial[4], -5, 5);
      trial[5] = clamp(trial[5], Math.log(120), Math.log(4000));
      trial[6] = clamp(
        trial[6],
        Math.log(Math.max(width, height) * 0.5),
        Math.log(Math.max(width, height) * 5),
      );
      for (let j = 8; j < p.length; j++) trial[j] = clamp(trial[j], -1.5, 1.5);
      const next = residual(trial, points, width, height, frame, guides),
        newCost = loss(next);
      if (newCost < cost) {
        const improvement = cost - newCost;
        p = trial;
        r = next;
        cost = newCost;
        lambda = Math.max(1e-7, lambda * 0.35);
        if (improvement < 1e-5) break;
      } else lambda = Math.min(1e8, lambda * 5);
    }

    return { p, cost, iterations };
  }
  function fit(source, detection, options = {}) {
    const diagnostics = options.diagnostics || {};
    const started = performance.now(),
      deadline = started + (options.budgetMs || 2600),
      frame = detection?.frame;
    if (!frame?.paths || frame.confidence < 0.65) return null;
    const observed = observe(source, frame, diagnostics);
    if (!observed) return null;
    const { points, pitch } = observed,
      quad = [
        frame.paths.top[0],
        frame.paths.top.at(-1),
        frame.paths.bottom.at(-1),
        frame.paths.bottom[0],
      ];
    const base = initial(quad, source.width, source.height);
    if (!base) return null;
    // Reserve full measured rows and columns for curve tracing. Intervening
    // dots remain independent holdouts: a denoised guide never incorporates
    // their coordinates, even where the two printed line families cross.
    const forTraining = (point) => point.row % 4 === 1 || point.col % 4 === 2,
      observedTraining = points.filter(forTraining),
      validation = points.filter((point) => !forTraining(point)),
      ordered = [...observedTraining].sort(
        (a, b) => a.row - b.row || a.col - b.col,
      ),
      trainingCount = Math.min(260, ordered.length),
      training = Array.from(
        { length: trainingCount },
        (_, i) =>
          ordered[Math.floor(((i + 0.5) * ordered.length) / trainingCount)],
      ),
      parallels = traceParallels(observedTraining, pitch),
      guides = parallels.samples,
      heldoutGuides = traceParallels(validation, pitch).samples;
    const tangentP90For = (model) => {
      const errors = heldoutGuides.map((guide) => {
        const before = project(
            model,
            guide.u - guide.du * 0.5,
            guide.v - guide.dv * 0.5,
          ),
          after = project(
            model,
            guide.u + guide.du * 0.5,
            guide.v + guide.dv * 0.5,
          );
        return Math.hypot(
          after.x - before.x - guide.tx,
          after.y - before.y - guide.ty,
        );
      });
      return errors.length ? percentile(errors, 0.9) : null;
    };
    const validateModel = (model) =>
      validErrors(surfaceErrors(model, validation, frame), pitch) &&
      visibleSurface(model) &&
      (tangentP90For(model) ?? 0) <= pitch * 0.22;
    diagnostics.parallels = {
      rows: new Set(
        parallels.curves
          .filter((curve) => curve.axis === "row")
          .map((curve) => curve.index),
      ).size,
      columns: new Set(
        parallels.curves
          .filter((curve) => curve.axis === "col")
          .map((curve) => curve.index),
      ).size,
      samples: guides.length,
    };
    let best = null;
    for (const beta of [0, Math.PI / 2, Math.PI / 4, -Math.PI / 4]) {
      if (performance.now() > deadline) break;
      const p = [...base];
      p[7] = beta;
      for (let i = 0; i < 8; i++)
        p[8 + i] = (((i < 4 ? i : i + 1) - 4) / 4) * 0.25;
      const result = optimize(
        p,
        training,
        source.width,
        source.height,
        deadline,
        frame,
        guides,
      );
      if (!best || result.cost < best.cost) best = result;
    }
    if (!best) return null;
    const coarseModel = prepare(best.p, source.width, source.height),
      coarseValid = validateModel(coarseModel);
    // A finer *3D* tangent profile is useful when traced parallels show a
    // smooth S bend that the initial nine knots cannot represent precisely.
    // Start it at the coarse physical solution; it cannot invent a 2D mesh.
    if (guides.length >= 32 && performance.now() < deadline - 180) {
      const coarseAngles = [...best.p.slice(8, 12), 0, ...best.p.slice(12)],
        refined = [...best.p.slice(0, 8)];
      for (let i = 0; i < 13; i++) {
        if (i === 6) continue;
        const at = (i * 8) / 12,
          index = Math.min(7, Math.floor(at)),
          fraction = at - index;
        refined.push(
          coarseAngles[index] * (1 - fraction) +
            coarseAngles[index + 1] * fraction,
        );
      }
      for (const preserveEnds of [false, true]) {
        if (performance.now() >= deadline) break;
        const candidate = optimize(
            refined,
            training,
            source.width,
            source.height,
            deadline,
            frame,
            guides,
            20,
            preserveEnds,
          ),
          model = prepare(candidate.p, source.width, source.height);
        // Blank outer margins do not contain dot evidence. First allow the
        // observed perimeter to refine them; if this folds an unobserved
        // margin, try retaining the coarse endpoint tangents while improving
        // only the interior. Both hypotheses face identical validation gates.
        if (
          (!coarseValid || candidate.cost < best.cost * 0.985) &&
          validateModel(model)
        ) {
          best = candidate;
          break;
        }
      }
    }
    const model = prepare(best.p, source.width, source.height),
      errors = surfaceErrors(model, validation, frame),
      { rms, p90, p99, borderP95 } = errors;
    Object.assign(diagnostics, errors, { parameters: best.p });
    if (!validErrors(errors, pitch) || !visibleSurface(model)) return null;
    // The held-out parallel curves use only held-out dots as well. Their
    // measured tangent verifies local direction, independently of training.
    const tangentP90 = tangentP90For(model);
    if (tangentP90 !== null && tangentP90 > pitch * 0.22) return null;
    diagnostics.parallelValidation = {
      samples: heldoutGuides.length,
      tangentP90,
    };
    return {
      type: "developable-pinhole",
      parameters: best.p,
      width: source.width,
      height: source.height,
      diagnostics: {
        observations: points.length,
        training: observedTraining.length,
        positionalSamples: training.length,
        validation: validation.length,
        parallels: diagnostics.parallels,
        parallelValidation: diagnostics.parallelValidation,
        profileKnots: best.p.length - 7,
        coverage: observed.coverage,
        phaseScore: observed.phaseScore,
        rms,
        p90,
        p99,
        borderP95,
        pitch,
        elapsedMs: performance.now() - started,
        iterations: best.iterations,
      },
      profile: {
        arcLengthMm: 2 * model.length,
        depthRangeMm: Math.max(...model.zs) - Math.min(...model.zs),
        bendAxisRadians: best.p[7],
      },
    };
  }
  const cache = new WeakMap();
  function map(surface, u, v) {
    let model = cache.get(surface);
    if (!model) {
      model = prepare(surface.parameters, surface.width, surface.height);
      cache.set(surface, model);
    }
    return project(model, u, v);
  }
  SP.MetricSurface = { fit, map, observe };
})();
