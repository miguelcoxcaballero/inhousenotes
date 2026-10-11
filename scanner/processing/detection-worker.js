/* Still-image geometry runs off the UI thread; video tracking has its own small detector. */
"use strict";
self.window = self;
self.document = {
  createElement(name) {
    if (name !== "canvas") throw new Error("Unsupported scanner worker element");
    return new OffscreenCanvas(1, 1);
  }
};
importScripts(`surface-parameterization.js${self.location.search}`, `lightweight.js${self.location.search}`);
self.onmessage = event => {
  const { bitmap, config } = event.data;
  try {
    self.ScannerPro.Config = config;
    const detection = self.ScannerPro.Lightweight.detectPage(bitmap);
    self.postMessage({ detection });
  } catch (error) {
    self.postMessage({ error: error?.message || "The page could not be analysed." });
  } finally {
    bitmap.close();
  }
};
