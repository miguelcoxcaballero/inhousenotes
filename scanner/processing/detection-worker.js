/* Each captured still runs its complete pipeline on an independent worker. */
"use strict";
self.window = self;
self.document = {
  createElement(name) {
    if (name !== "canvas") throw new Error("Unsupported scanner worker element");
    return new OffscreenCanvas(1, 1);
  }
};
importScripts(`liveDetection.js${self.location.search}`, `surface-parameterization.js${self.location.search}`,
  `metric-surface.js${self.location.search}`, `scan-template.js${self.location.search}`, `lightweight.js${self.location.search}`);
self.onmessage = async event => {
  const { operation, bitmap, blob, config, liveDetection, maxDimension, timeoutMs,
    previewProgress, reportProgress, surfaceBudgetMs } = event.data;
  try {
    self.ScannerPro.Config = config;
    if (operation === "process") {
      if (typeof OffscreenCanvas.prototype.convertToBlob !== "function"
        || !new OffscreenCanvas(1, 1).getContext("2d")) {
        self.postMessage({ unsupported: true });
        return;
      }
      const result = await self.ScannerPro.Lightweight.processPageAsync(blob, {
        liveDetection, maxDimension, timeoutMs, previewProgress,
        onProgress: (progress, detail) => self.postMessage({ progress, detail })
      });
      self.postMessage({ result });
      return;
    }
    const detection = self.ScannerPro.Lightweight.detectPage(bitmap, { liveDetection, surfaceBudgetMs,
      onProgress: reportProgress ? (progress, detail) => self.postMessage({ progress, detail }) : undefined });
    self.postMessage({ detection });
  } catch (error) {
    self.postMessage({ error: error?.message || "The page could not be analysed.", name: error?.name,
      needsImageDecoder: !!error?.needsImageDecoder });
  } finally {
    bitmap?.close();
  }
};
