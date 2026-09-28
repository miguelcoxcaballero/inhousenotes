(function (root) {
    'use strict';
    let worker = null;
    let sequence = 0;
    const processorUrl = new URL('document-processing-worker-v5.js', document.baseURI);
    processorUrl.search = new URL(document.currentScript.src, document.baseURI).search;
    const pending = new Map();
    function reset(error = new Error('Document processor stopped')) {
        worker?.terminate();
        worker = null;
        facade = null;
        for (const request of pending.values()) {
            clearTimeout(request.timer);
            request.reject(error);
        }
        pending.clear();
    }
    function run(type, data) {
        return new Promise((resolve, reject) => {
            if (!worker) {
                worker = new Worker(processorUrl);
                worker.onmessage = ({ data: response }) => {
                    const request = pending.get(response.id);
                    if (!request) return;
                    pending.delete(response.id);
                    clearTimeout(request.timer);
                    if (response.error) request.reject(new Error(response.error));
                    else request.resolve(response.value);
                };
                worker.onerror = event => reset(new Error(event.message || 'Document worker failed'));
                worker.onmessageerror = () => reset(new Error('Document worker response unreadable'));
            }
            const id = ++sequence;
            const timer = setTimeout(() => reset(new Error('Document processing timed out')), 90000);
            pending.set(id, { resolve, reject, timer });
            try { worker.postMessage({ id, type, data }); }
            catch (error) { pending.delete(id); clearTimeout(timer); reject(error); }
        });
    }
    let facade = null;
    async function pdfLib(library) {
        await run('pdfReady', {});
        if (facade) return facade;
        function documentProxy(info) {
            const id = info.id;
            function pageProxy(page) {
                return {
                    workerPageId: page.id,
                    getSize: () => page.size,
                    async drawImage(image, options) {
                        image.workerDrawName = await run('pdfDrawImage', { id, page: page.id, image: image.id, options });
                    },
                    drawRectangle: options => run('pdfRectangle', { id, page: page.id, options })
                };
            }
            return {
                workerDocumentId: id,
                getPageCount: () => info.pageCount,
                async copyPages(source, indices) {
                    return (await run('pdfCopyPages', { id, source: source.workerDocumentId, indices })).map(pageProxy);
                },
                async addPage(page) {
                    return pageProxy(await run('pdfAddPage', {
                        id, page: page?.workerPageId, size: Array.isArray(page) ? page : undefined
                    }));
                },
                embedPng: bytes => run('pdfEmbed', { id, bytes, kind: 'embedPng' }),
                embedJpg: bytes => run('pdfEmbed', { id, bytes, kind: 'embedJpg' }),
                setKeywords: value => run('pdfMetadata', { id, method: 'setKeywords', value }),
                setTitle: value => run('pdfMetadata', { id, method: 'setTitle', value }),
                setSubject: value => run('pdfMetadata', { id, method: 'setSubject', value }),
                setCreator: value => run('pdfMetadata', { id, method: 'setCreator', value }),
                attach: (bytes, name, options) => run('pdfAttach', { id, bytes, name, options }),
                save: options => run('pdfSave', { id, options }),
                dispose: () => run('pdfDispose', { id }).catch(() => {})
            };
        }
        facade = {
            rgb: library.rgb, PDFName: library.PDFName,
            PDFDocument: {
                load: async (bytes, options) => documentProxy(await run('pdfLoad', { bytes, options })),
                create: async () => documentProxy(await run('pdfCreate', {}))
            }
        };
        return facade;
    }
    root.InhouseDocumentProcessing = Object.freeze({ run, reset, pdfLib });
})(globalThis);
