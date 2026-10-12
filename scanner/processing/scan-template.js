/* The printed A4 bullet-journal stencil used by scanner downloads and scans.
 * Geometry and SVG are preserved from scanner/script.js's canonical stencil.
 * Canvas rendering has no paper fill, so applying it cannot whiten handwriting.
 */
(function (root) {
  "use strict";
  const SP = (root.ScannerPro = root.ScannerPro || {});
  const STENCIL_BG_SVG = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 21 29.7" width="21cm" height="29.7cm">
      <defs>
        <filter id="softBlur" x="-50%" y="-50%" width="200%" height="200%">
          <feGaussianBlur in="SourceGraphic" stdDeviation="0.02"/>
        </filter>
        <pattern id="dotGrid" x="1.75" y="1.68" width="0.5" height="0.5" patternUnits="userSpaceOnUse">
          <circle cx="0.25" cy="0.25" r="0.035" fill="#a8a8a8" filter="url(#softBlur)"/>
        </pattern>
      </defs>
      <rect x="0" y="0" width="21" height="29.7" fill="white"/>
      <rect x="1.5" y="1.43" width="18" height="27" rx="0.03" ry="0.03"
            fill="none" stroke="#f0db4c" stroke-width="0.06"/>
      <rect x="1.75" y="1.68" width="17.5" height="25.5" fill="url(#dotGrid)"/>
      <g stroke="#f0db4c" stroke-width="0.06" fill="none">
        <rect x="2.0" y="27.43" width="8.0" height="0.5" rx="0.03" ry="0.03"/>
        <path d="M 2.5 27.43 v 0.5 M 3.0 27.43 v 0.5 M 3.5 27.43 v 0.5
                 M 4.0 27.43 v 0.5 M 4.5 27.43 v 0.5 M 5.0 27.43 v 0.5
                 M 5.5 27.43 v 0.5 M 6.0 27.43 v 0.5 M 6.5 27.43 v 0.5
                 M 7.0 27.43 v 0.5 M 7.5 27.43 v 0.5 M 8.0 27.43 v 0.5
                 M 8.5 27.43 v 0.5 M 9.0 27.43 v 0.5 M 9.5 27.43 v 0.5"/>
        <rect x="11.0" y="27.43" width="8.0" height="0.5" rx="0.03" ry="0.03"/>
        <path d="M 11.5 27.43 v 0.5 M 12.0 27.43 v 0.5 M 12.5 27.43 v 0.5
                 M 13.0 27.43 v 0.5 M 13.5 27.43 v 0.5 M 14.0 27.43 v 0.5
                 M 14.5 27.43 v 0.5 M 15.0 27.43 v 0.5 M 15.5 27.43 v 0.5
                 M 16.0 27.43 v 0.5 M 16.5 27.43 v 0.5 M 17.0 27.43 v 0.5
                 M 17.5 27.43 v 0.5 M 18.0 27.43 v 0.5 M 18.5 27.43 v 0.5"/>
      </g>
      <g stroke="#f0db4c" stroke-width="0.06">
        <circle cx="10.125" cy="27.68" r="0.125" fill="#ff0000"/>
        <circle cx="10.375" cy="27.68" r="0.125" fill="#000000"/>
        <circle cx="10.625" cy="27.68" r="0.125" fill="#0000ff"/>
        <circle cx="10.875" cy="27.68" r="0.125" fill="#6eff12"/>
      </g>
    </svg>`;

  const STENCIL_OVERLAY_SVG = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 21 29.7" width="21cm" height="29.7cm">
  <rect x="1.5" y="1.43" width="18" height="27" rx="0.03" ry="0.03"
        fill="none" stroke="#f0db4c" stroke-width="0.06"/>
  <g stroke="#f0db4c" stroke-width="0.06" fill="none">
    <rect x="2.0" y="27.43" width="8.0" height="0.5" rx="0.03" ry="0.03"/>
    <path d="M 2.5 27.43 v 0.5 M 3.0 27.43 v 0.5 M 3.5 27.43 v 0.5
             M 4.0 27.43 v 0.5 M 4.5 27.43 v 0.5 M 5.0 27.43 v 0.5
             M 5.5 27.43 v 0.5 M 6.0 27.43 v 0.5 M 6.5 27.43 v 0.5
             M 7.0 27.43 v 0.5 M 7.5 27.43 v 0.5 M 8.0 27.43 v 0.5
             M 8.5 27.43 v 0.5 M 9.0 27.43 v 0.5 M 9.5 27.43 v 0.5"/>
    <rect x="11.0" y="27.43" width="8.0" height="0.5" rx="0.03" ry="0.03"/>
    <path d="M 11.5 27.43 v 0.5 M 12.0 27.43 v 0.5 M 12.5 27.43 v 0.5
             M 13.0 27.43 v 0.5 M 13.5 27.43 v 0.5 M 14.0 27.43 v 0.5
             M 14.5 27.43 v 0.5 M 15.0 27.43 v 0.5 M 15.5 27.43 v 0.5
             M 16.0 27.43 v 0.5 M 16.5 27.43 v 0.5 M 17.0 27.43 v 0.5
             M 17.5 27.43 v 0.5 M 18.0 27.43 v 0.5 M 18.5 27.43 v 0.5"/>
  </g>
  <g stroke="#f0db4c" stroke-width="0.06">
    <circle cx="10.125" cy="27.68" r="0.125" fill="#ff0000"/>
    <circle cx="10.375" cy="27.68" r="0.125" fill="#000000"/>
    <circle cx="10.625" cy="27.68" r="0.125" fill="#0000ff"/>
    <circle cx="10.875" cy="27.68" r="0.125" fill="#6eff12"/>
  </g>
</svg>`;

  const geometry = Object.freeze({
    widthMM: 210, heightMM: 297,
    frame: Object.freeze({ x: 15, y: 14.3, width: 180, height: 270, radius: .3, stroke: .6, color: "#f0db4c" }),
    bullets: Object.freeze({ x: 20, y: 19.3, pitch: 5, columns: 35, rows: 51, radius: .35, blur: .2, color: "#a8a8a8" }),
    boxes: Object.freeze({ starts: Object.freeze([20, 110]), y: 274.3, width: 80, height: 5, pitch: 5 }),
    markers: Object.freeze({ centers: Object.freeze([101.25, 103.75, 106.25, 108.75]), y: 276.8, radius: 1.25,
      colors: Object.freeze(["#ff0000", "#000000", "#0000ff", "#6eff12"]) })
  });

  function svg(options = {}) {
    if (options.dots === false) return STENCIL_OVERLAY_SVG;
    return options.background === true ? STENCIL_BG_SVG
      : STENCIL_BG_SVG.replace('<rect x="0" y="0" width="21" height="29.7" fill="white"/>', "");
  }

  // Kept deliberately small: the canvas may be a full-resolution OffscreenCanvas
  // in a worker, while this reusable dot stamp is only a few dozen pixels.
  let dotStamp = null;
  function makeDotStamp(scale) {
    if (dotStamp && dotStamp.scale === scale) return dotStamp;
    const dot = geometry.bullets;
    const size = Math.max(3, Math.ceil(2 * (dot.radius + 4 * dot.blur) * scale) + 2);
    const canvas = typeof root.OffscreenCanvas === "function" ? new root.OffscreenCanvas(size, size)
      : root.document.createElement("canvas");
    canvas.width = canvas.height = size;
    const context = canvas.getContext("2d");
    context.fillStyle = dot.color;
    context.filter = `blur(${dot.blur * scale}px)`;
    context.beginPath();
    context.arc(size / 2, size / 2, dot.radius * scale, 0, Math.PI * 2);
    context.fill();
    dotStamp = { canvas, scale, sizeMM: size / scale };
    return dotStamp;
  }

  function roundedRect(context, x, y, width, height, radius) {
    context.moveTo(x + radius, y);
    context.lineTo(x + width - radius, y);
    context.quadraticCurveTo(x + width, y, x + width, y + radius);
    context.lineTo(x + width, y + height - radius);
    context.quadraticCurveTo(x + width, y + height, x + width - radius, y + height);
    context.lineTo(x + radius, y + height);
    context.quadraticCurveTo(x, y + height, x, y + height - radius);
    context.lineTo(x, y + radius);
    context.quadraticCurveTo(x, y, x + radius, y);
    context.closePath();
  }

  function render(context, width, height, options = {}) {
    if (!context || !Number.isFinite(width) || !Number.isFinite(height) || width <= 0 || height <= 0) {
      throw new TypeError("A valid scan canvas is required for the printed template.");
    }
    const scaleX = width / geometry.widthMM, scaleY = height / geometry.heightMM;
    context.save();
    try {
      context.setTransform(scaleX, 0, 0, scaleY, 0, 0);
      context.globalAlpha = 1;
      // Ink survives where it crosses the printed pattern. On clean white paper
      // these are the exact canonical stencil colours; there is no white layer.
      context.globalCompositeOperation = "multiply";
      context.filter = "none";
      context.shadowColor = "transparent";
      context.shadowBlur = 0;
      if (options.dots !== false) {
        const dot = geometry.bullets;
        const stamp = makeDotStamp(Math.max(scaleX, scaleY));
        const half = stamp.sizeMM / 2;
        for (let row = 0; row < dot.rows; row++) for (let column = 0; column < dot.columns; column++) {
          context.drawImage(stamp.canvas, dot.x + column * dot.pitch - half,
            dot.y + row * dot.pitch - half, stamp.sizeMM, stamp.sizeMM);
        }
      }
      const frame = geometry.frame, boxes = geometry.boxes, markers = geometry.markers;
      context.strokeStyle = frame.color;
      context.lineWidth = frame.stroke;
      context.lineCap = "butt";
      context.lineJoin = "miter";
      context.setLineDash([]);
      context.beginPath();
      roundedRect(context, frame.x, frame.y, frame.width, frame.height, frame.radius);
      for (const x of boxes.starts) {
        roundedRect(context, x, boxes.y, boxes.width, boxes.height, frame.radius);
        for (let offset = boxes.pitch; offset < boxes.width; offset += boxes.pitch) {
          context.moveTo(x + offset, boxes.y);
          context.lineTo(x + offset, boxes.y + boxes.height);
        }
      }
      context.stroke();
      for (let index = 0; index < markers.centers.length; index++) {
        context.beginPath();
        context.arc(markers.centers[index], markers.y, markers.radius, 0, Math.PI * 2);
        context.fillStyle = markers.colors[index];
        context.fill();
        context.stroke();
      }
    } finally {
      context.restore();
    }
    return context.canvas;
  }

  SP.ScanTemplate = Object.freeze({ geometry, svg, render });
})(typeof window === "undefined" ? globalThis : window);
