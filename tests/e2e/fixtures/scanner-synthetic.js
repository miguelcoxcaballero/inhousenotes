(() => {
  'use strict';

  const A4_RATIO = 21 / 29.7;
  const FRAME = {
    u0: 1.5 / 21,
    u1: 19.5 / 21,
    v0: 1.43 / 29.7,
    v1: 28.43 / 29.7,
    boxTop: 27.43 / 29.7,
    marker: 27.68 / 29.7,
    boxBottom: 27.93 / 29.7
  };
  const INK_SWATCHES = [
    { name: 'red', x0: 2.7, x1: 4.3, y: 25.15, colour: '#d42b29', target: [232, 16, 16] },
    { name: 'black', x0: 6.4, x1: 8, y: 25.15, colour: '#272727', target: [77, 77, 77] },
    { name: 'blue', x0: 10.9, x1: 12.5, y: 25.15, colour: '#174fca', target: [0, 47, 217] },
    { name: 'green', x0: 15.1, x1: 16.7, y: 25.15, colour: '#51a933', target: [110, 255, 18] }
  ];

  const CASES = [
    { name: 'front-neutral', width: 900, height: 1180, quad: [[130, 55], [770, 66], [790, 1110], [105, 1090]], bowX: 0.004, bowY: -0.006, brightness: 1, contrast: 1, saturation: 1 },
    { name: 'keystone-warm', width: 900, height: 1180, quad: [[205, 72], [712, 30], [823, 1118], [78, 1072]], bowX: 0.018, bowY: 0.025, brightness: .91, contrast: 1.06, saturation: .82, tint: '#bd7b3a', tintAlpha: .10 },
    { name: 'keystone-cool', width: 900, height: 1180, quad: [[72, 122], [807, 58], [714, 1092], [164, 1135]], bowX: -.024, bowY: -.022, brightness: 1.08, contrast: .88, saturation: .72, tint: '#6688bb', tintAlpha: .09 },
    { name: 'landscape-clockwise', width: 1280, height: 900, quad: [[1100, 115], [1118, 785], [145, 822], [95, 92]], bowX: .012, bowY: .028, brightness: .96, contrast: 1.1, saturation: .95 },
    { name: 'landscape-counterclockwise', width: 1280, height: 900, quad: [[154, 810], [112, 138], [1110, 82], [1168, 788]], bowX: -.016, bowY: -.026, brightness: 1.03, contrast: .92, saturation: .78, tint: '#f0c56d', tintAlpha: .08 },
    { name: 'upside-down-shadow', width: 900, height: 1180, quad: [[786, 1110], [94, 1070], [145, 66], [760, 96]], bowX: .026, bowY: .032, brightness: .86, contrast: 1.12, saturation: .88, shadow: .28 },
    { name: 'strong-left-curl', width: 900, height: 1180, quad: [[118, 62], [784, 105], [742, 1114], [74, 1074]], bowX: .055, bowY: -.018, brightness: 1.04, contrast: .96, saturation: .9 },
    { name: 'strong-bottom-curl', width: 900, height: 1180, quad: [[132, 76], [772, 42], [824, 1080], [82, 1112]], bowX: -.012, bowY: .052, brightness: .98, contrast: 1.03, saturation: .7, blur: .45 },
    { name: 'faded-yellow', width: 900, height: 1180, quad: [[84, 102], [782, 62], [808, 1105], [116, 1130]], bowX: .02, bowY: -.034, brightness: 1.1, contrast: .82, saturation: .38, blur: .65 },
    { name: 'dark-camera', width: 900, height: 1180, quad: [[172, 50], [742, 92], [806, 1102], [76, 1078]], bowX: -.032, bowY: .028, brightness: .68, contrast: 1.18, saturation: .76, shadow: .36 },
    { name: 'overexposed', width: 900, height: 1180, quad: [[94, 48], [790, 88], [756, 1132], [126, 1092]], bowX: .025, bowY: .018, brightness: 1.22, contrast: .74, saturation: .58 },
    { name: 'magenta-cast', width: 900, height: 1180, quad: [[152, 112], [750, 48], [812, 1080], [70, 1125]], bowX: -.02, bowY: -.038, brightness: .94, contrast: 1.08, saturation: .86, tint: '#b24e78', tintAlpha: .075 },
    { name: 'green-cast', width: 900, height: 1180, quad: [[78, 56], [806, 118], [744, 1106], [136, 1072]], bowX: .034, bowY: .042, brightness: .9, contrast: .98, saturation: .74, tint: '#568d62', tintAlpha: .09 },
    { name: 'strong-right-curl-rose', width: 900, height: 1180, quad: [[138, 75], [770, 102], [814, 1098], [88, 1122]], bowX: -.052, bowY: .041, brightness: 1.01, contrast: 1.02, saturation: .84, tint: '#b86a72', tintAlpha: .065, shadow: .14 },
    { name: 'extreme-perspective-blur', width: 900, height: 1180, quad: [[278, 44], [676, 124], [834, 1092], [44, 1128]], bowX: .038, bowY: -.048, brightness: .83, contrast: 1.12, saturation: .63, tint: '#d89a56', tintAlpha: .08, blur: .8, shadow: .22 }
  ];

  const clamp = (value, minimum, maximum) => Math.max(minimum, Math.min(maximum, value));
  const point = value => ({ x: value[0], y: value[1] });
  const interpolateQuad = (quad, u, v) => {
    const top = {
      x: quad[0].x + (quad[1].x - quad[0].x) * u,
      y: quad[0].y + (quad[1].y - quad[0].y) * u
    };
    const bottom = {
      x: quad[3].x + (quad[2].x - quad[3].x) * u,
      y: quad[3].y + (quad[2].y - quad[3].y) * u
    };
    return {
      x: top.x + (bottom.x - top.x) * v,
      y: top.y + (bottom.y - top.y) * v
    };
  };

  function mappingFor(definition) {
    const quad = definition.quad.map(point);
    return (u, v) => {
      const base = interpolateQuad(quad, u, v);
      const horizontal = {
        x: (quad[1].x - quad[0].x) * (1 - v) + (quad[2].x - quad[3].x) * v,
        y: (quad[1].y - quad[0].y) * (1 - v) + (quad[2].y - quad[3].y) * v
      };
      const vertical = {
        x: (quad[3].x - quad[0].x) * (1 - u) + (quad[2].x - quad[1].x) * u,
        y: (quad[3].y - quad[0].y) * (1 - u) + (quad[2].y - quad[1].y) * u
      };
      const horizontalLength = Math.max(1, Math.hypot(horizontal.x, horizontal.y));
      const verticalLength = Math.max(1, Math.hypot(vertical.x, vertical.y));
      const horizontalNormal = { x: -horizontal.y / horizontalLength, y: horizontal.x / horizontalLength };
      const verticalNormal = { x: vertical.y / verticalLength, y: -vertical.x / verticalLength };
      // A separable Coons surface represents realistic cylindrical page bends.
      // Its complete geometry is observable in the four frame edges; the lower
      // calibration rails add two dense constraints where handheld pages curl most.
      const verticalBend = definition.bowY * verticalLength * Math.sin(Math.PI * u);
      const horizontalBend = definition.bowX * horizontalLength * Math.sin(Math.PI * v);
      return {
        x: base.x + horizontalNormal.x * verticalBend + verticalNormal.x * horizontalBend,
        y: base.y + horizontalNormal.y * verticalBend + verticalNormal.y * horizontalBend
      };
    };
  }

  function drawTemplate() {
    const canvas = document.createElement('canvas');
    canvas.width = 630;
    canvas.height = Math.round(canvas.width / A4_RATIO);
    const context = canvas.getContext('2d');
    const px = canvas.width / 21;
    const yellow = '#f0db4c';
    context.fillStyle = '#ffffff';
    context.fillRect(0, 0, canvas.width, canvas.height);

    context.fillStyle = '#c8c5bd';
    for (let y = 2.1; y < 27.1; y += .5) {
      for (let x = 2; x < 19.1; x += .5) {
        context.beginPath();
        context.arc(x * px, y * px, .032 * px, 0, Math.PI * 2);
        context.fill();
      }
    }

    context.strokeStyle = yellow;
    context.lineWidth = 2.8;
    context.lineJoin = 'round';
    context.strokeRect(1.5 * px, 1.43 * px, 18 * px, 27 * px);
    // Match the production stencil exactly: two separate eight-centimetre
    // calibration boxes with a one-centimetre marker gap. A continuous rail
    // here used to make the synthetic test easier than a real page.
    const drawCalibrationBox = start => {
      context.strokeRect(start * px, 27.43 * px, 8 * px, .5 * px);
      context.beginPath();
      for (let x = start + .5; x < start + 8; x += .5) {
        context.moveTo(x * px, 27.43 * px);
        context.lineTo(x * px, 27.93 * px);
      }
      context.stroke();
    };
    drawCalibrationBox(2);
    drawCalibrationBox(11);

    const markerColours = ['#d42b29', '#272727', '#174fca', '#51a933'];
    markerColours.forEach((colour, index) => {
      context.fillStyle = colour;
      context.strokeStyle = yellow;
      context.lineWidth = 1.8;
      context.beginPath();
      context.arc((10.125 + index * .25) * px, 27.68 * px, .125 * px, 0, Math.PI * 2);
      context.fill();
      context.stroke();
    });

    context.strokeStyle = '#183d89';
    context.fillStyle = '#183d89';
    context.lineWidth = 2.2;
    context.font = `${.28 * px}px system-ui`;
    for (let row = 0; row < 9; row += 1) {
      const y = (3.1 + row * 2.3) * px;
      context.beginPath();
      context.moveTo(2.4 * px, y);
      context.bezierCurveTo(6.3 * px, y - .22 * px, 12.5 * px, y + .25 * px, 18.1 * px, y - .05 * px);
      context.stroke();
      context.fillText(`Synthetic scanner line ${row + 1}`, 2.5 * px, y - .18 * px);
    }
    // Camera casts should not merely preserve the page geometry. Four short
    // handwriting-like strokes verify the calibration circles restore the
    // same production ink colours after every synthetic lighting transform.
    context.lineWidth = .085 * px;
    context.lineCap = 'round';
    INK_SWATCHES.forEach(swatch => {
      context.strokeStyle = swatch.colour;
      context.beginPath();
      context.moveTo(swatch.x0 * px, swatch.y * px);
      context.quadraticCurveTo(
        (swatch.x0 + swatch.x1) * .5 * px,
        (swatch.y - .08) * px,
        swatch.x1 * px,
        swatch.y * px
      );
      context.stroke();
    });
    context.fillText('TEMPLATE', 1.75 * px, 27.82 * px);
    context.fillText('15-08-26', 16.3 * px, 27.82 * px);
    return canvas;
  }

  function drawTriangle(context, source, sourceTriangle, destinationTriangle) {
    const [s0, s1, s2] = sourceTriangle;
    const [d0, d1, d2] = destinationTriangle;
    const denominator = s0.x * (s1.y - s2.y) + s1.x * (s2.y - s0.y) + s2.x * (s0.y - s1.y);
    const a = (d0.x * (s1.y - s2.y) + d1.x * (s2.y - s0.y) + d2.x * (s0.y - s1.y)) / denominator;
    const b = (d0.y * (s1.y - s2.y) + d1.y * (s2.y - s0.y) + d2.y * (s0.y - s1.y)) / denominator;
    const c = (d0.x * (s2.x - s1.x) + d1.x * (s0.x - s2.x) + d2.x * (s1.x - s0.x)) / denominator;
    const d = (d0.y * (s2.x - s1.x) + d1.y * (s0.x - s2.x) + d2.y * (s1.x - s0.x)) / denominator;
    const e = (d0.x * (s1.x * s2.y - s2.x * s1.y) + d1.x * (s2.x * s0.y - s0.x * s2.y) + d2.x * (s0.x * s1.y - s1.x * s0.y)) / denominator;
    const f = (d0.y * (s1.x * s2.y - s2.x * s1.y) + d1.y * (s2.x * s0.y - s0.x * s2.y) + d2.y * (s0.x * s1.y - s1.x * s0.y)) / denominator;
    const center = destinationTriangle.reduce((sum, vertex) => ({
      x: sum.x + vertex.x / 3,
      y: sum.y + vertex.y / 3
    }), { x: 0, y: 0 });
    const clipTriangle = destinationTriangle.map(vertex => {
      const dx = vertex.x - center.x;
      const dy = vertex.y - center.y;
      const length = Math.max(1, Math.hypot(dx, dy));
      return { x: vertex.x + dx / length * .9, y: vertex.y + dy / length * .9 };
    });
    context.save();
    context.beginPath();
    context.moveTo(clipTriangle[0].x, clipTriangle[0].y);
    context.lineTo(clipTriangle[1].x, clipTriangle[1].y);
    context.lineTo(clipTriangle[2].x, clipTriangle[2].y);
    context.closePath();
    context.clip();
    context.setTransform(a, b, c, d, e, f);
    context.drawImage(source, 0, 0);
    context.restore();
  }

  function renderCase(definition) {
    const template = drawTemplate();
    const map = mappingFor(definition);
    const clean = document.createElement('canvas');
    clean.width = definition.width;
    clean.height = definition.height;
    const context = clean.getContext('2d');
    const background = context.createLinearGradient(0, 0, clean.width, clean.height);
    background.addColorStop(0, '#875f3e');
    background.addColorStop(.5, '#ad8058');
    background.addColorStop(1, '#62472f');
    context.fillStyle = background;
    context.fillRect(0, 0, clean.width, clean.height);

    // Distractor sheets force the detector to identify this template rather
    // than simply selecting the largest pale connected component.
    context.fillStyle = '#e7e4dd';
    context.save();
    context.translate(clean.width * .06, clean.height * .15);
    context.rotate(-.14);
    context.fillRect(0, 0, clean.width * .7, clean.height * .62);
    context.restore();
    context.fillStyle = '#dedbd3';
    context.save();
    context.translate(clean.width * .55, -clean.height * .06);
    context.rotate(.18);
    context.fillRect(0, 0, clean.width * .55, clean.height * .42);
    context.restore();

    const columns = 30;
    const rows = 42;
    for (let row = 0; row < rows; row += 1) {
      const v0 = row / rows;
      const v1 = (row + 1) / rows;
      for (let column = 0; column < columns; column += 1) {
        const u0 = column / columns;
        const u1 = (column + 1) / columns;
        const s00 = { x: u0 * template.width, y: v0 * template.height };
        const s10 = { x: u1 * template.width, y: v0 * template.height };
        const s01 = { x: u0 * template.width, y: v1 * template.height };
        const s11 = { x: u1 * template.width, y: v1 * template.height };
        const d00 = map(u0, v0);
        const d10 = map(u1, v0);
        const d01 = map(u0, v1);
        const d11 = map(u1, v1);
        drawTriangle(context, template, [s00, s01, s10], [d00, d01, d10]);
        drawTriangle(context, template, [s10, s01, s11], [d10, d01, d11]);
      }
    }
    context.setTransform(1, 0, 0, 1, 0, 0);

    const processed = document.createElement('canvas');
    processed.width = clean.width;
    processed.height = clean.height;
    const output = processed.getContext('2d');
    output.filter = `brightness(${definition.brightness || 1}) contrast(${definition.contrast || 1}) saturate(${definition.saturation || 1}) blur(${definition.blur || 0}px)`;
    output.drawImage(clean, 0, 0);
    output.filter = 'none';
    if (definition.tint) {
      output.globalCompositeOperation = 'multiply';
      output.globalAlpha = definition.tintAlpha || .08;
      output.fillStyle = definition.tint;
      output.fillRect(0, 0, processed.width, processed.height);
      output.globalAlpha = 1;
      output.globalCompositeOperation = 'source-over';
    }
    if (definition.shadow) {
      const shade = output.createRadialGradient(
        processed.width * .72, processed.height * .35, 5,
        processed.width * .72, processed.height * .35, Math.max(processed.width, processed.height) * .78
      );
      shade.addColorStop(0, 'rgba(0,0,0,0)');
      shade.addColorStop(1, `rgba(0,0,0,${definition.shadow})`);
      output.fillStyle = shade;
      output.fillRect(0, 0, processed.width, processed.height);
    }
    return { source: processed, map };
  }

  const distance = (first, second) => Math.hypot(first.x - second.x, first.y - second.y);
  const samplePolyline = (points, amount) => {
    const scaled = clamp(amount, 0, 1) * (points.length - 1);
    const index = Math.min(points.length - 2, Math.floor(scaled));
    const mix = scaled - index;
    return {
      x: points[index].x + (points[index + 1].x - points[index].x) * mix,
      y: points[index].y + (points[index + 1].y - points[index].y) * mix
    };
  };

  function curveErrors(points, truth) {
    if (!points?.length) return { mean: Infinity, maximum: Infinity };
    const errors = [];
    for (let index = 0; index <= 24; index += 1) {
      const amount = index / 24;
      errors.push(distance(samplePolyline(points, amount), truth(amount)));
    }
    return {
      mean: errors.reduce((sum, value) => sum + value, 0) / errors.length,
      maximum: Math.max(...errors)
    };
  }

  function evaluateDetection(definition, detection) {
    const map = mappingFor(definition);
    const longEdge = Math.max(definition.width, definition.height);
    const expectedPage = [map(0, 0), map(1, 0), map(1, 1), map(0, 1)];
    const cornerErrors = detection.pageQuad.map((detected, index) => distance(detected, expectedPage[index]));
    const paperCornerErrors = detection.paperCandidate?.length === 4
      ? detection.paperCandidate.map((detected, index) => distance(detected, expectedPage[index]))
      : [Infinity];
    const frame = detection.frame;
    const mappedPage = frame ? [
      ScannerPro.Lightweight.mapDetectedFrame(frame, 0, 0),
      ScannerPro.Lightweight.mapDetectedFrame(frame, 1, 0),
      ScannerPro.Lightweight.mapDetectedFrame(frame, 1, 1),
      ScannerPro.Lightweight.mapDetectedFrame(frame, 0, 1)
    ] : null;
    const mappedCornerErrors = mappedPage?.every(Boolean)
      ? mappedPage.map((detected, index) => distance(detected, expectedPage[index]))
      : [Infinity];
    const curves = frame ? {
      top: curveErrors(frame.paths.top, amount => map(FRAME.u0 + amount * (FRAME.u1 - FRAME.u0), FRAME.v0)),
      right: curveErrors(frame.paths.right, amount => map(FRAME.u1, FRAME.v0 + amount * (FRAME.v1 - FRAME.v0))),
      bottom: curveErrors(frame.paths.bottom, amount => map(FRAME.u0 + amount * (FRAME.u1 - FRAME.u0), FRAME.v1)),
      left: curveErrors(frame.paths.left, amount => map(FRAME.u0, FRAME.v0 + amount * (FRAME.v1 - FRAME.v0))),
      boxTop: curveErrors(frame.box?.top, amount => map(FRAME.u0 + amount * (FRAME.u1 - FRAME.u0), FRAME.boxTop)),
      boxBottom: curveErrors(frame.box?.bottom, amount => map(FRAME.u0 + amount * (FRAME.u1 - FRAME.u0), FRAME.boxBottom))
    } : null;
    const allCurveErrors = curves ? Object.values(curves) : [];
    return {
      method: detection.method,
      confidence: detection.confidence,
      cornerErrors,
      cornerMaximumRatio: Math.max(...cornerErrors) / longEdge,
      paperCornerMaximumRatio: Math.max(...paperCornerErrors) / longEdge,
      mappedPage,
      mappedCornerMaximumRatio: Math.max(...mappedCornerErrors) / longEdge,
      curveMeanRatio: allCurveErrors.reduce((sum, value) => sum + value.mean, 0) / Math.max(1, allCurveErrors.length) / longEdge,
      curveMaximumRatio: Math.max(0, ...allCurveErrors.map(value => value.maximum)) / longEdge,
      curves,
      frameSupport: frame?.support || 0,
      boxSupport: frame?.box?.support || 0,
      refinementMovement: frame?.refinementMovement || 0,
      refinementArmSupport: frame?.refinementArmSupport || 0
    };
  }

  window.ScannerSyntheticFixtures = {
    CASES,
    FRAME,
    INK_SWATCHES,
    renderCase,
    mappingFor,
    evaluateDetection
  };
})();
