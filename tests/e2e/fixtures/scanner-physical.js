(() => {
  'use strict';

  const PAGE = { width: 210, height: 297 };
  const FRAME = { u0: 15 / 210, u1: 195 / 210, v0: 14.3 / 297, v1: 284.3 / 297 };
  const LINES = [35, 75, 115, 155, 195, 235];
  const VERTICAL_LINES = [35, 75, 115, 155, 180];
  const CASES = [
    { name: 'flat-pinhole', radius: 0, pitch: .38, yaw: .46 },
    { name: 'pitch-only-pinhole', radius: 0, pitch: .4, yaw: 0 },
    { name: 'cylindrical-paper', radius: 170, pitch: .24, yaw: .32 },
    { name: 'frontal-cylinder', radius: 150, pitch: 0, yaw: 0 },
    { name: 's-bent-paper', sBend: .58, pitch: .18, yaw: -.24 }
  ];

  function cameraFor(definition) {
    // Paper coordinates use millimetres of ARC LENGTH. Unlike the older Coons
    // fixtures, these are actual developable 3D surfaces followed by a pinhole
    // camera. Perspective and foreshortening arise from depth, independently
    // of the production scanner's interpolation or detected mesh.
    const count = 2048;
    const crossSection = [{ x: 0, z: 0 }];
    if (definition.sBend) {
      for (let i = 1; i <= count; i += 1) {
        const angle = definition.sBend * Math.cos(2 * Math.PI * ((i - .5) / count));
        crossSection.push({
          x: crossSection[i - 1].x + Math.cos(angle) * PAGE.width / count,
          z: crossSection[i - 1].z + Math.sin(angle) * PAGE.width / count
        });
      }
    }
    return (u, v) => {
      let x = (u - .5) * PAGE.width;
      let y = (v - .5) * PAGE.height;
      let z = 0;
      if (definition.radius) {
        z = definition.radius * (1 - Math.cos(x / definition.radius));
        x = definition.radius * Math.sin(x / definition.radius);
      } else if (definition.sBend) {
        const at = Math.max(0, Math.min(count, u * count));
        const index = Math.min(count - 1, Math.floor(at));
        const mix = at - index;
        const first = crossSection[index], second = crossSection[index + 1];
        x = first.x * (1 - mix) + second.x * mix - crossSection[count / 2].x;
        z = first.z * (1 - mix) + second.z * mix - crossSection[count / 2].z;
      }
      const yaw = definition.yaw || 0, pitch = definition.pitch || 0;
      const rotatedX = x * Math.cos(yaw) + z * Math.sin(yaw);
      const rotatedZ = -x * Math.sin(yaw) + z * Math.cos(yaw);
      y = y * Math.cos(pitch) - rotatedZ * Math.sin(pitch);
      z = (v - .5) * PAGE.height * Math.sin(pitch) + rotatedZ * Math.cos(pitch);
      const depth = 650 + z;
      return { x: 450 + 1900 * rotatedX / depth, y: 590 + 1900 * y / depth };
    };
  }

  function template(definition = {}) {
    const canvas = document.createElement('canvas');
    canvas.width = 840;
    canvas.height = 1188;
    const ctx = canvas.getContext('2d');
    const scale = canvas.width / PAGE.width;
    ctx.scale(scale, scale);
    ctx.fillStyle = '#fff';
    ctx.fillRect(0, 0, PAGE.width, PAGE.height);
    ctx.fillStyle = '#cbc8bf';
    for (let x = 20; x <= 190; x += 5) {
      for (let y = 19.3; y <= 269.3; y += 5) {
        ctx.beginPath(); ctx.arc(x, y, .32, 0, Math.PI * 2); ctx.fill();
      }
    }
    ctx.strokeStyle = '#f0db4c';
    ctx.lineWidth = .85;
    ctx.strokeRect(15, 14.3, 180, 270);
    for (const left of [20, 110]) {
      ctx.strokeRect(left, 274.3, 80, 5);
      for (let x = left + 5; x < left + 80; x += 5) {
        if (definition.missingTeeth?.includes(x)) continue;
        ctx.beginPath(); ctx.moveTo(x, 274.3); ctx.lineTo(x, 279.3); ctx.stroke();
      }
    }
    ['#d42b29', '#272727', '#174fca', '#51a933'].forEach((color, index) => {
      ctx.fillStyle = color;
      ctx.lineWidth = .45;
      ctx.beginPath(); ctx.arc(101.25 + index * 2.5, 276.8, 1.25, 0, Math.PI * 2);
      ctx.fill(); ctx.stroke();
    });
    ctx.strokeStyle = '#164fbb';
    ctx.lineWidth = .65;
    for (const y of LINES) {
      ctx.beginPath(); ctx.moveTo(25, y); ctx.lineTo(185, y); ctx.stroke();
    }
    ctx.strokeStyle = '#c52c28';
    for (const x of VERTICAL_LINES) {
      ctx.beginPath(); ctx.moveTo(x, 25); ctx.lineTo(x, 255); ctx.stroke();
    }
    ctx.fillStyle = '#292929';
    ctx.font = '4px monospace';
    ctx.fillText('surface 0123456789 i.j;: x+y = z', 26, 55);
    ctx.fillText('Pencil, punctuation and thin ink', 26, 135);
    return canvas;
  }

  function rasterTriangle(output, texture, vertices) {
    const [a, b, c] = vertices;
    const denominator = (b.y - c.y) * (a.x - c.x) + (c.x - b.x) * (a.y - c.y);
    if (Math.abs(denominator) < 1e-9) return;
    const minX = Math.max(0, Math.floor(Math.min(a.x, b.x, c.x)));
    const maxX = Math.min(output.width - 1, Math.ceil(Math.max(a.x, b.x, c.x)));
    const minY = Math.max(0, Math.floor(Math.min(a.y, b.y, c.y)));
    const maxY = Math.min(output.height - 1, Math.ceil(Math.max(a.y, b.y, c.y)));
    for (let y = minY; y <= maxY; y += 1) for (let x = minX; x <= maxX; x += 1) {
      const wa = ((b.y - c.y) * (x + .5 - c.x) + (c.x - b.x) * (y + .5 - c.y)) / denominator;
      const wb = ((c.y - a.y) * (x + .5 - c.x) + (a.x - c.x) * (y + .5 - c.y)) / denominator;
      const wc = 1 - wa - wb;
      if (Math.min(wa, wb, wc) < -1e-7) continue;
      const sx = Math.max(0, Math.min(texture.width - 1.001, (a.u * wa + b.u * wb + c.u * wc) * texture.width - .5));
      const sy = Math.max(0, Math.min(texture.height - 1.001, (a.v * wa + b.v * wb + c.v * wc) * texture.height - .5));
      const ix = Math.floor(sx), iy = Math.floor(sy), fx = sx - ix, fy = sy - iy;
      const src = (iy * texture.width + ix) * 4, dest = (y * output.width + x) * 4;
      for (let channel = 0; channel < 3; channel += 1) {
        const upper = texture.data[src + channel] * (1 - fx) + texture.data[src + 4 + channel] * fx;
        const lower = texture.data[src + texture.width * 4 + channel] * (1 - fx)
          + texture.data[src + texture.width * 4 + 4 + channel] * fx;
        output.data[dest + channel] = upper * (1 - fy) + lower * fy;
      }
      output.data[dest + 3] = 255;
    }
  }

  function render(definition) {
    const paper = template(definition);
    const texture = paper.getContext('2d').getImageData(0, 0, paper.width, paper.height);
    const source = document.createElement('canvas'); source.width = 900; source.height = 1180;
    const context = source.getContext('2d', { willReadFrequently: true });
    context.fillStyle = '#886244'; context.fillRect(0, 0, source.width, source.height);
    const image = context.getImageData(0, 0, source.width, source.height);
    const map = cameraFor(definition), columns = 72, rows = 88;
    const vertex = (u, v) => ({ ...map(u, v), u, v });
    for (let y = 0; y < rows; y += 1) for (let x = 0; x < columns; x += 1) {
      const tl = vertex(x / columns, y / rows), tr = vertex((x + 1) / columns, y / rows);
      const bl = vertex(x / columns, (y + 1) / rows), br = vertex((x + 1) / columns, (y + 1) / rows);
      rasterTriangle(image, texture, [tl, tr, br]); rasterTriangle(image, texture, [tl, br, bl]);
    }
    context.putImageData(image, 0, 0);
    return { source, map, pageQuad: [map(0, 0), map(1, 0), map(1, 1), map(0, 1)] };
  }

  function exactFrame(map) {
    const series = callback => Array.from({ length: 129 }, (_, index) => callback(index / 128));
    return {
      confidence: 1,
      paths: {
        top: series(t => map(FRAME.u0 + t * (FRAME.u1 - FRAME.u0), FRAME.v0)),
        bottom: series(t => map(FRAME.u0 + t * (FRAME.u1 - FRAME.u0), FRAME.v1)),
        left: series(t => map(FRAME.u0, FRAME.v0 + t * (FRAME.v1 - FRAME.v0))),
        right: series(t => map(FRAME.u1, FRAME.v0 + t * (FRAME.v1 - FRAME.v0)))
      },
      box: {
        support: 1,
        top: series(t => map(FRAME.u0 + t * (FRAME.u1 - FRAME.u0), 274.3 / 297)),
        bottom: series(t => map(FRAME.u0 + t * (FRAME.u1 - FRAME.u0), 279.3 / 297))
      }
    };
  }

  function lineErrors(canvas) {
    const { width, height } = canvas;
    const data = canvas.getContext('2d').getImageData(0, 0, width, height).data;
    const horizontal = [], vertical = [];
    for (const line of LINES) for (let xMm = 30; xMm <= 180; xMm += 15) {
      if (VERTICAL_LINES.some(vertical => Math.abs(vertical - xMm) < 1)) continue;
      const x = Math.round(xMm / PAGE.width * width), expected = line / PAGE.height * height;
      let weight = 0, position = 0;
      for (let y = Math.floor(expected - height * .04); y <= Math.ceil(expected + height * .04); y += 1) {
        const offset = (y * width + x) * 4;
        const blue = Math.max(0, data[offset + 2] - Math.max(data[offset], data[offset + 1]) - 35);
        weight += blue; position += (y + .5) * blue;
      }
      horizontal.push(weight ? Math.abs(position / weight - expected) / height : 1);
    }
    for (const line of VERTICAL_LINES) for (let yMm = 50; yMm <= 250; yMm += 40) {
      const y = Math.round(yMm / PAGE.height * height), expected = line / PAGE.width * width;
      let weight = 0, position = 0;
      for (let x = Math.floor(expected - width * .04); x <= Math.ceil(expected + width * .04); x += 1) {
        const offset = (y * width + x) * 4;
        const red = Math.max(0, data[offset] - Math.max(data[offset + 1], data[offset + 2]) - 35);
        weight += red; position += (x + .5) * red;
      }
      vertical.push(weight ? Math.abs(position / weight - expected) / width : 1);
    }
    const errors = [...horizontal, ...vertical];
    return {
      maximum: Math.max(...errors), mean: errors.reduce((sum, error) => sum + error, 0) / errors.length,
      horizontalMaximum: Math.max(...horizontal), verticalMaximum: Math.max(...vertical)
    };
  }

  window.ScannerPhysicalFixtures = { CASES, cameraFor, render, exactFrame, lineErrors };
})();
