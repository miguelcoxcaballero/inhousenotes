(() => {
  "use strict";
  const SP = window.ScannerPro = window.ScannerPro || {};
  const clamp = (value, low, high) => Math.max(low, Math.min(high, value));
  const mix = (a, b, t) => ({ x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t });
  function sample(path, t) {
    const position = clamp(t, 0, 1) * (path.length - 1);
    const index = Math.min(path.length - 2, Math.floor(position));
    return mix(path[index], path[index + 1], position - index);
  }
  function perspective(quad) {
    const [a, b, c, d] = quad;
    const dx1 = b.x - c.x, dx2 = d.x - c.x;
    const dy1 = b.y - c.y, dy2 = d.y - c.y;
    const dx3 = a.x - b.x + c.x - d.x, dy3 = a.y - b.y + c.y - d.y;
    const determinant = dx1 * dy2 - dx2 * dy1;
    if (Math.abs(determinant) < 1e-6) return null;
    const g = (dx3 * dy2 - dx2 * dy3) / determinant;
    const h = (dx1 * dy3 - dx3 * dy1) / determinant;
    if (!Number.isFinite(g + h) || Math.min(1 + g, 1 + h, 1 + g + h) < .3) return null;
    return { g, h };
  }
  const horizontal = (t, v, g, h) => t * (1 + g + h * v) / (1 + g * t + h * v);
  const vertical = (t, u, g, h) => t * (1 + g * u + h) / (1 + g * u + h * t);
  function markerAnchors(frame, rgbAt) {
    const count = 2048, signal = [], red = [], blue = [], green = [];
    for (let i = 0; i <= count; i++) {
      const t = i / count, top = sample(frame.box.top, t), bottom = sample(frame.box.bottom, t);
      const strengths = [.27, .38, .5, .62, .73].map(amount => {
        const [r, g, b] = rgbAt(mix(top, bottom, amount));
        return Math.max(0, (r + g) * .5 - b - Math.abs(r - g) * .4);
      }).sort((a, b) => a - b);
      signal.push(strengths[1]);
      const colors = [.2, .3, .4, .5, .6, .7, .8].map(amount => rgbAt(mix(top, bottom, amount)));
      red.push(Math.max(...colors.map(([r, g, b]) => r - Math.max(g, b))));
      blue.push(Math.max(...colors.map(([r, g, b]) => b - Math.max(r, g))));
      green.push(Math.max(...colors.map(([r, g, b]) => g - Math.max(r, b))));
    }
    const colorCenter = values => {
      let best = -Infinity, index = 0;
      for (let i = Math.floor(count * .3); i <= Math.ceil(count * .7); i++) {
        if (values[i] > best) { best = values[i]; index = i; }
      }
      if (best < 18) return null;
      let total = 0, sum = 0;
      for (let i = Math.max(0, index - 16); i <= Math.min(count, index + 16); i++) {
        const weight = Math.max(0, values[i] - best * .35);
        sum += weight * i / count; total += weight;
      }
      return sum / total;
    };
    const r = colorCenter(red), b = colorCenter(blue), g = colorCenter(green);
    if (r === null || b === null || g === null || !(r < b && b < g)) return null;
    const spacing = (g - r) / 1.5;
    if (spacing < .012 || spacing > .05 || Math.abs((b - r) / (g - r) - 2 / 3) > .16) return null;
    const threshold = Math.max(15, Math.max(...signal) * .2);
    const peaks = [];
    let start = -1;
    for (let i = 0; i <= signal.length; i++) {
      if (signal[i] > threshold) { if (start < 0) start = i; }
      else if (start >= 0) {
        const end = i - 1;
        let total = 0, sum = 0, maximum = 0;
        for (let j = start; j <= end; j++) {
          sum += signal[j] * j / count; total += signal[j]; maximum = Math.max(maximum, signal[j]);
        }
        if ((end - start) / count < spacing * .65 && maximum > threshold * 1.5) peaks.push(sum / total);
        start = -1;
      }
    }
    const anchors = [];
    // The coloured markers establish the absolute cell number. Counting peaks
    // from an arbitrary image edge can silently shift every anchor by one cell.
    for (const direction of [-1, 1]) {
      const target = (direction < 0 ? r : g) + direction * spacing * 1.25;
      const first = peaks.filter(t => Math.abs(t - target) < spacing * .38)
        .sort((a, b) => Math.abs(a - target) - Math.abs(b - target))[0];
      if (first === undefined) return null;
      let previous = first, stepSize = spacing;
      anchors.push({ u: (direction < 0 ? 16 : 20) / 36, t: first });
      for (let step = 1; step <= 15; step++) {
        const expected = previous + direction * stepSize;
        if (expected < -.003 || expected > 1.003) break;
        const next = peaks.filter(t => direction * (t - previous) > stepSize * .55
          && Math.abs(t - expected) < stepSize * .38)
          .sort((a, b) => Math.abs(a - expected) - Math.abs(b - expected))[0];
        if (next === undefined) { if (step === 15) break; return null; }
        const measured = Math.abs(next - previous);
        if (measured / stepSize < .72 || measured / stepSize > 1.28) return null;
        anchors.push({ u: ((direction < 0 ? 16 : 20) + direction * step) / 36, t: next });
        stepSize = measured * .8 + stepSize * .2;
        previous = next;
      }
    }
    anchors.push({ u: (101.25 - 15) / 180, t: r }, { u: (106.25 - 15) / 180, t: b }, { u: (108.75 - 15) / 180, t: g });
    anchors.sort((a, b) => a.u - b.u);
    return anchors.length >= 29 && anchors[0].u < .09 && anchors.at(-1).u > .91 ? anchors : null;
  }
  const cross = (a, b) => a.x * b.y - a.y * b.x;
  const subtract = (a, b) => ({ x: a.x - b.x, y: a.y - b.y });
  function verticalRuling(paths) {
    const left = subtract(paths.left.at(-1), paths.left[0]);
    const right = subtract(paths.right.at(-1), paths.right[0]);
    const denominator = cross(left, right);
    const vanishing = Math.abs(denominator) > .0001
      ? mix(paths.left[0], paths.left.at(-1), cross(subtract(paths.right[0], paths.left[0]), right) / denominator)
      : null;
    const direction = vanishing ? point => subtract(vanishing, point) : () => left;
    // Across a cylindrical or S-shaped sheet, upright printed lines share one
    // camera vanishing point. Curved side borders instead require the general
    // surface model; never force those observations onto straight rulings.
    for (const path of [paths.left, paths.right]) {
      const line = subtract(path.at(-1), path[0]);
      const length = Math.hypot(line.x, line.y);
      if (path.some(point => Math.abs(cross(subtract(point, path[0]), line)) / length > length * .004)) return null;
    }
    return (path, origin, expected) => {
      const ray = direction(origin);
      let best = null, error = Infinity;
      for (let i = 0; i < path.length - 1; i++) {
        const segment = subtract(path[i + 1], path[i]);
        const determinant = cross(segment, ray);
        if (Math.abs(determinant) < 1e-8) continue;
        const amount = cross(subtract(origin, path[i]), ray) / determinant;
        if (amount < -.001 || amount > 1.001) continue;
        const t = (i + amount) / (path.length - 1);
        if (Math.abs(t - expected) < error) {
          best = mix(path[i], path[i + 1], amount);
          error = Math.abs(t - expected);
        }
      }
      return best;
    };
  }

  function refine(source, detection) {
    const frame = detection?.frame;
    if (!frame?.paths || frame.parameterization || !(frame.box?.support >= .5)) return detection;
    const paths = frame.paths;
    if (![paths.top, paths.bottom, paths.left, paths.right, frame.box.top, frame.box.bottom]
      .every(path => path?.length >= 4 && path.every(point => Number.isFinite(point?.x + point?.y)))) return detection;
    const quad = [paths.top[0], paths.top.at(-1), paths.bottom.at(-1), paths.bottom[0]];
    const model = perspective(quad);
    if (!model) return detection;
    const scale = Math.min(1, 1200 / Math.max(source.width, source.height));
    const canvas = document.createElement("canvas");
    canvas.width = Math.round(source.width * scale);
    canvas.height = Math.round(source.height * scale);
    const context = canvas.getContext("2d", { willReadFrequently: true });
    context.drawImage(source, 0, 0, canvas.width, canvas.height);
    const image = context.getImageData(0, 0, canvas.width, canvas.height);
    const { data, width, height } = SP.LiveDetection?.normalizeIllumination?.(image) || image;
    const rgbAt = point => {
      const x = clamp(point.x * scale - .5, 0, width - 1.001), y = clamp(point.y * scale - .5, 0, height - 1.001);
      const ix = Math.floor(x), iy = Math.floor(y), fx = x - ix, fy = y - iy;
      const at = (iy * width + ix) * 4;
      return [0, 1, 2].map(channel =>
        (data[at + channel] * (1 - fx) + data[at + 4 + channel] * fx) * (1 - fy)
        + (data[at + width * 4 + channel] * (1 - fx) + data[at + width * 4 + 4 + channel] * fx) * fy);
    };
    const chroma = point => {
      const x = clamp(Math.round(point.x * scale), 0, width - 1);
      const y = clamp(Math.round(point.y * scale), 0, height - 1);
      const at = (y * width + x) * 4;
      const r = data[at], g = data[at + 1], b = data[at + 2];
      return Math.max(0, (r + g) * .5 - b - Math.abs(r - g) * .4);
    };
    const score = (g, h) => {
      let ticks = 0, count = 0;
      // The printed five-millimetre cells provide independent metric evidence.
      // Their vertical teeth must be yellower than the neighbouring cell centres.
      for (const start of [1, 19]) for (let index = start + 1; index < start + 16; index++) {
        const u = index / 36;
        const at = offset => {
          const amount = u + offset / 36;
          const top = sample(frame.box.top, horizontal(amount, 26 / 27, g, h));
          const bottom = sample(frame.box.bottom, horizontal(amount, 26.5 / 27, g, h));
          return (chroma(mix(top, bottom, .32)) + chroma(mix(top, bottom, .5)) + chroma(mix(top, bottom, .68))) / 3;
        };
        ticks += at(0) - (at(-.5) + at(.5)) * .5;
        count++;
      }
      // The known rail-to-border gap measures vertical foreshortening even when
      // camera yaw is small and horizontal teeth alone cannot distinguish it.
      let rails = 0, railCount = 0;
      for (let i = 0; i < 31; i++) {
        const u = .06 + i / 30 * .88;
        if (u > .46 && u < .54) continue;
        const top = sample(paths.top, horizontal(u, 0, g, h));
        const bottom = sample(paths.bottom, horizontal(u, 1, g, h));
        for (const v of [26 / 27, 26.5 / 27]) {
          const t = vertical(v, u, g, h);
          const point = mix(top, bottom, t);
          const offset = 2.2 / Math.max(1, Math.hypot(bottom.x - top.x, bottom.y - top.y));
          rails += chroma(point) - (chroma(mix(top, bottom, t - offset)) + chroma(mix(top, bottom, t + offset))) * .5;
          railCount++;
        }
      }
      return { ticks: ticks / count, rails: rails / railCount };
    };
    const linear = score(0, 0), projected = score(model.g, model.h);
    const ruling = verticalRuling(paths);
    const anchors = ruling ? markerAnchors(frame, rgbAt) : null;
    canvas.width = canvas.height = 0;
    const gapErrors = [];
    for (let i = 3; i < paths.top.length - 3; i++) {
      const u = i / (paths.top.length - 1);
      if (u > .40 && u < .60) continue;
      const top = paths.top[i], bottom = paths.bottom[i];
      const dx = bottom.x - top.x, dy = bottom.y - top.y;
      const squared = dx * dx + dy * dy;
      for (const [key, v] of [["top", 26 / 27], ["bottom", 26.5 / 27]]) {
        const rail = sample(frame.box[key], u);
        const measured = ((bottom.x - rail.x) * dx + (bottom.y - rail.y) * dy) / squared;
        gapErrors.push({ linear: Math.abs(measured - (1 - v)),
          projected: Math.abs(measured - (1 - vertical(v, u, model.g, model.h))) });
      }
    }
    const averageGap = key => gapErrors.reduce((sum, error) => sum + error[key], 0) / gapErrors.length;
    linear.gap = averageGap("linear");
    projected.gap = averageGap("projected");
    const tickGain = projected.ticks - linear.ticks;
    const railGain = projected.rails - linear.rails;
    const usePerspective = projected.gap < linear.gap - .0005 && projected.gap < linear.gap * .65
      || tickGain > 7 && projected.ticks > 8 && railGain > -8
      || railGain > 7 && projected.rails > 6 && tickGain > -5;
    frame.parameterization = { model: anchors ? "measured-surface" : usePerspective ? "projective-guides" : "observed-guides",
      ...model, linear, projected, anchors: anchors?.length || 0 };
    if (!usePerspective && !anchors) return detection;
    const g = usePerspective ? model.g : 0, h = usePerspective ? model.h : 0;
    const sourceBox = frame.box;
    const resample = (path, parameter) => Array.from({ length: 65 }, (_, i) => sample(path, parameter(i / 64)));
    frame.paths = {
      top: resample(paths.top, u => horizontal(u, 0, g, h)),
      bottom: resample(paths.bottom, u => horizontal(u, 1, g, h)),
      left: resample(paths.left, v => vertical(v, 0, g, h)),
      right: resample(paths.right, v => vertical(v, 1, g, h))
    };
    frame.box = { ...frame.box,
      top: resample(frame.box.top, u => horizontal(u, 26 / 27, g, h)),
      bottom: resample(frame.box.bottom, u => horizontal(u, 26.5 / 27, g, h)),
      outer: frame.paths.bottom
    };
    if (anchors) {
      // Anchor positions belong to the unmodified observed strips, whose ends
      // may not align with the page corners. Follow their camera rulings to
      // every guide rather than copying their numeric path parameter.
      const origins = anchors.map(anchor => ({ u: anchor.u, t: anchor.t,
        point: mix(sample(sourceBox.top, anchor.t), sample(sourceBox.bottom, anchor.t), .5) }));
      const measuredCurve = path => {
        const points = [{ u: 0, point: path[0] }, ...origins.map(anchor => ({ u: anchor.u,
          point: ruling(path, anchor.point, anchor.t) })), { u: 1, point: path.at(-1) }];
        if (points.some(anchor => !anchor.point)) return null;
        return Array.from({ length: 65 }, (_, index) => {
          const u = index / 64;
          const next = points.findIndex(anchor => anchor.u >= u);
          if (next <= 0) return points[0].point;
          const before = points[next - 1], after = points[next];
          return mix(before.point, after.point, (u - before.u) / (after.u - before.u));
        });
      };
      const top = measuredCurve(paths.top), bottom = measuredCurve(paths.bottom);
      const boxTop = measuredCurve(sourceBox.top), boxBottom = measuredCurve(sourceBox.bottom);
      if (top && bottom && boxTop && boxBottom) {
        frame.paths.top = top;
        frame.paths.bottom = bottom;
        frame.box.top = boxTop;
        frame.box.bottom = boxBottom;
        frame.box.outer = bottom;
      } else {
        frame.parameterization.model = usePerspective ? "projective-guides" : "observed-guides";
        frame.parameterization.anchors = 0;
      }
    }
    return detection;
  }
  SP.SurfaceParameterization = { refine };
})();
