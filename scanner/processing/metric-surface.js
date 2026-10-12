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
    KNOTS = 9,
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
    const angles = [...p.slice(8, 12), 0, ...p.slice(12)],
      xs = [0],
      zs = [0];
    let previous = angles[0];
    for (let i = 1; i <= STEPS; i++) {
      const t = (i / STEPS) * (KNOTS - 1),
        index = Math.min(KNOTS - 2, Math.floor(t));
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
          highest = Math.max(...channels),
          lowest = Math.min(...channels);
        const background = [0, 0, 0];
        for (let dy = -3; dy <= 3; dy++)
          for (let dx = -3; dx <= 3; dx++) {
            const at = ((y + dy) * width + x + dx) * 4;
            for (let c = 0; c < 3; c++) background[c] += rgba[at + c] / 49;
          }
        const contrast = background.map((value, c) => value - channels[c]);
        if (
          highest < 45 ||
          (highest - lowest) / Math.max(1, highest) > 0.32 ||
          Math.max(...contrast) - Math.min(...contrast) >
            Math.max(...contrast) * 0.45 + 3
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
    diagnostics.phases = phases
      .slice(0, 3)
      .map((p) => ({
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
    return {
      points: phase.selected.map((p) => ({
        u: (20 + 5 * (p.col - phase.c0)) / W,
        v: (19.3 + 5 * (p.row - phase.r0)) / H,
        x: p.x / scale,
        y: p.y / scale,
      })),
      pitch: pitch / scale,
      phaseScore: phase.score,
      coverage: phase.selected.length / (35 * 51),
    };
  }
  function residual(p, points, width, height, frame) {
    const model = prepare(p, width, height),
      out = new Float64Array(points.length * 2 + 7 + (frame ? 200 : 0));
    points.forEach((point, i) => {
      const predicted = project(model, point.u, point.v);
      out[i * 2] = predicted.x - point.x;
      out[i * 2 + 1] = predicted.y - point.y;
    });
    const a = [...p.slice(8, 12), 0, ...p.slice(12)];
    for (let i = 0; i < 7; i++)
      out[points.length * 2 + i] = (a[i] - 2 * a[i + 1] + a[i + 2]) * 0.5;
    if (frame) {
      let offset = points.length * 2 + 7;
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
    return out;
  }
  const loss = (values) =>
    values.reduce((sum, v) => sum + 2 * (Math.sqrt(1 + v * v) - 1), 0);
  function optimize(start, points, width, height, deadline, frame) {
    let p = [...start],
      r = residual(p, points, width, height, frame),
      cost = loss(r),
      lambda = 0.01,
      iterations = 0;
    for (; iterations < 48 && performance.now() < deadline; iterations++) {
      const jacobian = [];
      for (let j = 0; j < p.length; j++) {
        const perturbed = [...p],
          epsilon = 1e-4;
        perturbed[j] += epsilon;
        const next = residual(perturbed, points, width, height, frame);
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
      const next = residual(trial, points, width, height, frame),
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
    // A uniform subset of the lattice is training data. The remaining points are
    // independent observations and never participate in camera/shape fitting.
    const stride = Math.max(4, Math.ceil(points.length / 320)),
      training = points.filter((_, i) => i % stride === 0),
      validation = points.filter((_, i) => i % stride !== 0);
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
      );
      if (!best || result.cost < best.cost) best = result;
    }
    if (!best) return null;
    const model = prepare(best.p, source.width, source.height),
      errors = validation.map((p) => {
        const q = project(model, p.u, p.v);
        return Math.hypot(q.x - p.x, q.y - p.y);
      });
    const rms = Math.sqrt(
        errors.reduce((sum, v) => sum + v * v, 0) / errors.length,
      ),
      p90 = percentile(errors, 0.9),
      p99 = percentile(errors, 0.99);
    Object.assign(diagnostics, { rms, p90, p99, parameters: best.p });
    if (rms > pitch * 0.14 || p90 > pitch * 0.22 || p99 > pitch * 0.4)
      return null;
    const borders = [];
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
    diagnostics.borderP95 = percentile(borders, 0.95);
    if (
      median(borders) > pitch * 0.6 ||
      percentile(borders, 0.95) > pitch * 1.15
    )
      return null;
    // A visible smooth sheet must preserve orientation everywhere. Reject
    // self-occluding fits instead of sampling a reflected or folded surface.
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
          return null;
        sign = Math.sign(jac);
      }
    return {
      type: "developable-pinhole",
      parameters: best.p,
      width: source.width,
      height: source.height,
      diagnostics: {
        observations: points.length,
        training: training.length,
        validation: validation.length,
        coverage: observed.coverage,
        phaseScore: observed.phaseScore,
        rms,
        p90,
        p99,
        borderP95: percentile(borders, 0.95),
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
