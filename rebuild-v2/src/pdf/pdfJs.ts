let runtimePromise: Promise<unknown> | null = null;

/** Load pdf.js with its worker handler bundled for offline/single-file use. */
export function loadPdfJsRuntime<T>(): Promise<T> {
  if (!runtimePromise) {
    runtimePromise = (async () => {
      const worker = await import('pdfjs-dist/legacy/build/pdf.worker.mjs');
      const root = globalThis as typeof globalThis & {
        pdfjsWorker?: { WorkerMessageHandler: unknown };
      };
      root.pdfjsWorker = { WorkerMessageHandler: worker.WorkerMessageHandler };
      return import('pdfjs-dist/legacy/build/pdf.mjs');
    })();
  }
  return runtimePromise as Promise<T>;
}
