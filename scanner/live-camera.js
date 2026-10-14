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
  let lastObservedAt = 0;
  let visualTarget = null;
  let visualDetection = null;
  let visualFrame = 0;
  let visualFrameAt = 0;
  let visualResponse = 100;
  let processingController = null;
  let candidates = [];
  let result = null;
  let previewUrl = '';
  let documentName = 'Scanned page';
  let submitting = false;
  let selection = 0;
  let selectedIndex = 0;
  let explicitSelection = false;
  let pendingCandidates = 0;
  let processingVisualFrame = 0;
  let lastFrameTime = -1;
  let deliveredFrames = 0;
  let frameCallback = 0;
  let lastFrameAt = 0;
  let startingAt = 0;
  let resumeAfterPermission = false;
  let previousFocus = null;
  let automaticCapture = false;
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
  const resetReadiness = () => { $('liveProgress').value = 0; };
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
    resetReadiness();
    overlayContext.clearRect(0, 0, overlay.width, overlay.height);
    overlay.dataset.detected = 'false';
  };
  const clearReview = () => {
    selection++;
    processingController?.abort();
    processingController = null;
    clearProcessingVisual();
    result = null;
    candidates.forEach(candidate => {
      URL.revokeObjectURL(candidate.url);
      if (candidate.resultUrl && candidate.resultUrl !== candidate.url) URL.revokeObjectURL(candidate.resultUrl);
      if (candidate.processingPreviewUrl) URL.revokeObjectURL(candidate.processingPreviewUrl);
    });
    candidates = [];
    pendingCandidates = 0;
    explicitSelection = false;
    selectedIndex = 0;
    previewUrl = '';
    $('livePreview').removeAttribute('src');
    $('livePreview').onload = $('livePreview').onerror = null;
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
      resetReadiness();
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
          // The live image already passed focus, exposure and frame checks.
          // Freeze it synchronously before another camera frame can replace it.
          // Phone movement after this point cannot invalidate the good photo.
          $('liveProgress').value = 1;
          setState('stable');
          void capture(true, detection);
          return;
        } else {
          resetReadiness();
          setState('searching');
          status(detection ? detection.source === 'yellow' ? 'Looking for a clear view' : 'Page found · looking for yellow marks' : 'Find the yellow frame',
            detection && focusScore(detection) < 1 ? 'The camera is focusing automatically.'
              : 'Include all four edges and the bottom squares.');
        }
      }
    } catch (error) {
      resetReadiness();
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
  function showFrozenCapture(canvas, token) {
    const thumbnail = document.createElement('canvas');
    const scale = Math.min(1, 720 / Math.max(canvas.width, canvas.height));
    thumbnail.width = Math.max(1, Math.round(canvas.width * scale));
    thumbnail.height = Math.max(1, Math.round(canvas.height * scale));
    try {
      thumbnail.getContext('2d').drawImage(canvas, 0, 0, thumbnail.width, thumbnail.height);
      const image = $('livePreview');
      previewUrl = thumbnail.toDataURL('image/jpeg', .85);
      image.onload = () => { if (token === generation) image.hidden = false; };
      image.src = previewUrl;
      image.hidden = false;
      $('liveCameraView').hidden = true;
      $('liveReview').hidden = false;
      $('liveAddBtn').disabled = true;
      $('liveReviewStatus').textContent = 'Preparing the scan…';
      clearProcessingVisual();
    } catch (_) { /* Optional thumbnail never replaces the full captured photo. */ }
    finally { thumbnail.width = thumbnail.height = 0; }
  }
  async function nextCaptureFrame(previousFrame, token) {
    const deadline = performance.now() + 180;
    while (token === generation && performance.now() < deadline) {
      await new Promise(resolve => setTimeout(resolve, 12));
      if (currentFrameTime() !== previousFrame) return true;
    }
    return false;
  }
  async function capture(automatic = false, triggerDetection = null) {
    if (!['searching', 'stable'].includes(state)) return;
    const token = generation;
    let frozenFirst, frozenFirstQuality;
    if (automatic) {
      try {
        frozenFirst = snapshot();
        const quality = frozenFirstQuality = SP.CaptureQuality.inspect(frozenFirst, triggerDetection || lastDetection);
        window.dispatchEvent(new CustomEvent('scanner-capture-quality', {detail:quality}));
        if (!quality.valid) {
          frozenFirst.width = frozenFirst.height = 0;
          resetReadiness();
          setState('searching');
          status('Focusing on the page…', 'Scanning automatically when the writing is sharp.');
          timer = setTimeout(() => tick(token), 80);
          return;
        }
      } catch (_) {
        if (frozenFirst) frozenFirst.width = frozenFirst.height = 0;
        setState('searching'); timer = setTimeout(() => tick(token), 80); return;
      }
    }
    automaticCapture = automatic;
    const anchor = (triggerDetection || lastDetection)?.quad;
    const encodings = [];
    let captureError;
    let previousFrame = currentFrameTime();
    setState('capturing');
    clearTimeout(timer);
    status('Page captured', 'Choosing the clearest photo automatically.');
    const saveFrame = (canvas, detection, photoQuality) => {
      const width = canvas.width, height = canvas.height;
      // Freeze all three images without waiting for each JPEG callback. The
      // first good image survives focus changes or motion during the burst.
      encodings.push(encode(canvas).then(blob => ({ blob, detection,
        score: focusScore(detection) * (photoQuality ? Math.min(1.5, photoQuality.clarity / .5) : 1), width, height
      })).catch(error => { captureError = error; return null; })
        .finally(() => { canvas.width = canvas.height = 0; }));
    };
    try {
      for (let index = 0; index < 3; index++) {
        if (index && !await nextCaptureFrame(previousFrame, token)) break;
        if (token !== generation) return;
        previousFrame = currentFrameTime();
        let canvas;
        try {
          canvas = index === 0 && frozenFirst ? frozenFirst : snapshot();
          frozenFirst = null;
          // This is the exact decoded frame analyzed by tick in this JS task;
          // re-running detection would discard it if autofocus changes later.
          const detection = index === 0 && triggerDetection ? triggerDetection : detect(canvas, lastDetection);
          const photoQuality = index === 0 && frozenFirstQuality ? frozenFirstQuality
            : detection?.source === 'yellow' ? SP.CaptureQuality.inspect(canvas, detection) : null;
          if (automatic && (!captureReady(detection) || movement(anchor, detection.quad) > .03 || !photoQuality?.valid)) continue;
          saveFrame(canvas, detection, photoQuality);
          if (index === 0) showFrozenCapture(canvas, token);
          canvas = null; // Encoding owns this frozen canvas until it settles.
        } catch (error) { captureError = error; }
        finally { if (canvas) canvas.width = canvas.height = 0; }
      }
      if (token !== generation) return;
      if (!encodings.length) {
        if (captureError) throw captureError;
        resetReadiness();
        setState('searching');
        status('Looking for a clear view', 'Scanning automatically when the page is clear.');
        tick(token);
        return;
      }
      // The photos are frozen; the user can move the phone while encoding and
      // page correction finish. No more camera frames are needed.
      stopCamera();
      const captured = (await Promise.all(encodings)).filter(Boolean);
      if (token !== generation) return;
      if (!captured.length) throw captureError || new Error('Could not encode this scan');
      candidates = captured.sort((first, second) => second.score - first.score)
        .map(candidate => ({ ...candidate, url: URL.createObjectURL(candidate.blob) }));
      showAlternatives();
      await reviewCandidates(token);
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
      button.setAttribute('aria-label', `Review frame ${index + 1}`);
      const image = document.createElement('img');
      image.src = candidate.url;
      image.alt = '';
      button.append(image);
      button.onclick = () => {
        if (!submitting && !button.disabled) { explicitSelection = true; reviewCandidate(index, generation); }
      };
      $('liveAlternatives').append(button);
    });
  }
  // Validation never allocates a second full-resolution canvas. The worker
  // owns the processing pixels; the review keeps only encoded, decoded images.
  const imageSize = (url, signal) => new Promise((resolve, reject) => {
    const image = new Image();
    let timeout;
    const finish = error => {
      clearTimeout(timeout);
      signal?.removeEventListener('abort', abort);
      image.onload = image.onerror = null;
      if (error) { image.src = ''; reject(error); }
      else resolve({ width: image.naturalWidth, height: image.naturalHeight });
    };
    const abort = () => finish(new DOMException('Scan cancelled', 'AbortError'));
    if (signal?.aborted) { abort(); return; }
    signal?.addEventListener('abort', abort, { once: true });
    timeout = setTimeout(() => finish(new Error('Image decode timed out')), 6000);
    image.onload = () => finish(image.naturalWidth && image.naturalHeight ? null : new Error('This image is empty'));
    image.onerror = () => finish(new Error('This image could not be opened'));
    image.src = url;
  });
  const processingStages = ['decoding', 'detection', 'dots', 'surface', 'warp', 'colors', 'template', 'encoding'];
  function clearProcessingVisual() {
    cancelAnimationFrame(processingVisualFrame);
    processingVisualFrame = 0;
    const canvas = $('liveProcessingOverlay');
    canvas.hidden = true;
    canvas.getContext('2d').clearRect(0, 0, canvas.width, canvas.height);
    delete canvas.dataset.mode;
    delete canvas.dataset.pointCount;
    $('liveProcessingSteps').hidden = true;
  }
  function processingStatus(candidate) {
    if (candidate.result || candidate.error) return 'Comparing scans…';
    const count = candidate.processingGeometry?.points?.length || 0;
    switch (candidate.processingStage) {
      case 'decoding': return 'Opening photo…';
      case 'detection': return 'Finding the page…';
      case 'dots': return count ? `${count} bullet points found` : 'Finding bullet points…';
      case 'surface':
      case 'warp': return count ? `Straightening ${count} bullet points…` : 'Straightening the page…';
      case 'colors': return 'Cleaning the page…';
      case 'template': return 'Applying the page template…';
      case 'encoding': return 'Preparing scan…';
      default: return candidates.length > 1 ? 'Processing photos and choosing the best scan…' : 'Straightening the page…';
    }
  }
  function updateProcessingSteps(candidate) {
    const steps = $('liveProcessingSteps');
    steps.hidden = !pendingCandidates || !!candidate.result || !!candidate.error;
    const phase = candidate.processingStage;
    const active = ['decoding', 'detection', 'dots'].includes(phase) ? 'dots'
      : ['surface', 'warp'].includes(phase) ? 'surface'
        : phase === 'colors' ? 'colors' : ['template', 'encoding'].includes(phase) ? 'template' : '';
    let beforeCurrent = !!active;
    [...steps.children].forEach(step => {
      const current = step.dataset.step === active;
      if (current) beforeCurrent = false;
      step.dataset.complete = String(beforeCurrent);
      if (current) step.setAttribute('aria-current', 'step');
      else step.removeAttribute('aria-current');
    });
  }
  function scheduleProcessingOverlay() {
    if (processingVisualFrame) return;
    processingVisualFrame = requestAnimationFrame(() => {
      processingVisualFrame = 0;
      drawProcessingOverlay();
    });
  }
  function drawProcessingOverlay() {
    const canvas = $('liveProcessingOverlay');
    const preview = $('livePreview');
    const candidate = candidates[selectedIndex];
    const warped = !!candidate?.processingPreviewUrl && previewUrl === candidate.processingPreviewUrl;
    const geometry = warped ? candidate?.processingWarpGeometry : candidate?.processingGeometry;
    // Geometry belongs to one capture and one displayed image. Never draw a
    // late worker's points over another thumbnail or the finished document.
    if (!pendingCandidates || candidate?.result || candidate?.error || !geometry?.points?.length
      || preview.hidden || !preview.complete || !preview.naturalWidth || !['dots', 'surface', 'warp', 'colors'].includes(candidate.processingStage)) {
      canvas.hidden = true;
      return;
    }
    if (!warped && previewUrl !== candidate.url) { canvas.hidden = true; return; }
    const width = preview.clientWidth, height = preview.clientHeight;
    if (!width || !height) { canvas.hidden = true; return; }
    const ratio = Math.min(2, devicePixelRatio || 1);
    canvas.width = Math.round(width * ratio);
    canvas.height = Math.round(height * ratio);
    const context = canvas.getContext('2d');
    context.setTransform(ratio, 0, 0, ratio, 0, 0);
    const scale = Math.min(width / preview.naturalWidth, height / preview.naturalHeight);
    const imageWidth = preview.naturalWidth * scale, imageHeight = preview.naturalHeight * scale;
    const offsetX = (width - imageWidth) / 2, offsetY = (height - imageHeight) / 2;
    const location = point => {
      const x = point.x / geometry.width;
      const y = point.y / geometry.height;
      return Number.isFinite(x) && Number.isFinite(y) && x >= 0 && x <= 1 && y >= 0 && y <= 1
        ? { x: offsetX + x * imageWidth, y: offsetY + y * imageHeight } : null;
    };
    const points = geometry.points.map(location);
    context.save();
    context.beginPath(); context.rect(offsetX, offsetY, imageWidth, imageHeight); context.clip();
    // Each run contains only observed neighbours. Missing parts of the sheet
    // are never filled with an invented grid or connected across long gaps.
    const guides = (runs, color, locations = points) => {
      context.strokeStyle = color;
      context.lineWidth = .8;
      context.beginPath();
      (runs || []).forEach(run => {
        let connected = false;
        (run.points || []).forEach(index => {
          const point = locations[index];
          if (!point) { connected = false; return; }
          if (connected) context.lineTo(point.x, point.y);
          else { context.moveTo(point.x, point.y); connected = true; }
        });
      });
      context.stroke();
    };
    if (warped) {
      // These faint straight segments are the target positions; the bright
      // dots and coloured paths below remain the measured warped positions.
      const targets = geometry.points.map(point => Number.isFinite(point.targetX) && Number.isFinite(point.targetY)
        ? { x: offsetX + point.targetX / geometry.width * imageWidth,
          y: offsetY + point.targetY / geometry.height * imageHeight } : null);
      context.setLineDash([2, 3]);
      guides(geometry.rows, '#ffffff60', targets);
      guides(geometry.columns, '#ffffff40', targets);
      context.setLineDash([]);
    }
    guides(geometry.rows, '#f4d84b99');
    guides(geometry.columns, '#70e0e680');
    context.fillStyle = '#fff4a7';
    context.beginPath();
    points.forEach(point => {
      if (!point) return;
      context.moveTo(point.x + 1.4, point.y);
      context.arc(point.x, point.y, 1.4, 0, Math.PI * 2);
    });
    context.fill();
    context.restore();
    canvas.dataset.mode = warped ? 'corrected' : 'observed';
    canvas.dataset.pointCount = String(points.filter(Boolean).length);
    canvas.hidden = false;
  }
  function receiveProcessingProgress(candidate, index, token, current, stage, detail) {
    if (!current() || candidate.processingComplete || !processingStages.includes(stage)) return;
    candidate.processingStage = stage;
    if (detail?.points?.length && detail.width > 0 && detail.height > 0 && ['dots', 'surface'].includes(stage)) {
      candidate.processingGeometry = detail;
    }
    if (stage === 'warp' && detail?.previewBlob instanceof Blob) {
      if (candidate.processingPreviewUrl) URL.revokeObjectURL(candidate.processingPreviewUrl);
      candidate.processingPreviewUrl = URL.createObjectURL(detail.previewBlob);
      candidate.processingWarpGeometry = detail.space === 'corrected' && detail.points?.length ? detail : null;
    }
    if (index === selectedIndex) reviewCandidate(index, token);
  }
  if (typeof ResizeObserver === 'function') new ResizeObserver(scheduleProcessingOverlay).observe($('livePreview'));
  else window.addEventListener('resize', scheduleProcessingOverlay);
  function reviewCandidate(index, token) {
    const candidate = candidates[index];
    if (!candidate || token !== generation) return;
    selectedIndex = index;
    const selected = ++selection;
    if (submitting) return;
    const waiting = pendingCandidates > 0 && !(candidate.processingComplete && candidate.result?.corrected);
    result = waiting || !candidate.result?.corrected ? null : candidate.result;
    setState(waiting ? 'processing' : 'review');
    $('liveCameraView').hidden = true;
    $('liveReview').hidden = false;
    $('liveAddBtn').disabled = !result;
    $('liveAddBtn').textContent = embedded ? 'Add page to document' : 'Save scan';
    $('liveReviewStatus').textContent = waiting
      ? processingStatus(candidate)
      : result ? pendingCandidates ? 'Scan ready. Checking the other photos…' : 'Check the scan.'
        : candidate.error || 'Page correction could not be verified. Scan again or choose a clearer photo.';
    if (waiting) updateProcessingSteps(candidate);
    else clearProcessingVisual();
    $('liveProcessingOverlay').hidden = true;
    [...$('liveAlternatives').children].forEach((button, i) => button.setAttribute('aria-pressed', String(i === index)));
    const current = () => token === generation && selected === selection;
    const preview = $('livePreview');
    const nextUrl = candidate.resultUrl || candidate.processingPreviewUrl || candidate.url;
    preview.onload = () => {
      if (current()) { preview.hidden = false; scheduleProcessingOverlay(); }
    };
    preview.onerror = () => {
      if (!current()) return;
      preview.hidden = true;
      // A diagnostic preview is optional; failure must not hide the original
      // or make a valid final result unavailable.
      if (nextUrl === candidate.processingPreviewUrl) {
        URL.revokeObjectURL(candidate.processingPreviewUrl);
        candidate.processingPreviewUrl = '';
        reviewCandidate(index, token);
      }
    };
    if (previewUrl === nextUrl) {
      if (preview.complete && preview.naturalWidth) preview.hidden = false;
      scheduleProcessingOverlay();
      return;
    }
    previewUrl = nextUrl;
    preview.hidden = true;
    preview.src = nextUrl;
    if (preview.complete && preview.naturalWidth) preview.hidden = false;
    scheduleProcessingOverlay();
    if (candidate.result?.corrected) {
      preview.classList.remove('scan-reveal');
      void preview.offsetWidth;
      preview.classList.add('scan-reveal');
    }
  }
  function candidateRank(candidate) {
    if (!candidate.result) return -1;
    const focus = Math.max(0, Number.isFinite(candidate.score) ? candidate.score : 0);
    if (!candidate.result.corrected || candidate.analysis?.quality?.valid === false) return .1 * focus / (focus + 1);
    const bounded = value => Math.max(0, Math.min(1, Number.isFinite(value) ? value : 0));
    const surface = candidate.analysis?.frame?.metricSurface;
    const diagnostics = surface?.diagnostics;
    const physical = candidate.analysis?.quality?.valid === true
      && surface?.type === 'developable-pinhole' && diagnostics?.validation > 0
      && Number.isFinite(diagnostics.pitch) && diagnostics.pitch > 0
      && [['rms', .14], ['p90', .22], ['p99', .4], ['borderP95', 1.15]].every(([key, limit]) =>
        Number.isFinite(diagnostics[key]) && diagnostics[key] >= 0 && diagnostics[key] <= diagnostics.pitch * limit);
    const registration = diagnostics?.rasterRegistration || diagnostics?.registration;
    const registered = physical && !!surface.registration && registration?.anchors >= 350
      && Number.isFinite(registration.maximumAnchorError) && registration.maximumAnchorError < .01;
    // Physical validation remains a prerequisite. Within verified photos,
    // compare the cleaned output too: exact anchor interpolation alone says
    // nothing about the grey shadows that survived colour correction.
    const tier = physical ? 2 : 1;
    const geometry = physical
      ? 1 - .45 * bounded(diagnostics.rms / diagnostics.pitch / .14)
        - .25 * bounded(diagnostics.p99 / diagnostics.pitch / .4)
        - .3 * bounded(diagnostics.borderP95 / diagnostics.pitch / 1.15)
      : .55;
    const brightness = candidate.detection?.brightness;
    const exposure = Number.isFinite(brightness) ? 1 - bounded(Math.abs(brightness - 210) / 150) : .5;
    const quality = candidate.outputQuality;
    const measuredPaper = quality?.paperSamples >= 100 && quality.confidence >= .5
      && Number.isFinite(quality.paperResidualFraction) && quality.paperResidualFraction >= 0;
    // Sampled paper excludes the dot cores and handwriting. An unknown
    // measurement stays neutral; it is never advertised as spotless paper.
    const cleanPaper = measuredPaper ? 1 - bounded(quality.paperResidualFraction / .08) : .5;
    return tier + (registered ? .25 : 0) + .4 * cleanPaper + .17 * geometry
      + .08 * focus / (focus + 1) + .03 * bounded(candidate.analysis?.confidence) + .02 * exposure;
  }
  async function reviewCandidates(token) {
    processingController?.abort();
    const controller = processingController = new AbortController();
    const batch = candidates;
    explicitSelection = false;
    selectedIndex = 0;
    pendingCandidates = batch.length;
    reviewCandidate(0, token);
    const current = () => token === generation && processingController === controller && !controller.signal.aborted;
    // Paint the original immediately, then start all three independent worker
    // pipelines together. Thumbnail changes only switch cached images.
    await new Promise(resolve => requestAnimationFrame(() => setTimeout(resolve, 0)));
    if (!current()) return;
    await Promise.allSettled(batch.map(async (candidate, index) => {
      let outputUrl = '';
      try {
        const processed = await SP.Lightweight.processPageAsync(candidate.blob,
          { signal: controller.signal, liveDetection: candidate.detection, previewProgress: true,
            onProgress: (stage, detail) => receiveProcessingProgress(candidate, index, token, current, stage, detail) });
        if (!current()) return;
        if (processed.detection?.quality?.valid === false && processed.corrected) {
          throw new Error('The page geometry could not be verified');
        }
        const displayBlob = processed.previewBlob || processed.blob;
        outputUrl = displayBlob === candidate.blob ? candidate.url : URL.createObjectURL(displayBlob);
        const dimensions = await imageSize(outputUrl, controller.signal);
        if (!current()) return;
        if (processed.previewBlob) {
          // The visible composite and the ink asset have separate lifetimes.
          // Verify the asset we will insert too, before offering this capture.
          const inkUrl = URL.createObjectURL(processed.blob);
          try {
            const inkDimensions = await imageSize(inkUrl, controller.signal);
            if (inkDimensions.width !== dimensions.width || inkDimensions.height !== dimensions.height) {
              throw new Error('The scan layers do not match');
            }
          } finally { URL.revokeObjectURL(inkUrl); }
          if (!current()) return;
        }
        candidate.result = { blob: processed.blob, ...dimensions, name: 'Scanned page', corrected: !!processed.corrected,
          inkLayer: processed.inkLayer === true, templateKind: processed.templateKind };
        candidate.analysis = processed.detection;
        candidate.outputQuality = processed.outputQuality;
        candidate.resultUrl = outputUrl;
        outputUrl = '';
      } catch (error) {
        if (!current()) return;
        candidate.error = error.message;
        try {
          const dimensions = await imageSize(candidate.url, controller.signal);
          if (!current()) return;
          candidate.result = { blob: candidate.blob, ...dimensions, name: 'Scanned page', corrected: false };
          candidate.resultUrl = candidate.url;
        } catch (_) { /* A corrupt original remains unavailable for insertion. */ }
      } finally {
        if (outputUrl && outputUrl !== candidate.url) URL.revokeObjectURL(outputUrl);
        if (current()) {
          candidate.processingComplete = true;
          pendingCandidates--;
          const image = $('liveAlternatives').children[index]?.querySelector('img');
          if (image && candidate.resultUrl) image.src = candidate.resultUrl;
          const alternative = $('liveAlternatives').children[index];
          if (alternative) {
            alternative.disabled = !candidate.result?.corrected;
            alternative.setAttribute('aria-label', candidate.result?.corrected ? `Review frame ${index + 1}` : 'Photo could not be corrected');
          }
          const processingUrl = candidate.processingPreviewUrl;
          candidate.processingPreviewUrl = '';
          if (!explicitSelection || (batch[selectedIndex]?.processingComplete && !batch[selectedIndex]?.result?.corrected)) {
            explicitSelection = false;
            const ready = batch.map((value, i) => ({ value, i })).filter(({ value }) => value.processingComplete && value.result?.corrected);
            if (ready.length) selectedIndex = ready.reduce((best, next) => candidateRank(next.value) > candidateRank(best.value) ? next : best).i;
          }
          reviewCandidate(selectedIndex, token);
          if (processingUrl) URL.revokeObjectURL(processingUrl);
          candidate.processingGeometry = candidate.processingWarpGeometry = null;
        }
      }
    }));
    if (!current()) return;
    if (!explicitSelection) {
      selectedIndex = batch.reduce((best, candidate, index) =>
        candidateRank(candidate) > candidateRank(batch[best]) ? index : best, 0);
    }
    reviewCandidate(selectedIndex, token);
    processingController = null;
    if (automaticCapture && !batch.some(candidate => candidate.result?.corrected)) {
      $('liveAddBtn').disabled = true;
      $('liveReviewStatus').textContent = 'Looking for a clearer scan…';
      timer = setTimeout(() => { if (token === generation && !root.hidden && !document.hidden && !submitting) void open(); }, 350);
    }
  }
  async function importPhoto(file) {
    if (!file || submitting) return;
    const token = ++generation;
    stopCamera();
    clearReview();
    if (!file.type.startsWith('image/')) { cameraError(); return; }
    automaticCapture = false;
    candidates = [{ blob: file, url: URL.createObjectURL(file), detection: null, score: 0 }];
    await reviewCandidates(token);
  }
  async function addPage() {
    if (!result || submitting || state !== 'review') return;
    if (!embedded) {
      const link = document.createElement('a');
      link.href = previewUrl || $('livePreview').src;
      link.download = result.inkLayer ? 'scan.png' : 'scan.jpg';
      link.click();
      return;
    }
    submitting = true;
    processingController?.abort();
    processingController = null;
    pendingCandidates = 0;
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
