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
    const point = {
      x: model.width / 2 + (model.focal * X) / Z,
      y: model.height / 2 + (model.focal * Y) / Z,
    };
    if (model.refinement) {
      const correction = refinementAt(model.refinement, u, v);
      point.x += correction[0];
      point.y += correction[1];
    }
    if (model.registration) {
      const correction = registrationAt(model.registration, u, v);
      point.x += correction[0]; point.y += correction[1];
    }
    return point;
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
      height = Math.round(source.height * scale),
      scaleX = width / source.width,
      scaleY = height / source.height;
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
        x: p.x * scaleX - 0.5,
        y: p.y * scaleY - 0.5,
      }));
    let calibrationTop =
      frame.box?.support >= 0.5 &&
      frame.box.top?.length >= 7 &&
      frame.box.top?.every((p) => Number.isFinite(p.x) && Number.isFinite(p.y))
        ? frame.box.top.map((p) => ({ x: p.x * scaleX - 0.5, y: p.y * scaleY - 0.5 }))
        : null;
    const pixel = (x, y, c) =>
      rgba[
        (clamp(Math.round(y), 0, height - 1) * width +
          clamp(Math.round(x), 0, width - 1)) *
          4 +
          c
      ];
    const visibleCalibrationRail = (point, nx, ny) => {
      const background = [0, 1, 2].map(
        (c) =>
          (pixel(point.x + nx * 4, point.y + ny * 4, c) +
            pixel(point.x - nx * 4, point.y - ny * 4, c)) /
          2,
      );
      for (let offset = -2; offset <= 2; offset++) {
        const darkness = background.map(
          (value, c) =>
            (value - pixel(point.x + nx * offset, point.y + ny * offset, c)) /
            Math.max(40, value),
        );
        if (
          darkness[2] > 0.02 &&
          darkness[2] - (darkness[0] + darkness[1]) / 2 > 0.015
        ) return true;
      }
      return false;
    };
    if (calibrationTop?.length >= 3) {
      let hits = 0,
        checked = 0;
      for (let i = 1; i < calibrationTop.length - 1; i++) {
        const a = calibrationTop[i - 1],
          point = calibrationTop[i],
          b = calibrationTop[i + 1],
          length = Math.hypot(b.x - a.x, b.y - a.y);
        if (length < 1) continue;
        const nx = -(b.y - a.y) / length,
          ny = (b.x - a.x) / length;
        checked++;
        if (visibleCalibrationRail(point, nx, ny)) hits++;
      }
      // Some frame hypotheses infer the box position from page proportions.
      // Only an actually visible coloured rail may censor grid observations.
      if (checked < 5 || hits / checked < 0.35) calibrationTop = null;
    }
    const aboveCalibration = (point) => {
      if (!calibrationTop || calibrationTop.length < 2) return true;
      let closest = Infinity,
        side = 0,
        nearest = null,
        normalX = 0,
        normalY = 0;
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
          nearest = { x: a.x + t * dx, y: a.y + t * dy };
          normalX = -dy / length;
          normalY = dx / length;
        }
      }
      // The calibration boxes contain sharp corners at the same 5 mm pitch
      // as the dots. JPEG ringing can make those corners look like extra grid
      // rows. The measured upper box rail is a physical exclusion boundary;
      // the last real dot row sits one full pitch above it.
      // A rail can be well observed overall while its interpolated end bends
      // over genuine dots. Require colour evidence at this particular segment
      // before using it to exclude a point, retaining the full last grid row.
      return side > pitch * 0.25 || !nearest
        || !visibleCalibrationRail(nearest, normalX, normalY);
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
      x: (p.x + 0.5) / scaleX,
      y: (p.y + 0.5) / scaleY,
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
    const refined = refineDotCenters(source, lattice, pitch / Math.sqrt(scaleX * scaleY), diagnostics);
    if (refined.length < 350) return null;
    return {
      points: refined,
      raster: refined.raster,
      pitch: pitch / Math.sqrt(scaleX * scaleY),
      phaseScore: phase.score,
      coverage: refined.length / (35 * 51),
    };
  }
  function refineDotCenters(source, points, pitch, diagnostics, existingRaster = null) {
    // Observe the blob on the source raster, not only on the 1000px graph
    // analysis. Canvas coordinates describe pixel edges, so every weighted
    // sample contributes its centre (index + 0.5) in original source units.
    const scale = Math.min(1, 2600 / Math.max(source.width, source.height)),
      width = Math.round(source.width * scale), height = Math.round(source.height * scale),
      sx = width / source.width, sy = height / source.height,
      canvas = typeof OffscreenCanvas === "function" ? new OffscreenCanvas(width, height) : document.createElement("canvas");
    canvas.width = width; canvas.height = height;
    const ctx = canvas.getContext("2d", { willReadFrequently: true });
    if (!existingRaster) ctx.drawImage(source, 0, 0, width, height);
    const data = existingRaster || ctx.getImageData(0, 0, width, height).data,
      radius = Math.max(2.8, Math.min(6, pitch * Math.sqrt(sx * sy) * 0.2)),
      ringRadius = Math.max(3, radius + 1.5), out = [];
    for (const point of points) {
      const cx = point.x * sx - 0.5, cy = point.y * sy - 0.5,
        background = [[], [], []];
      for (let k = 0; k < 16; k++) {
        const angle = k * Math.PI / 8,
          x = clamp(Math.round(cx + Math.cos(angle) * ringRadius), 0, width - 1),
          y = clamp(Math.round(cy + Math.sin(angle) * ringRadius), 0, height - 1), at = (y * width + x) * 4;
        for (let c = 0; c < 3; c++) background[c].push(data[at + c]);
      }
      const paper = background.map(values => median(values));
      let sum = 0, wx = 0, wy = 0, xx = 0, yy = 0, xy = 0, peak = 0, edge = 0, coloredMass = 0, coloredPeak = 0;
      const samples = [];
      for (let y = Math.floor(cy - radius); y <= Math.ceil(cy + radius); y++)
        for (let x = Math.floor(cx - radius); x <= Math.ceil(cx + radius); x++) {
          if (x < 0 || y < 0 || x >= width || y >= height || Math.hypot(x - cx, y - cy) > radius) continue;
          const at = (y * width + x) * 4,
            contrast = [0, 1, 2].map(c => (paper[c] - data[at + c]) / Math.max(40, paper[c])),
            low = Math.min(...contrast), high = Math.max(...contrast);
          // Neutral printed ink inherits local paper colour; coloured ink
          // and long strokes must not become exact registration anchors.
          const weight = low >= high * 0.36 - 0.012 && high - low <= high * 0.56 + 0.016
            ? Math.max(0, (contrast[0] + contrast[1] + contrast[2]) / 3) : 0;
          if (!weight && high > 0) { coloredMass += high; coloredPeak = Math.max(coloredPeak, high); }
          peak = Math.max(peak, weight);
          samples.push({ x, y, weight });
        }
      for (const sample of samples) {
        const weight = Math.max(0, sample.weight - Math.max(0.006, peak * 0.08));
        const dx = sample.x - cx, dy = sample.y - cy;
        sum += weight; wx += sample.x * weight; wy += sample.y * weight;
        xx += dx * dx * weight; yy += dy * dy * weight; xy += dx * dy * weight;
        if (Math.hypot(dx, dy) > radius * 0.8) edge += weight;
      }
      if (sum < 0.035 || peak < 0.018 || (coloredMass > sum * 0.35 && coloredPeak > peak * 0.6)) { out.push({ ...point, centerRefined: false }); continue; }
      const x = wx / sum, y = wy / sum, dx = x - cx, dy = y - cy,
        vx = xx / sum - dx * dx, vy = yy / sum - dy * dy, cov = xy / sum - dx * dy,
        large = (vx + vy + Math.hypot(vx - vy, 2 * cov)) / 2,
        small = (vx + vy - Math.hypot(vx - vy, 2 * cov)) / 2;
      if (Math.hypot(dx, dy) > radius * 0.45 || edge / sum > 0.19 ||
        large > Math.max(0.5, small) * 2.9 || large > radius * radius * 0.35) { out.push({ ...point, centerRefined: false }); continue; }
      const peakSample = samples.reduce((best, sample) => sample.weight > best.weight ? sample : best, samples[0]),
        coreRadius = Math.max(1.8, Math.min(4, pitch * Math.sqrt(sx * sy) * 0.14)),
        paperLuma = paper[0] * 0.2126 + paper[1] * 0.7152 + paper[2] * 0.0722;
      let coreWeight = 0, coreX = 0, coreY = 0;
      for (const sample of samples) {
        if (Math.hypot(sample.x - peakSample.x, sample.y - peakSample.y) > coreRadius) continue;
        const at = (sample.y * width + sample.x) * 4,
          luma = data[at] * 0.2126 + data[at + 1] * 0.7152 + data[at + 2] * 0.0722,
          weight = sample.weight > 0 ? Math.max(0, paperLuma - luma - Math.max(2, peak * paperLuma * 0.035)) : 0;
        coreWeight += weight; coreX += sample.x * weight; coreY += sample.y * weight;
      }
      let finalX = coreWeight ? coreX / coreWeight : x,
        finalY = coreWeight ? coreY / coreWeight : y;
      const coreSamples = samples.map(sample => {
        const at = (sample.y * width + sample.x) * 4;
        return { ...sample, darkness: paperLuma - (data[at] * 0.2126 + data[at + 1] * 0.7152 + data[at + 2] * 0.0722) };
      }), peakLuma = Math.max(...coreSamples.map(sample => sample.darkness)),
        softRadius = pitch * Math.sqrt(sx * sy) * 0.14;
      // Fractional pixel coverage removes integer window jumps. Iterating a
      // compact luminance centroid follows the printed core without letting
      // the surrounding JPEG halo or paper texture displace its anchor.
      for (let iteration = 0; iteration < 3; iteration++) {
        let mass = 0, mx = 0, my = 0;
        for (const sample of coreSamples) {
          if (sample.weight <= 0) continue;
          const fraction = clamp(softRadius + 0.5 - Math.hypot(sample.x - finalX, sample.y - finalY), 0, 1),
            weight = Math.max(0, sample.darkness - Math.max(2, peakLuma * 0.035)) * fraction;
          mass += weight; mx += sample.x * weight; my += sample.y * weight;
        }
        if (mass) { finalX = mx / mass; finalY = my / mass; }
      }
      out.push({ ...point, x: (finalX + 0.5) / sx, y: (finalY + 0.5) / sy, centerRefined: true, dotContrast: peak });
    }
    diagnostics.sourceRefinedDots = out.filter(p => p.centerRefined).length;
    diagnostics.ambiguousDotCenters = points.length - diagnostics.sourceRefinedDots;
    canvas.width = canvas.height = 0;
    out.raster = data;
    return out;
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
  function observationDetail(observed, source) {
    // These are measured dot centres, not resampled curves or inferred dots.
    // Group references keep the worker message small and preserve real gaps.
    const points = observed.points.map((point) => ({ ...point }));
    const families = {};
    for (const [name, axis, along] of [["rows", "row", "col"], ["columns", "col", "row"]]) {
      const groups = new Map();
      points.forEach((point, index) => {
        if (!groups.has(point[axis])) groups.set(point[axis], []);
        groups.get(point[axis]).push(index);
      });
      families[name] = [];
      for (const [index, group] of groups) {
        group.sort((a, b) => points[a][along] - points[b][along]);
        let run = [];
        for (const at of group) {
          if (run.length && points[at][along] - points[run.at(-1)][along] > 2) {
            if (run.length > 1) families[name].push({ index, points: run });
            run = [];
          }
          run.push(at);
        }
        if (run.length > 1) families[name].push({ index, points: run });
      }
    }
    return { width: source.width, height: source.height, pitch: observed.pitch, points, ...families };
  }

  function splineBasis(value, count) {
    const at = clamp(value, 0, 1) * (count - 1),
      cell = Math.floor(at), t = at - cell,
      weights = [(1 - t) ** 3 / 6, (3 * t ** 3 - 6 * t * t + 4) / 6,
        (-3 * t ** 3 + 3 * t * t + 3 * t + 1) / 6, t ** 3 / 6];
    const merged = new Map();
    for (let i = 0; i < 4; i++) {
      const index = clamp(cell + i - 1, 0, count - 1);
      merged.set(index, (merged.get(index) || 0) + weights[i]);
    }
    return [...merged];
  }
  function refinementBasis(field, u, v) {
    // Dots constrain the interior and observed rails anchor the perimeter.
    // Fade to the physical model at the outside paper edge, where no printed
    // observations constrain extrapolation through the blank margin.
    const fade = (at, low, high, outerLow, outerHigh) => {
      const t = at < low ? (at - outerLow) / (low - outerLow)
        : at > high ? (outerHigh - at) / (outerHigh - high) : 1;
      const x = clamp(t, 0, 1);
      return x * x * (3 - 2 * x);
    };
    const weight = fade(u, 15 / W, 195 / W, 0, 1) *
      fade(v, 14.3 / H, 284.3 / H, 0, 1);
    if (!weight) return [];
    const out = [], xs = splineBasis(u, field.cols), ys = splineBasis(v, field.rows);
    for (const [y, wy] of ys) for (const [x, wx] of xs)
      out.push([y * field.cols + x, wx * wy * weight]);
    return out;
  }
  function refinementAt(field, u, v) {
    let x = 0, y = 0;
    for (const [i, weight] of refinementBasis(field, u, v)) {
      x += field.dx[i] * weight;
      y += field.dy[i] * weight;
    }
    return [x, y];
  }
  function fitMeasuredRefinement(model, training, pitch, frame) {
    // A low-dimensional smooth residual accounts for lens/print error and
    // small local departures from a single developable bend. It never fits
    // held-out dots and cannot rescue a physically rejected sheet.
    const field = { cols: 9, rows: 13 }, count = field.cols * field.rows,
      matrix = Array.from({ length: count }, () => Array(count).fill(0)),
      bx = Array(count).fill(0), by = Array(count).fill(0);
    const penalty = (terms, weight) => {
      for (const [i, a] of terms) for (const [j, b] of terms)
        matrix[i][j] += a * b * weight;
    };
    const anchors = [];
    for (const name of ["top", "right", "bottom", "left"]) for (let i = 0; i <= 24; i++) {
      const t = i / 24,
        u = name === "left" ? 15 / W : name === "right" ? 195 / W : (15 + 180 * t) / W,
        v = name === "top" ? 14.3 / H : name === "bottom" ? 284.3 / H : (14.3 + 270 * t) / H,
        p = project(model, u, v), path = frame.paths[name];
      let nearest = null, distance = Infinity;
      for (let j = 0; j < path.length - 1; j++) {
        const a = path[j], b = path[j + 1], dx = b.x - a.x, dy = b.y - a.y,
          f = clamp(((p.x - a.x) * dx + (p.y - a.y) * dy) / (dx * dx + dy * dy || 1), 0, 1),
          q = { x: a.x + f * dx, y: a.y + f * dy }, d = Math.hypot(q.x - p.x, q.y - p.y);
        if (d < distance) { nearest = q; distance = d; }
      }
      // A noisy rail estimate far from an already validated physical fit
      // must not pull the dot field into an unobserved corner.
      if (nearest && distance <= pitch * 0.22) anchors.push({ ...nearest, u, v, weight: 0.4 });
    }
    for (const point of [...training, ...anchors]) {
      const q = project(model, point.u, point.v),
        rx = point.x - q.x, ry = point.y - q.y,
        weight = (point.weight || 1) * Math.min(1, pitch * 0.08 / Math.max(1e-6, Math.hypot(rx, ry))),
        basis = refinementBasis(field, point.u, point.v);
      for (const [i, a] of basis) {
        bx[i] += a * rx * weight;
        by[i] += a * ry * weight;
        for (const [j, b] of basis) matrix[i][j] += a * b * weight;
      }
    }
    for (let y = 0; y < field.rows; y++) for (let x = 0; x < field.cols; x++) {
      const i = y * field.cols + x;
      matrix[i][i] += x === 0 || x === field.cols - 1 || y === 0 || y === field.rows - 1 ? 0.5 : 0.025;
      if (x && x < field.cols - 1) penalty([[i - 1, 1], [i, -2], [i + 1, 1]], 0.12);
      if (y && y < field.rows - 1) penalty([[i - field.cols, 1], [i, -2], [i + field.cols, 1]], 0.12);
    }
    field.dx = solve(matrix, bx);
    field.dy = solve(matrix, by);
    return field.dx && field.dy ? field : null;
  }
  function boundRefinement(model, refined, pitch) {
    // Derive a single conservative gain from displacement and local scale,
    // without consulting any held-out dots. Back off the whole smooth field
    // rather than clipping individual control points or loosening a bound.
    let maximumDisplacement = 0, maximumDerivative = 0;
    for (let y = 0; y <= 48; y++) for (let x = 0; x <= 36; x++) {
      const u = x / 36, v = y / 48, step = 0.0001,
        p = project(model, u, v), q = project(refined, u, v);
      maximumDisplacement = Math.max(maximumDisplacement, Math.hypot(p.x - q.x, p.y - q.y));
      for (const [du, dv] of [[step, 0], [0, step]]) {
        const a = project(model, u + du, v + dv), b = project(refined, u + du, v + dv),
          length = Math.hypot(a.x - p.x, a.y - p.y);
        maximumDerivative = Math.max(maximumDerivative,
          Math.hypot((b.x - q.x) - (a.x - p.x), (b.y - q.y) - (a.y - p.y)) / Math.max(1e-9, length));
      }
    }
    const scale = Math.min(1, pitch * 0.22 / Math.max(1e-9, maximumDisplacement),
      0.08 / Math.max(1e-9, maximumDerivative));
    if (!Number.isFinite(scale) || scale < 0.35) return null;
    if (scale < 1) {
      refined.refinement.dx = refined.refinement.dx.map(v => v * scale);
      refined.refinement.dy = refined.refinement.dy.map(v => v * scale);
    }
    if (!visibleSurface(refined)) return null;
    return { scale, maximumDisplacement: maximumDisplacement * scale,
      maximumDerivative: maximumDerivative * scale };
  }
  function refineProjectedCenters(source, model, points, raster) {
    // Measure the continuous photographed spot in the local flat-page basis.
    // Bilinear source resampling here is the same image formation used by
    // the final warp; native integer-pixel moments alone jump for tiny JPEG
    // dots. This needs only small patches, never a second full-page warp.
    const scale = Math.min(1, 2600 / Math.max(source.width, source.height)),
      width = Math.round(source.width * scale), height = Math.round(source.height * scale),
      sx = width / source.width, sy = height / source.height;
    const luma = (sourceX, sourceY) => {
      const x = clamp(sourceX * sx - 0.5, 0, width - 1.001),
        y = clamp(sourceY * sy - 0.5, 0, height - 1.001),
        ix = Math.floor(x), iy = Math.floor(y), fx = x - ix, fy = y - iy, at = (iy * width + ix) * 4;
      const value = i => raster[i] * 0.2126 + raster[i + 1] * 0.7152 + raster[i + 2] * 0.0722;
      return (value(at) * (1 - fx) + value(at + 4) * fx) * (1 - fy) +
        (value(at + width * 4) * (1 - fx) + value(at + width * 4 + 4) * fx) * fy;
    };
    return points.map(point => {
      if (!point.centerRefined) return point;
      const left = project(model, point.u - 0.1 / W, point.v), right = project(model, point.u + 0.1 / W, point.v),
        top = project(model, point.u, point.v - 0.1 / H), bottom = project(model, point.u, point.v + 0.1 / H),
        ax = (right.x - left.x) / 0.2, ay = (right.y - left.y) / 0.2,
        bx = (bottom.x - top.x) / 0.2, by = (bottom.y - top.y) / 0.2,
        sample = (x, y) => luma(point.x + ax * x + bx * y, point.y + ay * x + by * y), ring = [];
      for (let i = 0; i < 24; i++) { const angle = i * Math.PI / 12; ring.push(sample(Math.cos(angle) * 1.8, Math.sin(angle) * 1.8)); }
      const paper = percentile(ring, 0.65);
      let peak = 0, px = 0, py = 0;
      for (let y = -1; y <= 1; y += 0.25) for (let x = -1; x <= 1; x += 0.25) {
        const dark = paper - sample(x, y);
        if (dark > peak) { peak = dark; px = x; py = y; }
      }
      if (peak < 5) return point;
      let mass = 0, mx = 0, my = 0;
      for (let y = -0.875; y <= 0.875; y += 0.125) for (let x = -0.875; x <= 0.875; x += 0.125) {
        const coverage = clamp((0.8 - Math.hypot(x, y)) / 0.125 + 0.5, 0, 1);
        if (!coverage) continue;
        const weight = Math.max(0, paper - sample(px + x, py + y) - Math.max(2, peak * 0.035)) * coverage;
        mass += weight; mx += (px + x) * weight; my += (py + y) * weight;
      }
      if (!mass) return point;
      mx /= mass; my /= mass;
      if (Math.hypot(mx, my) > 0.3) return point;
      return { ...point, x: point.x + ax * mx + bx * my, y: point.y + ay * mx + by * my };
    });
  }
  function registrationAt(field, u, v) {
    const locate = (positions, value) => {
      let low = 0, high = positions.length - 1;
      while (high - low > 1) { const middle = (low + high) >> 1; if (positions[middle] > value) high = middle; else low = middle; }
      return Math.min(positions.length - 2, low);
    };
    const x = locate(field.colPositions, u), y = locate(field.rowPositions, v),
      tx = clamp((u - field.colPositions[x]) / (field.colPositions[x + 1] - field.colPositions[x]), 0, 1),
      ty = clamp((v - field.rowPositions[y]) / (field.rowPositions[y + 1] - field.rowPositions[y]), 0, 1),
      n = field.colPositions.length, i = y * n + x;
    const value = values => (values[i] * (1 - tx) + values[i + 1] * tx) * (1 - ty) +
      (values[i + n] * (1 - tx) + values[i + n + 1] * tx) * ty;
    return [value(field.dx), value(field.dy)];
  }
  function registerMeasuredDots(model, points, pitch, options = {}) {
    // This final calibration uses all reliable real centres, after the physical
    // fit has passed independent holdouts. Its anchor error is an interpolation
    // metric, explicitly not another independent validation score.
    const field = {
      colPositions: [0, 15 / W, ...Array.from({ length: 35 }, (_, i) => (20 + i * 5) / W), 195 / W, 1],
      rowPositions: [0, 14.3 / H, ...Array.from({ length: 51 }, (_, i) => (19.3 + i * 5) / H), 284.3 / H, 1],
    }, cols = field.colPositions.length, rows = field.rowPositions.length,
      count = cols * rows, candidates = new Map();
    for (const point of points) {
      if (point.centerRefined === false) continue;
      const q = project(model, point.u, point.v), dx = point.x - q.x, dy = point.y - q.y;
      if (Math.hypot(dx, dy) > pitch * 0.3) continue;
      candidates.set((point.row + 2) * cols + point.col + 2, { point, dx, dy });
    }
    // Compare a candidate to both independently ordered dot families. A
    // punctuation fragment cannot become a sharp local warp control merely
    // because a smooth global fit tolerated its error in a long tail.
    // Raster controls have already passed this measured-centre check. Their
    // bounded photometric offset is validated by the cell-scale/no-fold tests
    // below, rather than being mistaken for a new noisy source observation.
    for (const [index, candidate] of options.rasterControls ? [] : [...candidates]) {
      const predictions = [];
      for (const step of [1, cols]) for (const distance of [1, 2]) {
        const a = candidates.get(index - distance * step), b = candidates.get(index + distance * step);
        if (a && b) predictions.push({ x: (a.dx + b.dx) / 2, y: (a.dy + b.dy) / 2 });
      }
      if (predictions.length >= 2 && Math.hypot(candidate.dx - median(predictions.map(p => p.x)),
        candidate.dy - median(predictions.map(p => p.y))) > Math.max(0.35, pitch * 0.06)) candidates.delete(index);
    }
    const minimum = Math.max(350, Math.floor(points.length * 0.35));
    if (candidates.size < minimum) return null;
    const fill = () => {
      field.dx = Array(count).fill(0); field.dy = Array(count).fill(0);
      const fixed = new Uint8Array(count);
      for (let y = 0; y < rows; y++) for (let x = 0; x < cols; x++)
        if (x <= 1 || x >= cols - 2 || y <= 1 || y >= rows - 2) fixed[y * cols + x] = 1;
      for (const [i, candidate] of candidates) { field.dx[i] = candidate.dx; field.dy[i] = candidate.dy; fixed[i] = 1; }
      // Only missing controls are interpolated. They are never reported as
      // detected dots or as accepted exact anchors in the progress overlay.
      for (let iteration = 0; iteration < 100; iteration++) {
        let change = 0;
        for (let y = 2; y < rows - 2; y++) for (let x = 2; x < cols - 2; x++) {
          const i = y * cols + x;
          if (fixed[i]) continue;
          for (const values of [field.dx, field.dy]) {
            const next = (values[i - 1] + values[i + 1] + values[i - cols] + values[i + cols]) / 4;
            change = Math.max(change, Math.abs(next - values[i])); values[i] = next;
          }
        }
        if (change < 1e-5) break;
      }
    };
    const sourceNodes = Array.from({ length: count }, (_, i) => project(model,
      field.colPositions[i % cols], field.rowPositions[Math.floor(i / cols)]));
    const badCells = () => {
      const bad = [];
      for (let y = 1; y < rows - 2; y++) for (let x = 1; x < cols - 2; x++) {
        const indices = [y * cols + x, y * cols + x + 1, (y + 1) * cols + x, (y + 1) * cols + x + 1];
        let failed = false;
        for (const [a, b] of [[0, 1], [2, 3], [0, 2], [1, 3]]) {
          const i = indices[a], j = indices[b], p = sourceNodes[i], q = sourceNodes[j],
            length = Math.hypot(q.x - p.x, q.y - p.y),
            shift = Math.hypot(field.dx[j] - field.dx[i], field.dy[j] - field.dy[i]);
          if (shift > length * 0.12) failed = true;
        }
        const p = indices.map(i => ({ x: sourceNodes[i].x + field.dx[i], y: sourceNodes[i].y + field.dy[i] }));
        for (const [a, b, c] of [[0, 1, 2], [1, 3, 0], [2, 0, 3], [3, 2, 1]]) {
          const determinant = (p[b].x - p[a].x) * (p[c].y - p[a].y) - (p[b].y - p[a].y) * (p[c].x - p[a].x),
            original = sourceNodes[indices[a]], one = sourceNodes[indices[b]], two = sourceNodes[indices[c]],
            expected = (one.x - original.x) * (two.y - original.y) - (one.y - original.y) * (two.x - original.x);
          if (!Number.isFinite(determinant) || determinant * expected <= 0) failed = true;
        }
        if (failed) bad.push(indices);
      }
      return bad;
    };
    for (let attempt = 0; attempt < 5; attempt++) {
      fill();
      const bad = badCells();
      if (!bad.length) {
        const anchors = [...candidates.values()].map(candidate => candidate.point),
          registered = { ...model, registration: field };
        if (!visibleSurface(registered)) return null;
        return { field, anchors, diagnostics: { anchors: anchors.length, rejected: points.length - anchors.length,
          interpolated: 35 * 51 - anchors.length, localScaleLimit: 0.12, maximumAnchorError: Math.max(...anchors.map(point => {
            const mapped = project(registered, point.u, point.v);
            return Math.hypot(mapped.x - point.x, mapped.y - point.y);
          })) } };
      }
      const remove = new Set();
      for (const cell of bad) {
        const ranked = cell.filter(i => candidates.has(i)).sort((a, b) =>
          Math.hypot(candidates.get(b).dx, candidates.get(b).dy) - Math.hypot(candidates.get(a).dx, candidates.get(a).dy));
        if (ranked.length) remove.add(ranked[0]);
      }
      if (!remove.size) return null;
      for (const i of remove) candidates.delete(i);
      if (candidates.size < minimum) return null;
    }
    return null;
  }
  function fit(source, detection, options = {}) {
    const diagnostics = options.diagnostics || {};
    const started = performance.now(),
      deadline = started + (options.budgetMs || 2600),
      frame = detection?.frame;
    if (!frame?.paths || frame.confidence < 0.65) return null;
    const observed = observe(source, frame, diagnostics);
    if (!observed) return null;
    if (typeof options.onObservations === "function")
      options.onObservations(observationDetail(observed, source));
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
    let best = null, starts = 0;
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
      starts++;
      if (!best || result.cost < best.cost) best = result;
      // A different bend-axis seed is unnecessary after an independently
      // checked subpixel fit already explains both measured line families.
      // Difficult pages still exhaust all starts and the original budget.
      const candidateModel = prepare(result.p, source.width, source.height),
        candidateErrors = surfaceErrors(candidateModel, validation, frame);
      if (candidateErrors.rms < pitch * 0.025 &&
        candidateErrors.p99 < pitch * 0.065 &&
        (tangentP90For(candidateModel) ?? Infinity) < pitch * 0.035 &&
        validErrors(candidateErrors, pitch) && visibleSurface(candidateModel)) break;
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
      const profileCandidates = [];
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
        profileCandidates.push(candidate);
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
      // The unconstrained endpoint solution can fold only its blank margin,
      // while fixing endpoints can underfit a genuine curl near the dots.
      // Interpolate the two physical tangent profiles (not 2D coordinates)
      // and retain a fully visible, independently validated compromise.
      if (profileCandidates.length === 2) {
        for (const mix of [0.25, 0.5, 0.75]) {
          const p = profileCandidates[0].p.map((value, index) =>
            value * (1 - mix) + profileCandidates[1].p[index] * mix),
            candidateModel = prepare(p, source.width, source.height),
            cost = loss(residual(p, training, source.width, source.height, frame, guides));
          if ((!validateModel(prepare(best.p, source.width, source.height)) || cost < best.cost * 0.985) && validateModel(candidateModel))
            best = { p, cost, iterations: 0 };
        }
      }
    }
    let model = prepare(best.p, source.width, source.height),
      errors = surfaceErrors(model, validation, frame);
    Object.assign(diagnostics, errors, { parameters: best.p });
    if (!validErrors(errors, pitch) || !visibleSurface(model)) return null;
    // The held-out parallel curves use only held-out dots as well. Their
    // measured tangent verifies local direction, independently of training.
    let tangentP90 = tangentP90For(model);
    if (tangentP90 !== null && tangentP90 > pitch * 0.22) return null;
    const baseErrors = errors,
      field = fitMeasuredRefinement(model, observedTraining, pitch, frame);
    let refinementBounds = null;
    if (field) {
      const refined = { ...model, refinement: field },
        bounds = boundRefinement(model, refined, pitch);
      if (bounds) {
        const refinedErrors = surfaceErrors(refined, validation, frame),
          refinedTangent = tangentP90For(refined);
        if (refinedErrors.rms < errors.rms * 0.88 &&
          refinedErrors.p90 < errors.p90 * 0.93 &&
          refinedErrors.p99 <= errors.p99 * 1.03 &&
          (refinedTangent ?? 0) <= Math.max((tangentP90 ?? 0) * 1.03, pitch * 0.025) &&
          validErrors(refinedErrors, pitch)) {
          model = refined;
          errors = refinedErrors;
          tangentP90 = refinedTangent;
          refinementBounds = bounds;
        }
      }
    }
    diagnostics.measuredRefinement = {
      accepted: !!model.refinement,
      training: observedTraining.length,
      validation: validation.length,
      beforeRMS: baseErrors.rms,
      afterRMS: errors.rms,
      controls: model.refinement ? model.refinement.cols * model.refinement.rows : 0,
      ...(refinementBounds || {}),
    };
    const { rms, p90, p99, borderP95 } = errors;
    Object.assign(diagnostics, errors);
    diagnostics.parallelValidation = {
      samples: heldoutGuides.length,
      tangentP90,
    };
    const present = new Set(points.map(point => `${point.row},${point.col}`)), missing = [];
    for (let row = 0; row < 51; row++) for (let col = 0; col < 35; col++) {
      if (present.has(`${row},${col}`)) continue;
      const u = (20 + col * 5) / W, v = (19.3 + row * 5) / H;
      missing.push({ ...project(model, u, v), u, v, row, col });
    }
    const typicalDotContrast = median(points.filter(point => point.centerRefined).map(point => point.dotContrast)),
      recovered = refineDotCenters(source, missing, pitch, {}, observed.raster).filter(point =>
        point.centerRefined && point.dotContrast < typicalDotContrast * 2.3 && (() => {
          const expected = project(model, point.u, point.v);
          return Math.hypot(expected.x - point.x, expected.y - point.y) <= Math.max(0.25, Math.min(pitch * 0.08, errors.p90 * 1.2));
        })()),
      registrationPoints = refineProjectedCenters(source, model, [...points, ...recovered], observed.raster),
      registration = registerMeasuredDots(model, registrationPoints, pitch);
    if (registration) {
      registration.diagnostics.recovered = registration.anchors.filter(point => !present.has(`${point.row},${point.col}`)).length;
      registration.field.points = registration.anchors;
    }
    if (registration && typeof options.onRegistration === "function")
      options.onRegistration(observationDetail({ ...observed, points: registration.anchors }, source));
    return {
      type: "developable-pinhole",
      parameters: best.p,
      ...(model.refinement ? { refinement: model.refinement } : {}),
      ...(registration ? { registration: registration.field } : {}),
      width: source.width,
      height: source.height,
      diagnostics: {
        observations: points.length,
        registration: registration?.diagnostics || { anchors: 0, rejected: points.length },
        training: observedTraining.length,
        positionalSamples: training.length,
        validation: validation.length,
        parallels: diagnostics.parallels,
        parallelValidation: diagnostics.parallelValidation,
        measuredRefinement: diagnostics.measuredRefinement,
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
        starts,
      },
      profile: {
        arcLengthMm: 2 * model.length,
        depthRangeMm: Math.max(...model.zs) - Math.min(...model.zs),
        bendAxisRadians: best.p[7],
      },
    };
  }
  const rasterCalibration = new WeakMap();
  function refineForRaster(source, surface, options = {}) {
    const prior = surface && rasterCalibration.get(surface), outputWidth = options.width, outputHeight = options.height,
      original = prior?.original || surface?.registration;
    if (!original?.points?.length || !outputWidth || !outputHeight) return null;
    if (prior?.width === outputWidth && prior?.height === outputHeight) return prior.detail;
    const started = performance.now();
    const sourceWidth = source.width, sourceHeight = source.height;
    let image = options.imageData;
    if (!image) {
      const canvas = typeof OffscreenCanvas === "function" ? new OffscreenCanvas(sourceWidth, sourceHeight) : document.createElement("canvas");
      canvas.width = sourceWidth; canvas.height = sourceHeight;
      const context = canvas.getContext("2d", { willReadFrequently: true });
      context.drawImage(source, 0, 0); image = context.getImageData(0, 0, sourceWidth, sourceHeight);
      canvas.width = canvas.height = 0;
    }
    const data = image.data, pitch = surface.diagnostics.pitch,
      field = { ...original, dx: [...original.dx], dy: [...original.dy] },
      model = prepare(surface.parameters, surface.width, surface.height);
    if (surface.refinement) model.refinement = surface.refinement;
    const coefficients = [0.2126, 0.7152, 0.0722];
    const luma = (x, y) => {
      x = clamp(x - 0.5, 0, sourceWidth - 1.001); y = clamp(y - 0.5, 0, sourceHeight - 1.001);
      const ix = Math.floor(x), iy = Math.floor(y), fx = x - ix, fy = y - iy, at = (iy * sourceWidth + ix) * 4;
      let out = 0;
      for (let c = 0; c < 3; c++) {
        const value = Math.round((data[at + c] * (1 - fx) + data[at + 4 + c] * fx) * (1 - fy) +
          (data[at + sourceWidth * 4 + c] * (1 - fx) + data[at + sourceWidth * 4 + 4 + c] * fx) * fy);
        out += value * coefficients[c];
      }
      return out;
    };
    const fractions = (count, extra) => [...Array.from({length:count+1}, (_,i)=>i/count), ...extra]
      .sort((a,b)=>a-b).filter((value,i,values)=>!i||value-values[i-1]>1e-7),
      us = options.mesh?.us || fractions(64,[15/W,195/W,...field.colPositions]),
      vs = options.mesh?.vs || fractions(80,[14.3/H,274.3/H,279.3/H,284.3/H,...field.rowPositions]),
      nodes = new Map(),
      node = (x,y) => { const key=y*us.length+x; let p=nodes.get(key);
        if(!p){p=project(model,us[x],vs[y]);nodes.set(key,p);} return p; },
      interval = (values,t) => {let low=0,high=values.length-1;while(high-low>1){const m=(low+high)>>1;if(values[m]<=t)low=m;else high=m;}return low;},
      baseAt = (u,v) => {const x=interval(us,u),y=interval(vs,v),s=(u-us[x])/(us[x+1]-us[x]),t=(v-vs[y])/(vs[y+1]-vs[y]),
        a=node(x,y),b=node(x+1,y),c=node(x,y+1),d=node(x+1,y+1);
        return {x:(a.x*(1-s)+b.x*s)*(1-t)+(c.x*(1-s)+d.x*s)*t,
          y:(a.y*(1-s)+b.y*s)*(1-t)+(c.y*(1-s)+d.y*s)*t};};
    const outputPitch = outputWidth * 5 / W, radius = Math.max(2, Math.ceil(outputPitch * 0.14)),
      search = Math.max(2, Math.ceil(outputPitch * 0.12)), plans = original.points.map(point => {
        const center = project(model, point.u, point.v),
          a = project(model, point.u + 1 / outputWidth, point.v),
          b = project(model, point.u, point.v + 1 / outputHeight);
        return { point, center, ax: a.x - center.x, ay: a.y - center.y, bx: b.x - center.x, by: b.y - center.y,
          tx: point.u * outputWidth, ty: point.v * outputHeight,
          index: (point.row + 2) * field.colPositions.length + point.col + 2 };
      });
    let maximumCorrection = 0;
    // Sample the same output-pixel phase, mesh and rounded RGB values as the
    // warp. Each calibration step must improve the visible glyph centroid;
    // retaining the best step avoids toggling between equal-valued peaks.
    for (let pass = 0; pass < 2; pass++) for (const plan of plans) {
      const { point, ax, ay, bx, by, tx, ty, index } = plan,
        samples = new Map();
      let generation = 0;
      const sample = (x, y) => {
          const key=y*outputWidth+x; let prepared=samples.get(key);
          if(!prepared){
            const u=(x+0.5)/outputWidth,v=(y+0.5)/outputHeight,
              col=interval(field.colPositions,u),row=interval(field.rowPositions,v),n=field.colPositions.length;
            prepared={base:baseAt(u,v),i:row*n+col,n,
              tx:clamp((u-field.colPositions[col])/(field.colPositions[col+1]-field.colPositions[col]),0,1),
              ty:clamp((v-field.rowPositions[row])/(field.rowPositions[row+1]-field.rowPositions[row]),0,1),generation:-1};
            samples.set(key,prepared);
          }
          if(prepared.generation===generation)return prepared.value;
          const {i,n,tx,ty}=prepared,
            correctionX=(field.dx[i]*(1-tx)+field.dx[i+1]*tx)*(1-ty)+(field.dx[i+n]*(1-tx)+field.dx[i+n+1]*tx)*ty,
            correctionY=(field.dy[i]*(1-tx)+field.dy[i+1]*tx)*(1-ty)+(field.dy[i+n]*(1-tx)+field.dy[i+n+1]*tx)*ty;
          prepared.value=luma(prepared.base.x+correctionX,prepared.base.y+correctionY);
          prepared.generation=generation;
          return prepared.value;
        },
        measure = () => {
          generation++;
          const ring = [];
          for (let i=0;i<24;i++){const angle=i*Math.PI/12;
            ring.push(sample(Math.round(tx-0.5+Math.cos(angle)*outputPitch*0.36),
              Math.round(ty-0.5+Math.sin(angle)*outputPitch*0.36)));}
          const paper=percentile(ring,0.65),cx=Math.round(tx-0.5),cy=Math.round(ty-0.5);
          let peak=0,px=cx,py=cy;
          for(let y=cy-search;y<=cy+search;y++)for(let x=cx-search;x<=cx+search;x++){
            const darkness=paper-sample(x,y);if(darkness>peak){peak=darkness;px=x;py=y;}}
          if(peak<5)return null;
          let mass=0,mx=0,my=0;
          for(let y=-radius;y<=radius;y++)for(let x=-radius;x<=radius;x++){
            if(x*x+y*y>radius*radius)continue;
            const weight=Math.max(0,paper-sample(px+x,py+y)-Math.max(2,peak*0.035));
            mass+=weight;mx+=(px+x+0.5)*weight;my+=(py+y+0.5)*weight;}
          if(!mass)return null;
          const dx=mx/mass-tx,dy=my/mass-ty;return {dx,dy,error:Math.hypot(dx,dy)};
        };
      let measured=measure();
      for(let iteration=0;measured && measured.error>0.025 && iteration<3;iteration++){
        if(measured.error>outputPitch*0.08)break;
        const oldX=field.dx[index],oldY=field.dy[index];
        let improved=false;
        for(const amount of [1,0.5,0.25]){
          const nextX=oldX+(ax*measured.dx+bx*measured.dy)*amount,
            nextY=oldY+(ay*measured.dx+by*measured.dy)*amount,
            correction=Math.hypot(nextX-original.dx[index],nextY-original.dy[index]);
          if(correction>pitch*0.08)continue;
          field.dx[index]=nextX;field.dy[index]=nextY;
          const candidate=measure();
          if(candidate && candidate.error<measured.error){
            measured=candidate;improved=true;maximumCorrection=Math.max(maximumCorrection,correction);break;}
          field.dx[index]=oldX;field.dy[index]=oldY;
        }
        if(!improved)break;
      }
    }
    const points = plans.map(({ point, center, index }) => ({ ...point,
      measuredX: point.measuredX ?? point.x, measuredY: point.measuredY ?? point.y,
      x: center.x + field.dx[index], y: center.y + field.dy[index] })),
      registered = registerMeasuredDots(model, points, pitch, {rasterControls:true});
    if (!registered) return null;
    registered.field.points = registered.anchors;
    surface.registration = registered.field;
    cache.delete(surface);
    surface.diagnostics.rasterRegistration = { ...registered.diagnostics,
      width: outputWidth, height: outputHeight, maximumSourceCorrection: maximumCorrection,
      elapsedMs: performance.now() - started };
    const detail = { ...observationDetail({ points: registered.anchors, pitch }, source),
      rasterAligned: true, registrationDiagnostics: surface.diagnostics.rasterRegistration };
    rasterCalibration.set(surface, { original, width: outputWidth, height: outputHeight, detail });
    return detail;
  }
  const cache = new WeakMap();
  function map(surface, u, v) {
    let model = cache.get(surface);
    if (!model) {
      model = prepare(surface.parameters, surface.width, surface.height);
      if (surface.refinement) model.refinement = surface.refinement;
      if (surface.registration) model.registration = surface.registration;
      cache.set(surface, model);
    }
    return project(model, u, v);
  }
  SP.MetricSurface = { fit, map, observe, refineForRaster };
})();
