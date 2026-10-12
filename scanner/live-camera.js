(() => {
  'use strict';

  const SP = window.ScannerPro = window.ScannerPro || {};
  const $ = id => document.getElementById(id);
  const root = $('liveScanner');
  if (!root) return;
  const video = $('liveVideo');
  const overlay = $('liveOverlay');
  const overlayContext = overlay.getContext('2d');
  const analysis = document.createElement('canvas');
  const analysisContext = analysis.getContext('2d', { willReadFrequently: true });
  const params = new URLSearchParams(location.search);
  const embedded = window.__scannerEmbedMode === true;
  const sessionId = params.get('session') || '';
  let generation = 0;
  let stream = null;
  let timer = 0;
  let state = 'closed';
  let lastDetection = null;
  let stableAnchor = null;
  let stableSince = 0;
  let stableFrames = 0;
  let lastAnalysisTime = 0;
  let lastObservedAt = 0;
  let visualTarget = null;
  let visualDetection = null;
  let visualFrame = 0;
  let visualFrameAt = 0;
  let visualResponse = 100;
  let lastQualifiedAt = 0;
  let bufferedFrames = [];
  let bufferEpoch = 0;
  let buffering = false;
  let lastBufferedAt = 0;
  let processingController = null;
  let candidates = [];
  let result = null;
  let previewUrl = '';
  let documentName = 'Scanned page';
  let submitting = false;
  let selection = 0;
  let lastFrameTime = -1;
  let deliveredFrames = 0;
  let frameCallback = 0;
  let lastFrameAt = 0;
  let startingAt = 0;
  let resumeAfterPermission = false;
  let previousFocus = null;
  const backgroundElements = new Map();

  const post = (type, payload = {}) => {
    if (!embedded || window.parent === window) return false;
    window.parent.postMessage({ source: 'inhouse-scanner', type, sessionId, ...payload }, location.origin);
    return true;
  };
  const setState = next => {
    state = next;
    root.dataset.state = next;
    root.dataset.phase = next === 'review' ? 'review'
      : next === 'capturing' || next === 'processing' ? 'processing'
        : next === 'error' ? 'error' : 'camera';
    $('liveCaptureBtn').disabled = !['searching', 'stable'].includes(next);
  };
  const status = (title, hint = '') => {
    $('liveStatus').textContent = title;
    $('liveHint').textContent = hint;
  };
  const resetStability = () => {
    stableAnchor = null;
    stableSince = 0;
    stableFrames = 0;
    lastAnalysisTime = 0;
    lastQualifiedAt = 0;
    bufferedFrames = [];
    bufferEpoch++;
    lastBufferedAt = 0;
    $('liveProgress').value = 0;
  };
  const stopCamera = () => {
    clearTimeout(timer);
    timer = 0;
    clearVisualTracking();
    if (frameCallback) video.cancelVideoFrameCallback?.(frameCallback);
    frameCallback = 0;
    if (stream) stream.getTracks().forEach(track => track.stop());
    stream = null;
    video.srcObject = null;
    lastDetection = null;
    lastObservedAt = 0;
    lastFrameTime = -1;
    deliveredFrames = 0;
    lastFrameAt = 0;
    root.dataset.cameraReady = 'false';
    resetStability();
    overlayContext.clearRect(0, 0, overlay.width, overlay.height);
    overlay.dataset.detected = 'false';
  };
  const clearReview = () => {
    selection++;
    processingController?.abort();
    processingController = null;
    result = null;
    candidates.forEach(candidate => URL.revokeObjectURL(candidate.url));
    candidates = [];
    if (previewUrl) URL.revokeObjectURL(previewUrl);
    previewUrl = '';
    $('livePreview').removeAttribute('src');
    $('livePreview').hidden = true;
    $('liveAlternatives').replaceChildren();
    $('liveAddBtn').disabled = true;
  };
  function close() {
    if (submitting) return false;
    generation++;
    resumeAfterPermission = false;
    stopCamera();
    clearReview();
    root.hidden = true;
    document.body.classList.remove('live-scanner-open');
    backgroundElements.forEach((inert, element) => { element.inert = inert; });
    backgroundElements.clear();
    if (previousFocus?.isConnected) previousFocus.focus({ preventScroll: true });
    previousFocus = null;
    setState('closed');
    return true;
  }
  function cameraError(error, interrupted = false) {
    generation++;
    resumeAfterPermission = false;
    stopCamera();
    setState('error');
    $('liveRetryBtn').hidden = false;
    const denied = error?.name === 'NotAllowedError' || error?.name === 'SecurityError';
    const missing = error?.name === 'NotFoundError' || error?.name === 'DevicesNotFoundError';
    status(interrupted ? 'Camera interrupted' : denied ? 'Camera permission needed' : missing ? 'No camera found' : 'Camera unavailable',
      denied ? 'Allow camera access in your browser or device settings, then try again. You can also choose a photo.'
        : 'Try opening the camera again, or choose a photo to scan.');
  }
  async function open() {
    if (submitting) return;
    const token = ++generation;
    resumeAfterPermission = false;
    stopCamera();
    clearReview();
    if (root.hidden) {
      previousFocus = document.activeElement;
      [...document.body.children].filter(element => element !== root).forEach(element => {
        backgroundElements.set(element, element.inert);
        element.inert = true;
      });
    }
    root.hidden = false;
    document.body.classList.add('live-scanner-open');
    $('sourceModal')?.classList.remove('open');
    $('liveCameraView').hidden = false;
    $('liveReview').hidden = true;
    $('liveRetryBtn').hidden = true;
    setState('requesting');
    $('liveCloseBtn').focus({ preventScroll: true });
    status('Opening camera…', 'Point the camera at the whole page, including the yellow frame and bottom squares.');
    try {
      if (!navigator.mediaDevices?.getUserMedia) throw new Error('Camera API unavailable');
      let acquired;
      try {
        acquired = await navigator.mediaDevices.getUserMedia({ audio: false, video: {
          facingMode: { ideal: 'environment' }, width: { ideal: 2560 }, height: { ideal: 1920 }
        } });
      } catch (error) {
        if (token !== generation) return;
        if (error?.name !== 'OverconstrainedError') throw error;
        acquired = await navigator.mediaDevices.getUserMedia({ audio: false, video: true });
      }
      // Android permission sheets can briefly hide the WebView. Keep the
      // pending request valid, but never keep a granted camera in the background.
      if (token !== generation) {
        acquired.getTracks().forEach(track => track.stop());
        return;
      }
      if (document.hidden) {
        acquired.getTracks().forEach(track => track.stop());
        pauseCamera(true);
        resumeAfterPermission = true;
        return;
      }
      stream = acquired;
      video.srcObject = acquired;
      // Keep the camera adapting while the phone approaches the paper. Only
      // request modes that this particular camera advertises as supported.
      const cameraTrack = acquired.getVideoTracks()[0];
      try {
        const capabilities = cameraTrack?.getCapabilities?.() || {};
        const continuous = {};
        ['focusMode', 'exposureMode', 'whiteBalanceMode'].forEach(key => {
          if (capabilities[key]?.includes('continuous')) continuous[key] = 'continuous';
        });
        if (Object.keys(continuous).length) void cameraTrack.applyConstraints({ advanced: [continuous] }).catch(() => {});
      } catch (_) { /* Optional camera controls must never block the preview. */ }
      acquired.getVideoTracks().forEach(track => track.addEventListener('ended', () => {
        if (token === generation && ['starting', 'searching', 'stable', 'capturing'].includes(state)) cameraError(null, true);
      }, { once: true }));
      setState('starting');
      startingAt = performance.now();
      const observeFrame = () => {
        frameCallback = video.requestVideoFrameCallback(() => {
          if (token !== generation || !stream) return;
          if (!stream.getVideoTracks()[0]?.muted) {
            deliveredFrames++;
            lastFrameAt = performance.now();
          }
          observeFrame();
        });
      };
      if (video.requestVideoFrameCallback) observeFrame();
      visualFrame = requestAnimationFrame(renderOverlay);
      // play() may stay pending when a device never delivers camera frames.
      // The frame watchdog below keeps retry available in that case too.
      void video.play().catch(error => { if (token === generation) cameraError(error); });
      tick(token);
    } catch (error) {
      if (token === generation) cameraError(error);
    }
  }
  function drawDetection(detection, opacity = 1) {
    const bounds = video.getBoundingClientRect();
    const dpr = Math.min(devicePixelRatio || 1, 2);
    const width = Math.max(1, Math.round(bounds.width * dpr));
    const height = Math.max(1, Math.round(bounds.height * dpr));
    if (overlay.width !== width || overlay.height !== height) {
      overlay.width = width;
      overlay.height = height;
    }
    overlayContext.clearRect(0, 0, width, height);
    overlayContext.globalAlpha = opacity;
    overlay.dataset.detected = detection ? 'true' : 'false';
    if (!detection || !video.videoWidth) return;
    const scale = Math.min(width / video.videoWidth, height / video.videoHeight);
    const dw = video.videoWidth * scale, dh = video.videoHeight * scale;
    const dx = (width - dw) / 2, dy = (height - dh) / 2;
    const polygon = (quad, thickness, dashed = false) => {
      overlayContext.beginPath();
      quad.forEach((point, index) => overlayContext[index ? 'lineTo' : 'moveTo'](dx + point.x * dw, dy + point.y * dh));
      overlayContext.closePath();
      overlayContext.lineWidth = thickness * dpr;
      overlayContext.setLineDash(dashed ? [6 * dpr, 5 * dpr] : []);
      overlayContext.strokeStyle = '#ffde00';
      overlayContext.stroke();
    };
    // Only observed geometry is drawn; no decorative squares or fixed frame.
    if (detection.source === 'yellow' && detection.pageQuad) polygon(detection.pageEdges?.flat() || detection.pageQuad, 1, true);
    polygon(detection.contour?.length > 4 ? detection.contour
      : detection.frameEdges?.flat() || detection.quad, 3);
    if (detection.markerEdges) detection.markerEdges.forEach(edges => polygon(edges.flat(), 1.4));
    else (detection.markers || []).forEach(quad => polygon(quad, 1.4));
  }
  function detect(source = video, previous = lastDetection) {
    const sourceWidth = source.videoWidth || source.width;
    const sourceHeight = source.videoHeight || source.height;
    if (!sourceWidth || !sourceHeight) return null;
    const scale = Math.min(1, 480 / Math.max(sourceWidth, sourceHeight));
    const width = Math.max(1, Math.round(sourceWidth * scale));
    const height = Math.max(1, Math.round(sourceHeight * scale));
    if (analysis.width !== width || analysis.height !== height) {
      analysis.width = width;
      analysis.height = height;
    }
    analysisContext.drawImage(source, 0, 0, width, height);
    return SP.LiveDetection.detect(analysisContext.getImageData(0, 0, width, height), previous);
  }
  const movement = (first, second) => !first || !second ? Infinity
    : Math.max(...first.map((point, index) => Math.hypot(point.x - second[index].x, point.y - second[index].y)));
  // An empty page has almost no interior contrast. Its observed printed rails
  // still reveal focus; normalize both measurements to their usable level.
  const focusScore = detection => Math.max((detection?.sharpness || 0) / 32,
    (detection?.edgeSharpness || 0) / 160);
  const captureReady = detection => detection?.source === 'yellow'
    && detection.confidence >= .78 && focusScore(detection) >= 1
    && detection.brightness >= 60 && detection.brightness <= 252
    && detection.coverage >= .16
    && detection.quad.every(point => point.x >= .008 && point.y >= .008 && point.x <= .992 && point.y <= .992);
  function clearVisualTracking() {
    if (visualFrame) cancelAnimationFrame(visualFrame);
    visualFrame = 0;
    visualFrameAt = 0;
    visualTarget = visualDetection = null;
    drawDetection(null);
  }
  const mapGeometry = (previous, next, mapPoint) => {
    const map = (old, points) => points?.map((point, index) => Array.isArray(point)
      ? map(old?.[index], point)
      : old?.length === points.length ? mapPoint(old[index], point) : { ...point });
    const result = { ...next };
    ['quad', 'pageQuad', 'frameEdges', 'pageEdges', 'markers', 'markerEdges'].forEach(key => {
      result[key] = map(previous?.[key], next[key]);
    });
    // The outline and its edge samples must describe the exact same curve.
    result.contour = result.frameEdges?.flat() || map(previous?.contour, next.contour);
    return result;
  };
  function observeVisual(next, now) {
    const previous = lastDetection;
    if (!visualTarget || !previous || previous.source !== next.source
      || now - lastObservedAt > 350 || movement(previous.quad, next.quad) > .16) {
      visualTarget = visualDetection = mapGeometry(null, next, (_, point) => ({ ...point }));
      visualResponse = 100;
      drawDetection(visualDetection);
      return;
    }
    // Estimate one common translation/rotation/scale from the measured corners.
    // Move the complete outline with it, then damp only the remaining local
    // shape change. Independently smoothing every rail used to make the page
    // and the bottom markers visibly pull apart during normal hand movement.
    const center = quad => quad.reduce((sum, point) => ({ x: sum.x + point.x / 4, y: sum.y + point.y / 4 }), { x: 0, y: 0 });
    const before = center(previous.quad), after = center(next.quad);
    let dot = 0, cross = 0, span = 0;
    previous.quad.forEach((point, i) => {
      const x = point.x - before.x, y = point.y - before.y;
      const u = next.quad[i].x - after.x, v = next.quad[i].y - after.y;
      dot += x * u + y * v;
      cross += x * v - y * u;
      span += x * x + y * y;
    });
    const a = span > 1e-6 ? dot / span : 1, b = span > 1e-6 ? cross / span : 0;
    const transform = point => ({ x: after.x + a * (point.x - before.x) - b * (point.y - before.y),
      y: after.y + b * (point.x - before.x) + a * (point.y - before.y) });
    const commonMotion = Math.max(...previous.quad.map(point => {
      const moved = transform(point);
      return Math.hypot(moved.x - point.x, moved.y - point.y);
    }));
    const speed = commonMotion / Math.max(16, now - lastObservedAt);
    visualResponse = Math.max(35, Math.min(110, 110 - speed * 350000));
    const motionWeight = Math.min(1, .35 + commonMotion / .012);
    visualTarget = mapGeometry(visualTarget, next, (point, measured) => {
      const moved = transform(point);
      const x = point.x + (moved.x - point.x) * motionWeight;
      const y = point.y + (moved.y - point.y) * motionWeight;
      return { x: x + (measured.x - x) * .32, y: y + (measured.y - y) * .32 };
    });
  }
  function renderOverlay(now) {
    visualFrame = 0;
    if (!stream || root.hidden) return;
    const age = now - lastObservedAt;
    if (visualTarget && age <= 350 && root.dataset.cameraReady === 'true') {
      const elapsed = visualFrameAt ? Math.max(0, now - visualFrameAt) : 16;
      const alpha = 1 - Math.exp(-Math.min(elapsed, 64) / visualResponse);
      visualDetection = mapGeometry(visualDetection, visualTarget, (point, target) => ({
        x: point.x + (target.x - point.x) * alpha,
        y: point.y + (target.y - point.y) * alpha
      }));
      drawDetection(visualDetection, Math.min(1, (350 - age) / 190));
    } else drawDetection(null);
    visualFrameAt = now;
    visualFrame = requestAnimationFrame(renderOverlay);
  }
  async function bufferFrame(detection, token, now) {
    if (buffering || now - lastBufferedAt < 220) return;
    const epoch = bufferEpoch;
    buffering = true;
    lastBufferedAt = now;
    let canvas;
    try {
      canvas = snapshot();
      const blob = await encode(canvas);
      if (token !== generation || epoch !== bufferEpoch || !['searching', 'stable'].includes(state)) return;
      bufferedFrames.push({ blob, detection, score: focusScore(detection), at: now,
        width: canvas.width, height: canvas.height });
      bufferedFrames = bufferedFrames.filter(frame => now - frame.at <= 1200).slice(-3);
    } catch (_) { /* A failed reserve frame does not interrupt the live feed. */ }
    finally {
      if (canvas) canvas.width = canvas.height = 0;
      buffering = false;
    }
  }
  const currentFrameTime = () => video.requestVideoFrameCallback ? deliveredFrames
    : video.getVideoPlaybackQuality?.().totalVideoFrames || video.currentTime;
  function tick(token) {
    if (token !== generation || !['starting', 'searching', 'stable'].includes(state)) return;
    if (document.hidden) { pauseCamera(); return; }
    const now = performance.now();
    const frameTime = currentFrameTime();
    const track = stream?.getVideoTracks()[0];
    const freshFrame = video.readyState >= 2 && video.videoWidth > 0 && frameTime > 0
      && frameTime !== lastFrameTime && track?.readyState === 'live' && !track.muted;
    if (freshFrame && !video.requestVideoFrameCallback) lastFrameAt = now;
    if (!freshFrame && now - (lastFrameAt || startingAt) > (lastFrameAt ? 5000 : 12000)) {
      cameraError(null, true);
      return;
    }
    if (track?.muted || (lastFrameAt && now - lastFrameAt > 750)) {
      resetStability();
      drawDetection(null);
      root.dataset.cameraReady = 'false';
      setState('starting');
      status('Waiting for camera…', 'Keep the camera available, or choose a photo.');
    }
    try {
      // Never count repeated frames of a stalled video as stability evidence.
      if (freshFrame) {
        const previousFrameTime = lastFrameTime;
        lastFrameTime = frameTime;
        // A single decoded frame is not yet a live preview.
        if (previousFrameTime < 0) {
          timer = setTimeout(() => tick(token), 120);
          return;
        }
        root.dataset.cameraReady = 'true';
        const detection = detect();
        if (detection) {
          observeVisual(detection, now);
          lastDetection = detection;
          lastObservedAt = now;
        } else if (now - lastObservedAt > 350) lastDetection = null;
        // Brief misses retain the observed outline, but never count as a
        // qualified frame or authorize capture from an old camera image.
        window.dispatchEvent(new CustomEvent('scanner-live-detection', { detail: detection }));
        if (captureReady(detection)) {
          if (movement(stableAnchor, detection.quad) > .025 || now - lastAnalysisTime > 600) {
            resetStability();
            stableAnchor = detection.quad;
            stableSince = now;
          }
          lastAnalysisTime = now;
          lastQualifiedAt = now;
          stableFrames++;
          void bufferFrame(detection, token, now);
          const progress = Math.min(1, (now - stableSince) / 800, stableFrames / 5);
          $('liveProgress').value = progress;
          setState('stable');
          status('Hold steady…', 'The yellow frame is aligned. Capturing the clearest frame automatically.');
          if (progress >= 1) { void capture(true); return; }
        } else if (stableAnchor && now - lastQualifiedAt <= 350
          && (!detection || (detection.source === 'yellow' && movement(stableAnchor, detection.quad) <= .025))) {
          // Autofocus and yellow-border detection can fluctuate for one or two
          // frames. Pause progress rather than restarting the whole hold.
          lastAnalysisTime = now;
          setState('stable');
          status('Hold steady…', 'Waiting for a clear frame.');
        } else {
          resetStability();
          setState('searching');
          status(detection ? detection.source === 'yellow' ? 'Keep the page clear and fully in view' : 'Page found · looking for yellow marks' : 'Find the yellow frame',
            detection && focusScore(detection) < 1 ? 'Hold still and let the camera focus, or capture now to review manually.'
              : 'Include all four edges and the bottom squares. You can also capture now.');
        }
      }
    } catch (error) {
      resetStability();
      visualTarget = visualDetection = null;
      lastDetection = null;
      lastObservedAt = 0;
      drawDetection(null);
      setState('searching');
      status('Automatic detection unavailable', 'You can capture now and check the page before adding it.');
    }
    // Analysis has a bounded cadence; drawing runs independently on animation
    // frames. Subtract detector time instead of adding it to every interval.
    timer = setTimeout(() => tick(token), Math.max(25, 80 - (performance.now() - now)));
  }
  const encode = canvas => new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error('Image encoding took too long')), 6000);
    canvas.toBlob(blob => {
      clearTimeout(timeout);
      if (blob?.size) resolve(blob);
      else reject(new Error('Could not encode this scan'));
    }, 'image/jpeg', .94);
  });
  function snapshot() {
    const track = stream?.getVideoTracks()[0];
    // Analysis pauses during the capture burst. Older WebViews need their
    // frame clock sampled here too, especially when JPEG encoding is slow.
    if (!video.requestVideoFrameCallback && video.readyState >= 2
      && track?.readyState === 'live' && !track.muted) {
      const frameTime = currentFrameTime();
      if (frameTime > 0 && frameTime !== lastFrameTime) {
        lastFrameTime = frameTime;
        lastFrameAt = performance.now();
      }
    }
    if (!track || track.readyState !== 'live' || track.muted
      || !lastFrameAt || performance.now() - lastFrameAt > 750) {
      throw new Error('The camera image stopped. Open the camera and try again.');
    }
    const canvas = document.createElement('canvas');
    const scale = Math.min(1, 2560 / Math.max(video.videoWidth, video.videoHeight));
    canvas.width = Math.round(video.videoWidth * scale);
    canvas.height = Math.round(video.videoHeight * scale);
    if (!canvas.width || !canvas.height) throw new Error('Wait for the camera image, then try again.');
    canvas.getContext('2d').drawImage(video, 0, 0, canvas.width, canvas.height);
    return canvas;
  }
  async function capture(automatic = false) {
    if (!['searching', 'stable'].includes(state)) return;
    const token = generation;
    const anchor = lastDetection?.quad;
    const reserve = automatic ? bufferedFrames.filter(frame => performance.now() - frame.at <= 1200
      && movement(anchor, frame.detection.quad) <= .025) : [];
    const captured = [...reserve];
    setState('capturing');
    clearTimeout(timer);
    status('Capturing…', 'Keep still for a moment.');
    try {
      // Include recent clear frames from before the shutter. A focus/exposure
      // adjustment immediately after detection must not lose the good scan.
      const attempts = Math.max(1, 3 - captured.length);
      let captureError;
      for (let index = 0; index < attempts; index++) {
        if (index) await new Promise(resolve => setTimeout(resolve, 130));
        if (token !== generation) return;
        let canvas;
        try {
          canvas = snapshot();
          const detection = detect(canvas, lastDetection);
          if (automatic && (!captureReady(detection) || movement(anchor, detection.quad) > .03)) continue;
          const blob = await encode(canvas);
          if (token !== generation) return;
          captured.push({ blob, detection, score: focusScore(detection), width: canvas.width, height: canvas.height });
        } catch (error) { captureError = error; }
        finally { if (canvas) canvas.width = canvas.height = 0; }
      }
      if (token !== generation) return;
      if (!captured.length) {
        if (captureError) throw captureError;
        resetStability();
        setState('searching');
        status('The page moved', 'Hold steady while the yellow frame is visible.');
        tick(token);
        return;
      }
      candidates = captured.sort((first, second) => second.score - first.score).slice(0, 3)
        .map(candidate => ({ ...candidate, url: URL.createObjectURL(candidate.blob) }));
      stopCamera();
      showAlternatives();
      await reviewCandidate(0, token);
    } catch (error) {
      if (token === generation) {
        stopCamera();
        setState('error');
        $('liveRetryBtn').hidden = false;
        status('Could not capture the page', error.message || 'Try again or choose a photo.');
      }
    }
  }
  function showAlternatives() {
    $('liveAlternatives').replaceChildren();
    candidates.forEach((candidate, index) => {
      if (candidates.length < 2) return;
      const button = document.createElement('button');
      button.type = 'button';
      button.setAttribute('aria-label', index === 0 ? 'Review clearest frame' : `Review alternative frame ${index + 1}`);
      const image = document.createElement('img');
      image.src = candidate.url;
      image.alt = '';
      button.append(image);
      button.onclick = () => { if (!submitting) void reviewCandidate(index, generation); };
      $('liveAlternatives').append(button);
    });
  }
  const decode = blob => new Promise((resolve, reject) => {
    const image = new Image();
    const url = URL.createObjectURL(blob);
    const finish = () => {
      clearTimeout(timeout);
      image.onload = image.onerror = null;
      URL.revokeObjectURL(url);
    };
    const timeout = setTimeout(() => { finish(); reject(new Error('Image decode timed out')); }, 6000);
    image.onload = () => {
      try {
        const canvas = document.createElement('canvas');
        const scale = Math.min(1, 2560 / Math.max(image.naturalWidth, image.naturalHeight));
        canvas.width = Math.round(image.naturalWidth * scale);
        canvas.height = Math.round(image.naturalHeight * scale);
        canvas.originalWidth = image.naturalWidth;
        canvas.originalHeight = image.naturalHeight;
        if (!canvas.width || !canvas.height) throw new Error('This image is empty');
        // Copy pixels before releasing the temporary decode URL. The preview
        // and alternatives keep their own URLs for their entire visible life.
        canvas.getContext('2d').drawImage(image, 0, 0, canvas.width, canvas.height);
        finish();
        resolve(canvas);
      } catch (error) { finish(); reject(error); }
    };
    image.onerror = () => { finish(); reject(new Error('This image could not be opened')); };
    image.src = url;
  });
  async function reviewCandidate(index, token) {
    processingController?.abort();
    const controller = processingController = new AbortController();
    const selected = ++selection;
    const candidate = candidates[index];
    if (!candidate) return;
    result = null;
    setState('processing');
    $('liveCameraView').hidden = true;
    $('liveReview').hidden = false;
    $('liveAddBtn').disabled = true;
    $('liveAddBtn').textContent = embedded ? 'Add page to document' : 'Save scan';
    $('liveReviewStatus').textContent = 'Straightening the page…';
    [...$('liveAlternatives').children].forEach((button, i) => button.setAttribute('aria-pressed', String(i === index)));
    const current = () => token === generation && selected === selection;
    const preview = $('livePreview');
    const previousUrl = previewUrl;
    previewUrl = '';
    preview.hidden = true;
    preview.onload = () => { if (current()) preview.hidden = false; };
    preview.onerror = () => { if (current()) preview.hidden = true; };
    preview.src = candidate.url;
    if (preview.complete && preview.naturalWidth) preview.hidden = false;
    if (previousUrl) URL.revokeObjectURL(previousUrl);
    let deadline;
    let original = null;
    const checkProcessing = () => {
      if (controller.signal.aborted) throw new DOMException('Scan cancelled', 'AbortError');
    };
    const work = async () => {
      let source, output;
      try {
        source = await decode(candidate.blob);
        original = { blob: candidate.blob, width: source.originalWidth, height: source.originalHeight, name: 'Scanned page' };
        checkProcessing();
        // Let the original photo paint before starting detailed correction.
        await new Promise(resolve => requestAnimationFrame(() => setTimeout(resolve, 0)));
        checkProcessing();
        if (current()) $('liveReviewStatus').textContent = 'Finding page edges…';
        const detection = SP.Lightweight.detectPageAsync
          ? await SP.Lightweight.detectPageAsync(source, { signal: controller.signal, liveDetection: candidate.detection })
          : SP.Lightweight.detectPage(source, { liveDetection: candidate.detection });
        checkProcessing();
        if (detection.quality?.valid === false) {
          throw new Error(detection.quality.reason || 'The page geometry could not be verified');
        }
        const precise = ['stencil', 'marker-guided'].includes(detection.method) && detection.confidence >= .55;
        const liveQuad = candidate.detection?.pageQuad;
        const quad = precise ? detection.pageQuad : liveQuad?.map(point => ({ x: point.x * source.width, y: point.y * source.height }));
        if (current()) $('liveReviewStatus').textContent = 'Straightening the page…';
        output = quad ? await SP.Lightweight.warp(source, quad, precise ? detection.frame : null,
          { signal: controller.signal }) : source;
        checkProcessing();
        if (current()) $('liveReviewStatus').textContent = 'Finishing the scan…';
        if (precise) await SP.Lightweight.correctColors(output,
          { useStencil: true, preciseStencil: true, signal: controller.signal });
        checkProcessing();
        const blob = await encode(output);
        checkProcessing();
        const verified = await decode(blob);
        verified.width = verified.height = 0;
        checkProcessing();
        return { blob, width: output.width, height: output.height, name: 'Scanned page', corrected: !!quad };
      } finally {
        // A superseded async job owns its canvases until it actually settles.
        // A timeout must not zero a canvas that a warp is still reading.
        if (output && output !== source) output.width = output.height = 0;
        if (source) source.width = source.height = 0;
      }
    };
    try {
      const processed = await Promise.race([work(), new Promise((resolve, reject) => {
        deadline = setTimeout(() => {
          controller.abort();
          reject(new Error('Page correction took too long'));
        }, 20000);
      })]);
      if (!current()) return;
      result = processed;
      previewUrl = URL.createObjectURL(processed.blob);
      preview.src = previewUrl;
      preview.classList.remove('scan-reveal');
      void preview.offsetWidth;
      preview.classList.add('scan-reveal');
      $('liveAddBtn').disabled = false;
      $('liveReviewStatus').textContent = processed.corrected
        ? 'Check the scan. You can choose another frame or scan again.'
        : 'Page edges were not clear. Check the full photo, or scan again.';
      setState('review');
    } catch (error) {
      if (current()) {
        result = original;
        $('liveAddBtn').disabled = !result;
        $('liveReviewStatus').textContent = result
          ? 'Original photo ready. Page correction was unavailable; check the photo before adding it.'
          : error.message || 'Could not process this frame. Choose another or scan again.';
        setState('review');
      }
    } finally {
      clearTimeout(deadline);
      if (processingController === controller) processingController = null;
    }
  }
  async function importPhoto(file) {
    if (!file || submitting) return;
    const token = ++generation;
    stopCamera();
    clearReview();
    if (!file.type.startsWith('image/')) { cameraError(); return; }
    candidates = [{ blob: file, url: URL.createObjectURL(file), detection: null, score: 0 }];
    await reviewCandidate(0, token);
  }
  async function addPage() {
    if (!result || submitting || state !== 'review') return;
    if (!embedded) {
      const link = document.createElement('a');
      link.href = previewUrl || $('livePreview').src;
      link.download = 'scan.jpg';
      link.click();
      return;
    }
    submitting = true;
    $('liveAddBtn').disabled = true;
    $('liveRetakeBtn').disabled = true;
    $('liveCloseBtn').disabled = true;
    $('liveReviewStatus').textContent = 'Adding page to document…';
    if (!post('ihn-scanner-pages', { pages: [result], documentName, stencil: false })) {
      finishSubmission(false, 'The document is no longer available.');
    }
  }
  function finishSubmission(ok, message) {
    submitting = false;
    $('liveRetakeBtn').disabled = false;
    $('liveCloseBtn').disabled = false;
    if (ok) close();
    else {
      $('liveAddBtn').disabled = !result;
      $('liveReviewStatus').textContent = message || 'Could not add the page. Your scan is still here; try again.';
    }
  }
  function pauseCamera(includePermission = false) {
    if (root.hidden || !['requesting', 'starting', 'searching', 'stable', 'capturing'].includes(state)) return;
    // A permission dialog is not a cancelled camera request. If it resolves
    // while hidden, open() releases its stream and resumes once visible.
    if (state === 'requesting' && !includePermission) return;
    generation++;
    stopCamera();
    clearReview();
    setState('error');
    $('liveRetryBtn').hidden = false;
    status('Camera paused', 'Open the camera to continue scanning.');
  }
  $('liveCloseBtn').onclick = () => { if (close()) post('ihn-scanner-close'); };
  $('liveRetryBtn').onclick = open;
  $('liveRetakeBtn').onclick = open;
  $('liveCaptureBtn').onclick = () => capture(false);
  $('liveImportBtn').onclick = () => $('liveFileInput').click();
  $('liveFileInput').onchange = event => {
    const file = event.target.files?.[0];
    event.target.value = '';
    if (file) void importPhoto(file);
  };
  $('liveAddBtn').onclick = addPage;
  // Replace the native still-photo picker with the same live feed everywhere.
  $('btnSourceCamera').onclick = open;
  $('btnLandingStart').onclick = open;
  window.addEventListener('message', event => {
    if (event.origin !== location.origin || event.source !== parent) return;
    const data = event.data;
    if (!data || data.source !== 'inhouse-notes' || data.sessionId !== sessionId) return;
    if (data.type === 'ihn-scanner-init') documentName = String(data.documentName || 'Scanned page').slice(0, 160);
    if (data.type === 'ihn-scanner-result' && submitting) finishSubmission(data.ok === true, data.message);
  });
  document.addEventListener('visibilitychange', () => {
    if (document.hidden) pauseCamera();
    else if (resumeAfterPermission && !root.hidden) void open();
  });
  window.addEventListener('pagehide', () => {
    resumeAfterPermission = false;
    pauseCamera(true);
  });
  document.addEventListener('keydown', event => {
    if (root.hidden) return;
    if (event.key === 'Tab') {
      const controls = [...root.querySelectorAll('button:not(:disabled), [tabindex="0"]')]
        .filter(element => element.getClientRects().length);
      const first = controls[0], last = controls[controls.length - 1];
      if (!first) { event.preventDefault(); return; }
      if (!root.contains(document.activeElement) || (event.shiftKey && document.activeElement === first)
        || (!event.shiftKey && document.activeElement === last)) {
        event.preventDefault();
        (event.shiftKey ? last : first).focus();
      }
      return;
    }
    if (event.key !== 'Escape') return;
    event.preventDefault();
    event.stopImmediatePropagation();
    if (close()) post('ihn-scanner-close');
  }, true);
  new ResizeObserver(() => drawDetection(visualDetection && performance.now() - lastObservedAt <= 350 ? visualDetection : null)).observe(video);
  SP.LiveCamera = { open, close };
  if (embedded) void open();
})();
