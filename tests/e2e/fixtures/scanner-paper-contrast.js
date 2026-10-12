/* Independent reflectance/illumination fixture for the post-warp colour stage.
 * Ink is painted before the light field: shadows affect paper and ink equally.
 * Observations include only printed dots actually visible outside the writing.
 */
(() => {
  const WIDTH = 840, HEIGHT = 1188, SCALE = WIDTH / 210;
  const luminance = (r, g, b) => .2126 * r + .7152 * g + .0722 * b;
  const smooth = value => { const t = Math.max(0, Math.min(1, value)); return t * t * (3 - 2 * t); };
  const canvas = () => {
    const result = document.createElement('canvas'); result.width = WIDTH; result.height = HEIGHT; return result;
  };
  const inkLayers = [
    { name: 'pencil', color: '#c6c6c6', draw(context) {
      context.lineWidth = .36; context.beginPath();
      for (let x = 28; x <= 182; x += .25) {
        const y = 58 + 1.2 * Math.sin(x / 4) + .5 * Math.sin(x / 1.4);
        if (x === 28) context.moveTo(x, y); else context.lineTo(x, y);
      }
      context.stroke();
    } },
    { name: 'faintPencil', color: '#dedede', draw(context) {
      context.lineWidth = .4; context.beginPath();
      for (let x = 28; x <= 182; x += .25) {
        const y = 98 + 1.5 * Math.sin(x / 5) + .55 * Math.sin(x / 1.3);
        if (x === 28) context.moveTo(x, y); else context.lineTo(x, y);
      }
      context.stroke();
    } },
    { name: 'blackPen', color: '#303030', draw(context) {
      context.lineWidth = .4; context.beginPath(); context.moveTo(28, 130);
      context.bezierCurveTo(68, 126, 127, 137, 181, 131); context.stroke();
    } },
    { name: 'bluePen', color: '#214da9', draw(context) {
      context.lineWidth = .5; context.beginPath(); context.moveTo(28, 153);
      context.bezierCurveTo(60, 147, 131, 160, 183, 151); context.stroke();
    } },
    { name: 'punctuation', color: '#a6a6a6', draw(context) {
      for (const x of [39, 74, 109, 144, 174]) {
        context.beginPath(); context.arc(x, 117, .46, 0, Math.PI * 2); context.fill();
        context.lineWidth = .3; context.beginPath(); context.moveTo(x + 2, 116);
        context.lineTo(x + 2, 118.5); context.stroke();
      }
    } },
    { name: 'grayDrawing', color: '#999999', draw(context) {
      context.beginPath(); context.ellipse(128, 201, 19, 13, -.15, 0, Math.PI * 2); context.fill();
    } },
    { name: 'charcoalDrawing', color: '#4d4d4d', draw(context) {
      context.beginPath(); context.moveTo(44, 206); context.lineTo(61, 191);
      context.lineTo(78, 209); context.lineTo(62, 218); context.closePath(); context.fill();
    } }
  ];
  function lightAt(x, y, definition) {
    const u = x / WIDTH, v = y / HEIGHT;
    const soft = Math.exp(-2 * (((u - .37) / .3) ** 2 + ((v - .65) / .18) ** 2));
    // A close obstruction produces a narrow but nonzero penumbra. Its edge
    // crosses both faint writing lines, so removing it cannot just erase ink.
    const distance = (x + .86 * y - WIDTH * 1.03) / Math.hypot(1, .86);
    const upper = 1 - smooth((v - .31) / .06);
    const hard = smooth((distance + 2) / 4) * upper;
    const bottomDistance = Math.hypot((u - .8) / .25, (v - .86) / .14);
    const lower = 1 - smooth((bottomDistance - .8) / .2);
    return { exposure: (.96 - .07 * u) * (1 - .38 * soft) * (1 - .41 * hard) * (1 - .3 * lower), hard,
      zone: hard > .95 ? 'hardShadow' : Math.abs(distance) < 6 && upper > .95 ? 'shadowEdge'
        : soft > .6 ? 'softShadow' : lower > .9 ? 'lowerShadow' : 'lit' };
  }
  async function render(definition = {}) {
    const source = canvas(), context = source.getContext('2d', { willReadFrequently: true });
    context.fillStyle = '#fafafa'; context.fillRect(0, 0, WIDTH, HEIGHT);
    context.save(); context.scale(SCALE, SCALE);
    context.strokeStyle = '#f0db4c'; context.lineWidth = .6;
    context.strokeRect(15, 14.3, 180, 270);
    context.save(); context.filter = 'blur(.7px)'; context.fillStyle = '#a8a8a8';
    const dots = [];
    for (let row = 0; row < 51; row++) for (let col = 0; col < 35; col++) {
      const x = 20 + col * 5, y = 19.3 + row * 5;
      context.beginPath(); context.arc(x, y, .35, 0, Math.PI * 2); context.fill();
      dots.push({ x: x * SCALE, y: y * SCALE, u: x / 210, v: y / 297, row, col });
    }
    context.restore();
    for (const left of [20, 110]) {
      context.strokeRect(left, 274.3, 80, 5);
      for (let x = left + 5; x < left + 80; x += 5) {
        context.beginPath(); context.moveTo(x, 274.3); context.lineTo(x, 279.3); context.stroke();
      }
    }
    ['#f00', '#000', '#00f', '#6eff12'].forEach((color, index) => {
      context.fillStyle = color; context.beginPath(); context.arc(101.25 + index * 2.5, 276.8, 1.25, 0, Math.PI * 2);
      context.fill(); context.stroke();
    });
    context.restore();
    const printed = context.getImageData(0, 0, WIDTH, HEIGHT).data;
    const mask = canvas(), maskContext = mask.getContext('2d', { willReadFrequently: true });
    const ink = new Uint8Array(WIDTH * HEIGHT), samples = {}, strokeColumns = {};
    for (const layer of inkLayers) {
      context.save(); context.scale(SCALE, SCALE);
      context.fillStyle = context.strokeStyle = definition.paleWriting && layer.name === 'bluePen' ? '#b2c1d7' : layer.color;
      layer.draw(context); context.restore();
      maskContext.clearRect(0, 0, WIDTH, HEIGHT);
      maskContext.save(); maskContext.scale(SCALE, SCALE);
      maskContext.fillStyle = maskContext.strokeStyle = '#000'; layer.draw(maskContext); maskContext.restore();
      const alpha = maskContext.getImageData(0, 0, WIDTH, HEIGHT).data, positions = [];
      for (let i = 0; i < ink.length; i++) {
        if (alpha[i * 4 + 3]) ink[i] = 1;
        if (alpha[i * 4 + 3] > 235 && printed[i * 4] >= 248 && i % 2 === 0) positions.push(i);
      }
      samples[layer.name] = positions;
      if (['pencil', 'faintPencil', 'blackPen', 'bluePen'].includes(layer.name)) {
        // Measure the full painted stroke, including where it crosses printed
        // bullets. Excluding printed pixels here would hide cleanup cutouts.
        const columns = new Map();
        for (let i = 0; i < ink.length; i++) if (alpha[i * 4 + 3] > 100) {
          const x = i % WIDTH;
          if (!columns.has(x)) columns.set(x, []);
          columns.get(x).push(i);
        }
        strokeColumns[layer.name] = [...columns].sort((a, b) => a[0] - b[0]);
      }
    }
    const image = context.getImageData(0, 0, WIDTH, HEIGHT);
    const clean = new Uint8ClampedArray(image.data), paper = { lit: [], hardShadow: [], shadowEdge: [], softShadow: [], lowerShadow: [] };
    if (definition.missingCornerObservations) paper.unobservedCorner = [];
    const gains = definition.gains || [1, 1, 1];
    for (let y = 0; y < HEIGHT; y++) for (let x = 0; x < WIDTH; x++) {
      const i = y * WIDTH + x, offset = i * 4, illumination = lightAt(x, y, definition);
      // Deterministic fine camera grain must not become dark speckles after
      // increasing writing contrast. Its amplitude is below the pencil signal.
      const noise = (((Math.imul(x + 71, 1103515245) ^ Math.imul(y + 19, 12345)) >>> 8) % 13 - 6) / 4;
      for (let channel = 0; channel < 3; channel++) image.data[offset + channel] =
        Math.max(0, clean[offset + channel] * illumination.exposure * gains[channel] + noise);
      if (x % 3 === 0 && y % 3 === 0 && x > 18 * SCALE && x < 192 * SCALE && y > 17 * SCALE && y < 271 * SCALE
        && !ink[i] && Math.min(...clean.subarray(offset, offset + 3)) >= 249) {
        paper[illumination.zone].push(i);
        if (definition.missingCornerObservations && x < 80 * SCALE && y < 114 * SCALE) paper.unobservedCorner.push(i);
      }
    }
    context.putImageData(image, 0, 0);
    if (definition.jpeg) {
      const blob = await new Promise(resolve => source.toBlob(resolve, 'image/jpeg', .94));
      const bitmap = await createImageBitmap(blob); context.drawImage(bitmap, 0, 0); bitmap.close();
    }
    const visibleDots = dots.filter(point => {
      const x = Math.round(point.x), y = Math.round(point.y), radius = 4;
      for (let dy = -radius; dy <= radius; dy++) for (let dx = -radius; dx <= radius; dx++) {
        if (ink[(y + dy) * WIDTH + x + dx]) return false;
      }
      return true;
    });
    // Dropped observations model low-confidence detections, not an erased
    // patch of paper. Source pixels stay identical to the fully observed case.
    let dotObservations = visibleDots.filter(point =>
      !(definition.missingCornerObservations && point.col < 13 && point.row < 20)
      && !(definition.missingShadowObservations && lightAt(point.x, point.y, definition).hard > .05));
    if (Number.isInteger(definition.observationLimit) && definition.observationLimit < dotObservations.length) {
      const full = dotObservations;
      dotObservations = Array.from({ length: definition.observationLimit }, (_, index) =>
        full[Math.floor((index + .5) * full.length / definition.observationLimit)]);
    }
    return { source, clean, samples, strokeColumns, paper, dotObservations, visibleDotCount: visibleDots.length, gains };
  }
  function measure(source, fixture) {
    const data = source.getContext('2d').getImageData(0, 0, WIDTH, HEIGHT).data;
    const summarize = positions => {
      const values = positions.map(index => luminance(data[index * 4], data[index * 4 + 1], data[index * 4 + 2])).sort((a, b) => a - b);
      const sample = fraction => values[Math.min(values.length - 1, Math.floor(values.length * fraction))];
      return { count: values.length, p10: sample(.1), median: sample(.5), p90: sample(.9),
        darkFraction: values.filter(value => value < 235).length / values.length,
        mean: values.reduce((sum, value) => sum + value, 0) / values.length };
    };
    const writing = {};
    for (const [name, positions] of Object.entries(fixture.samples)) {
      const statistics = summarize(positions);
      const byIllumination = {};
      for (const index of positions) {
        const zone = lightAt(index % WIDTH, Math.floor(index / WIDTH), {}).zone;
        (byIllumination[zone] ||= []).push(index);
      }
      for (const [zone, indices] of Object.entries(byIllumination)) {
        const measurements = summarize(indices);
        byIllumination[zone] = { count: indices.length, contrast: 255 - measurements.median };
      }
      let continuity;
      if (fixture.strokeColumns[name]) {
        let run = 0, longestWhiteRun = 0, missingColumns = 0;
        const gaps = [];
        for (const [x, indices] of fixture.strokeColumns[name]) {
          const darkest = Math.min(...indices.map(index => luminance(data[index * 4], data[index * 4 + 1], data[index * 4 + 2])));
          if (darkest > 245) {
            run++; missingColumns++; longestWhiteRun = Math.max(run, longestWhiteRun);
            gaps.push(x);
          } else run = 0;
        }
        continuity = { longestWhiteRun, missingColumns, columnCount: fixture.strokeColumns[name].length, gaps };
      }
      writing[name] = { ...statistics, contrast: 255 - statistics.median, byIllumination, continuity };
    }
    const paper = {};
    for (const [name, positions] of Object.entries(fixture.paper)) paper[name] = summarize(positions);
    const blue = fixture.samples.bluePen;
    const blueChroma = blue.map(index => data[index * 4 + 2] - data[index * 4]).sort((a, b) => a - b);
    return { writing, paper, blueChroma: blueChroma[Math.floor(blueChroma.length / 2)] };
  }
  window.ScannerPaperContrastFixtures = { render, measure };
})();
