/* Frozen editor renderer from v5.11.62, used only as an independent pixel
 * regression oracle while the production renderer is shared with scans. */
window.renderOriginalEditorBulletTemplate = function (w, h) {
    const offCanvas = document.createElement('canvas');
    offCanvas.width = w;
    offCanvas.height = h;
    const ctx = offCanvas.getContext('2d');

    ctx.fillStyle = 'white';
    ctx.fillRect(0, 0, w, h);

    const u = w / 21;
    const strokeColor = '#f0db4c';
    const strokeW = 0.06 * u;
    const cornerR = 0.03 * u;

    // Outer frame
    ctx.strokeStyle = strokeColor;
    ctx.lineWidth = strokeW;
    ctx.beginPath();
    ctx.roundRect(1.5 * u, 1.43 * u, 18 * u, 27 * u, cornerR);
    ctx.stroke();

    // Dot grid with inward radial gradient per dot
    const dotR = 0.035 * u;
    const blurSigma = 0.02 * u;
    const stampR = dotR + blurSigma;
    const stampSize = Math.ceil(stampR * 2) + 2;
    const stampCanvas = document.createElement('canvas');
    stampCanvas.width = stampSize;
    stampCanvas.height = stampSize;
    const sCtx = stampCanvas.getContext('2d');
    const sc = stampSize / 2;
    const grad = sCtx.createRadialGradient(sc, sc, 0, sc, sc, stampR);
    grad.addColorStop(0, '#a8a8a8');
    grad.addColorStop(Math.max(0, (dotR - blurSigma) / stampR), '#a8a8a8');
    grad.addColorStop(1, 'rgba(168,168,168,0)');
    sCtx.fillStyle = grad;
    sCtx.beginPath();
    sCtx.arc(sc, sc, stampR, 0, Math.PI * 2);
    sCtx.fill();

    for (let col = 0; col < 35; col++) {
        const cx = (2.0 + col * 0.5) * u;
        for (let row = 0; row < 51; row++) {
            const cy = (1.93 + row * 0.5) * u;
            ctx.drawImage(stampCanvas, cx - sc, cy - sc);
        }
    }

    // Footer rectangles + ticks
    const lineY = 27.43 * u;
    const lineH = 0.5 * u;
    ctx.strokeStyle = strokeColor;
    ctx.lineWidth = strokeW;
    ctx.beginPath();
    ctx.roundRect(2.0 * u, lineY, 8.0 * u, lineH, cornerR);
    ctx.stroke();
    ctx.beginPath();
    ctx.roundRect(11.0 * u, lineY, 8.0 * u, lineH, cornerR);
    ctx.stroke();
    ctx.beginPath();
    for (let x = 2.5; x <= 9.5 + 1e-9; x += 0.5) {
        ctx.moveTo(x * u, lineY);
        ctx.lineTo(x * u, lineY + lineH);
    }
    for (let x = 11.5; x <= 18.5 + 1e-9; x += 0.5) {
        ctx.moveTo(x * u, lineY);
        ctx.lineTo(x * u, lineY + lineH);
    }
    ctx.stroke();

    // Footer color circles
    const circleY = 27.68 * u;
    const circleR = 0.125 * u;
    const colors = ['#ff0000', '#000000', '#0000ff', '#6eff12'];
    let circleX = 10.125 * u;
    ctx.strokeStyle = strokeColor;
    ctx.lineWidth = strokeW;
    for (const color of colors) {
        ctx.fillStyle = color;
        ctx.beginPath();
        ctx.arc(circleX, circleY, circleR, 0, Math.PI * 2);
        ctx.fill();
        ctx.stroke();
        circleX += 0.25 * u;
    }

    return offCanvas;
};
