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
    $('liveProgress').value = 0;
  };
  const stopCamera = () => {
    clearTimeout(timer);
    timer = 0;
    if (frameCallback) video.cancelVideoFrameCallback?.(frameCallback);
    frameCallback = 0;
    if (stream) stream.getTracks().forEach(track => track.stop());
    stream = null;
    video.srcObject = null;
    lastDetection = null;
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
    result = null;
    candidates.forEach(candidate => URL.revokeObjectURL(candidate.url));
    candidates = [];
    if (previewUrl) URL.revokeObjectURL(previewUrl);
    previewUrl = '';
    $('livePreview').removeAttribute('src');
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
      // play() may stay pending when a device never delivers camera frames.
      // The frame watchdog below keeps retry available in that case too.
      void video.play().catch(error => { if (token === generation) cameraError(error); });
      tick(token);
    } catch (error) {
      if (token === generation) cameraError(error);
    }
  }
  function drawDetection(detection) {
    const bounds = video.getBoundingClientRect();
    const dpr = Math.min(devicePixelRatio || 1, 2);
    const width = Math.max(1, Math.round(bounds.width * dpr));
    const height = Math.max(1, Math.round(bounds.height * dpr));
    if (overlay.width !== width || overlay.height !== height) {
      overlay.width = width;
      overlay.height = height;
    }
    overlayContext.clearRect(0, 0, width, height);
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
    if (detection.source === 'yellow' && detection.pageQuad) polygon(detection.pageQuad, 1, true);
    polygon(detection.quad, 3);
    (detection.markers || []).forEach(quad => polygon(quad, 1.4));
  }
  function detect(source = video) {
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
    return SP.LiveDetection.detect(analysisContext.getImageData(0, 0, width, height));
  }
  const movement = (first, second) => !first || !second ? Infinity
    : Math.max(...first.map((point, index) => Math.hypot(point.x - second[index].x, point.y - second[index].y)));
  const captureReady = detection => detection?.source === 'yellow'
    && detection.confidence >= .78 && detection.sharpness >= 45
    && detection.brightness >= 60 && detection.brightness <= 252
    && detection.coverage >= .16
    && (detection.pageQuad || detection.quad).every(point => point.x >= .008 && point.y >= .008 && point.x <= .992 && point.y <= .992);
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
        lastDetection = detection;
        drawDetection(detection);
        window.dispatchEvent(new CustomEvent('scanner-live-detection', { detail: detection }));
        if (captureReady(detection)) {
          if (movement(stableAnchor, detection.quad) > .012 || now - lastAnalysisTime > 400) {
            stableAnchor = detection.quad;
            stableSince = now;
            stableFrames = 0;
          }
          lastAnalysisTime = now;
          stableFrames++;
          const progress = Math.min(1, (now - stableSince) / 1000);
          $('liveProgress').value = progress;
          setState('stable');
          status('Hold steady…', 'The yellow frame is aligned. Capturing the clearest frame automatically.');
          if (progress >= 1 && stableFrames >= 7) { void capture(true); return; }
        } else {
          resetStability();
          setState('searching');
          status(detection ? detection.source === 'yellow' ? 'Keep the page clear and fully in view' : 'Page found · looking for yellow marks' : 'Find the yellow frame',
            detection?.sharpness < 45 ? 'Hold still and let the camera focus, or capture now to review manually.'
              : 'Include all four edges and the bottom squares. You can also capture now.');
        }
      }
    } catch (error) {
      resetStability();
      drawDetection(null);
      setState('searching');
      status('Automatic detection unavailable', 'You can capture now and check the page before adding it.');
    }
    timer = setTimeout(() => tick(token), 120);
  }
  const encode = canvas => new Promise((resolve, reject) => canvas.toBlob(blob => blob ? resolve(blob) : reject(new Error('Could not encode this scan')), 'image/jpeg', .94));
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
    setState('capturing');
    clearTimeout(timer);
    status('Capturing…', 'Keep still for a moment.');
    try {
      for (let index = 0; index < 3; index++) {
        if (index) await new Promise(resolve => setTimeout(resolve, 130));
        if (token !== generation) return;
        const canvas = snapshot();
        try {
          const detection = detect(canvas);
          if (automatic && (!captureReady(detection) || movement(anchor, detection.quad) > .022)) continue;
          const blob = await encode(canvas);
          if (token !== generation) return;
          candidates.push({ blob, detection, score: detection?.sharpness || 0, url: URL.createObjectURL(blob) });
        } finally { canvas.width = canvas.height = 0; }
      }
      if (token !== generation) return;
      if (!candidates.length) {
        resetStability();
        setState('searching');
        status('The page moved', 'Hold steady while the yellow frame is visible.');
        tick(token);
        return;
      }
      stopCamera();
      candidates.sort((first, second) => second.score - first.score);
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
    const finish = () => { clearTimeout(timeout); URL.revokeObjectURL(url); };
    const timeout = setTimeout(() => { finish(); reject(new Error('Image decode timed out')); }, 15000);
    image.onload = () => { finish(); resolve(image); };
    image.onerror = () => { finish(); reject(new Error('This image could not be opened')); };
    image.src = url;
  });
  async function reviewCandidate(index, token) {
    const selected = ++selection;
    const candidate = candidates[index];
    if (!candidate) return;
    setState('processing');
    $('liveCameraView').hidden = true;
    $('liveReview').hidden = false;
    $('liveAddBtn').disabled = true;
    $('liveReviewStatus').textContent = 'Straightening the page…';
    [...$('liveAlternatives').children].forEach((button, i) => button.setAttribute('aria-pressed', String(i === index)));
    let source, output;
    const current = () => token === generation && selected === selection;
    try {
      const image = await decode(candidate.blob);
      if (!current()) return;
      source = document.createElement('canvas');
      const scale = Math.min(1, 2560 / Math.max(image.naturalWidth, image.naturalHeight));
      source.width = Math.round(image.naturalWidth * scale);
      source.height = Math.round(image.naturalHeight * scale);
      source.getContext('2d').drawImage(image, 0, 0, source.width, source.height);
      // Detailed correction runs once after capture, never in the live loop.
      await new Promise(resolve => setTimeout(resolve, 0));
      if (!current()) return;
      const detection = SP.Lightweight.detectPage(source);
      const precise = ['stencil', 'marker-guided'].includes(detection.method) && detection.confidence >= .55;
      const liveQuad = candidate.detection?.pageQuad;
      const quad = precise ? detection.pageQuad : liveQuad?.map(point => ({ x: point.x * source.width, y: point.y * source.height }));
      output = quad ? await SP.Lightweight.warp(source, quad, precise ? detection.frame : null) : source;
      if (!current()) return;
      // Preserve handwriting and original colour when no calibration was found.
      if (precise) await SP.Lightweight.correctColors(output, { useStencil: true, preciseStencil: true });
      const blob = await encode(output);
      if (!current()) return;
      result = { blob, width: output.width, height: output.height, name: 'Scanned page' };
      if (previewUrl) URL.revokeObjectURL(previewUrl);
      previewUrl = URL.createObjectURL(blob);
      const preview = $('livePreview');
      preview.src = previewUrl;
      preview.classList.remove('scan-reveal');
      void preview.offsetWidth;
      preview.classList.add('scan-reveal');
      $('liveAddBtn').disabled = false;
      $('liveAddBtn').textContent = embedded ? 'Add page to document' : 'Save scan';
      $('liveReviewStatus').textContent = quad
        ? 'Check the scan. You can choose another frame or scan again.'
        : 'Page edges were not clear. Check the full photo, or scan again.';
      setState('review');
    } catch (error) {
      if (current()) {
        result = null;
        $('liveReviewStatus').textContent = error.message || 'Could not process this frame. Choose another or scan again.';
        setState('review');
      }
    } finally {
      if (output && output !== source) output.width = output.height = 0;
      if (source) source.width = source.height = 0;
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
      link.href = previewUrl;
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
  new ResizeObserver(() => drawDetection(lastDetection)).observe(video);
  SP.LiveCamera = { open, close };
  if (embedded) void open();
})();
