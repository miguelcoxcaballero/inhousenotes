/* Native bullet-journal template shared by the editor and scanner preview.
 * Draw in backing pixels, using the editor's width-based A4 geometry and
 * source-over colours. Photographed ink is a separate layer above this one.
 */
(function (root) {
  "use strict";

  const dotStamps = new Map();

  function createCanvas(width, height) {
    const canvas = root.document?.createElement
      ? root.document.createElement("canvas") : new root.OffscreenCanvas(width, height);
    canvas.width = width;
    canvas.height = height;
    return canvas;
  }

  function dotStamp(u) {
    let stamp = dotStamps.get(u);
    if (stamp) return stamp;
    const dotR = 0.035 * u;
    const blurSigma = 0.02 * u;
    const stampR = dotR + blurSigma;
    const stampSize = Math.ceil(stampR * 2) + 2;
    const canvas = createCanvas(stampSize, stampSize);
    const context = canvas.getContext("2d");
    const center = stampSize / 2;
    const gradient = context.createRadialGradient(center, center, 0, center, center, stampR);
    gradient.addColorStop(0, "#a8a8a8");
    gradient.addColorStop(Math.max(0, (dotR - blurSigma) / stampR), "#a8a8a8");
    gradient.addColorStop(1, "rgba(168,168,168,0)");
    context.fillStyle = gradient;
    context.beginPath();
    context.arc(center, center, stampR, 0, Math.PI * 2);
    context.fill();
    stamp = { canvas, center };
    if (dotStamps.size >= 4) dotStamps.delete(dotStamps.keys().next().value);
    dotStamps.set(u, stamp);
    return stamp;
  }

  function render(context, width, height, options = {}) {
    if (!context || !Number.isFinite(width) || !Number.isFinite(height) || width <= 0 || height <= 0) {
      throw new TypeError("A valid canvas is required for the bullet-journal template.");
    }
    context.save();
    try {
      context.setTransform(1, 0, 0, 1, 0, 0);
      context.globalAlpha = 1;
      context.globalCompositeOperation = "source-over";
      context.filter = "none";
      context.shadowColor = "transparent";
      context.shadowBlur = 0;
      context.lineCap = "butt";
      context.lineJoin = "miter";
      context.setLineDash([]);
      if (options.background !== false) {
        context.fillStyle = "white";
        context.fillRect(0, 0, width, height);
      }

      const u = width / 21;
      const strokeColor = "#f0db4c";
      const strokeW = 0.06 * u;
      const cornerR = 0.03 * u;
      context.strokeStyle = strokeColor;
      context.lineWidth = strokeW;
      context.beginPath();
      context.roundRect(1.5 * u, 1.43 * u, 18 * u, 27 * u, cornerR);
      context.stroke();

      if (options.dots !== false) {
        const stamp = dotStamp(u);
        for (let col = 0; col < 35; col++) {
          const cx = (2.0 + col * 0.5) * u;
          for (let row = 0; row < 51; row++) {
            const cy = (1.93 + row * 0.5) * u;
            context.drawImage(stamp.canvas, cx - stamp.center, cy - stamp.center);
          }
        }
      }

      const lineY = 27.43 * u;
      const lineH = 0.5 * u;
      context.beginPath();
      context.roundRect(2.0 * u, lineY, 8.0 * u, lineH, cornerR);
      context.stroke();
      context.beginPath();
      context.roundRect(11.0 * u, lineY, 8.0 * u, lineH, cornerR);
      context.stroke();
      context.beginPath();
      for (let x = 2.5; x <= 9.5 + 1e-9; x += 0.5) {
        context.moveTo(x * u, lineY);
        context.lineTo(x * u, lineY + lineH);
      }
      for (let x = 11.5; x <= 18.5 + 1e-9; x += 0.5) {
        context.moveTo(x * u, lineY);
        context.lineTo(x * u, lineY + lineH);
      }
      context.stroke();

      const circleY = 27.68 * u;
      const circleR = 0.125 * u;
      const colors = ["#ff0000", "#000000", "#0000ff", "#6eff12"];
      let circleX = 10.125 * u;
      for (const color of colors) {
        context.fillStyle = color;
        context.beginPath();
        context.arc(circleX, circleY, circleR, 0, Math.PI * 2);
        context.fill();
        context.stroke();
        circleX += 0.25 * u;
      }
    } finally {
      context.restore();
    }
    return context.canvas;
  }

  root.InhouseBulletTemplate = Object.freeze({ render });
})(typeof window === "undefined" ? globalThis : window);
