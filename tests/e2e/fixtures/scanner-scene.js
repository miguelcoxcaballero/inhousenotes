(() => {
  'use strict';

  // Independent 3D paper comes from the pinhole/arc-length fixture, not from
  // scanner frame paths or its surface interpolation. The same S-shaped sheet
  // is photographed on a plain table and on a larger pale book, so adding
  // surroundings cannot legitimately move its writing or include the book.
  const CASES = [
    { name: 's-bend-on-plain-table', clutter: false },
    { name: 's-bend-on-pale-book', clutter: true },
    { name: 'warm-s-bend-on-pale-book', clutter: true, warm: true },
    { name: 'faint-soft-s-bend-on-pale-book', clutter: true, warm: true, railOpacity: .35, blur: .8 }
  ];

  function render(definition) {
    const rendered = ScannerPhysicalFixtures.render({
      name: definition.name, sBend: .58, pitch: .18, yaw: -.24
    });
    const { source } = rendered;
    const context = source.getContext('2d', { willReadFrequently: true });
    const pixels = context.getImageData(0, 0, source.width, source.height);
    const scene = document.createElement('canvas');
    scene.width = source.width; scene.height = source.height;
    const background = scene.getContext('2d');
    background.fillStyle = '#a47b51';
    background.fillRect(0, 0, scene.width, scene.height);
    if (definition.clutter) {
      // Pale covers touching the paper confuse silhouette segmentation. The
      // invented mark is deliberately multicoloured without reproducing any
      // private photograph, real brand or exact calibration-marker sequence.
      background.save();
      background.translate(405, 584);
      background.rotate(-.035);
      background.fillStyle = '#d7d9dc';
      background.fillRect(-347, -475, 720, 980);
      background.fillStyle = '#43494e';
      background.fillRect(295, -475, 78, 980);
      background.restore();
      ['#de5545', '#438ad1', '#48a268', '#e9cf46'].forEach((color, index) => {
        background.strokeStyle = color;
        background.lineWidth = 12;
        background.beginPath();
        background.arc(112 + index * 19, 318, 20, -.6, Math.PI * 1.5);
        background.stroke();
      });
      background.strokeStyle = '#262e34';
      background.lineWidth = 5;
      background.beginPath(); background.arc(127, 474, 49, 0, Math.PI * 2); background.stroke();
      background.font = '19px sans-serif';
      background.fillStyle = '#9a4e52';
      background.fillText('BOOK', 92, 480);
      background.save();
      background.translate(770, 114); background.rotate(.32);
      background.fillStyle = '#fff1be'; background.fillRect(-100, -90, 170, 170);
      for (let y = -80; y < 75; y += 24) {
        background.fillStyle = y < 0 ? '#d0b94f' : '#317061';
        background.fillRect(-100, y, 170, 11);
      }
      background.restore();
      background.fillStyle = '#e5c847'; background.fillRect(24, 1040, 110, 22);
    }
    const scenePixels = background.getImageData(0, 0, scene.width, scene.height);
    const mask = document.createElement('canvas');
    mask.width = source.width; mask.height = source.height;
    const maskContext = mask.getContext('2d');
    const maskPixels = maskContext.createImageData(mask.width, mask.height);
    for (let offset = 0; offset < pixels.data.length; offset += 4) {
      const outside = pixels.data[offset] === 136
        && pixels.data[offset + 1] === 98 && pixels.data[offset + 2] === 68;
      for (let channel = 0; channel < 3; channel += 1) {
        if (outside) pixels.data[offset + channel] = scenePixels.data[offset + channel];
        maskPixels.data[offset + channel] = outside ? 0 : 255;
      }
      maskPixels.data[offset + 3] = 255;
      if (!outside && definition.railOpacity != null
        && pixels.data[offset] > 150 && pixels.data[offset + 1] > 130
        && pixels.data[offset] > pixels.data[offset + 2] + 35
        && pixels.data[offset + 1] > pixels.data[offset + 2] + 20) {
        for (let channel = 0; channel < 3; channel += 1) {
          pixels.data[offset + channel] = 255
            + (pixels.data[offset + channel] - 255) * definition.railOpacity;
        }
      }
      if (definition.warm) {
        const x = (offset / 4) % source.width;
        const shade = .88 + .09 * x / source.width;
        pixels.data[offset] *= shade;
        pixels.data[offset + 1] *= shade * .955;
        pixels.data[offset + 2] *= shade * .925;
      }
    }
    context.putImageData(pixels, 0, 0);
    if (definition.blur) {
      const photo = document.createElement('canvas');
      photo.width = source.width; photo.height = source.height;
      const photoContext = photo.getContext('2d');
      photoContext.filter = `blur(${definition.blur}px)`;
      photoContext.drawImage(source, 0, 0);
      context.drawImage(photo, 0, 0);
    }
    maskContext.putImageData(maskPixels, 0, 0);
    return { ...rendered, mask };
  }

  function foreignInteriorRatio(canvas) {
    const { width, height } = canvas;
    const pixels = canvas.getContext('2d').getImageData(0, 0, width, height).data;
    let foreign = 0, samples = 0;
    // Test inside the printed frame, where the warp must sample actual paper.
    // Blank margins synthesized by production cannot hide an incorrect crop.
    for (let y = Math.ceil(height * .065); y < height * .94; y += 1) {
      for (let x = Math.ceil(width * .085); x < width * .915; x += 1) {
        samples += 1;
        if (pixels[(y * width + x) * 4] < 128) foreign += 1;
      }
    }
    return foreign / samples;
  }

  window.ScannerSceneFixtures = { CASES, render, foreignInteriorRatio };
})();
