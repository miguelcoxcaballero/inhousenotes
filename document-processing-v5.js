(function (root) {
    'use strict';
    const processorUrl = new URL(root.InhouseAssetUrl('document-processing-worker-v5.js'));
    const processorVersion = new URL(document.currentScript.src, document.baseURI).searchParams.get('v');
    if (processorVersion) processorUrl.searchParams.set('v', processorVersion);
    const client = new root.InhouseRuntimeCore.WorkerRpcClient(() => new Worker(processorUrl));
    // Checkpoint compression must not queue behind a PDF export.
    const checkpointClient = new root.InhouseRuntimeCore.WorkerRpcClient(() => new Worker(processorUrl));
    const run = (type, data, options) => (type === 'packInkCheckpoint' || type === 'unpackInkCheckpoint' ? checkpointClient : client).run(type, data, options);
    function reset(error) { client.reset(error); checkpointClient.reset(error); facade = null; }
    let facade = null;
    async function pdfLib(library) {
        await run('pdfReady', {});
        if (facade?.epoch === client.epoch) return facade;
        function documentProxy(info, epoch) {
            const id = info.id;
            let disposed = false;
            const owned = (type, data) => {
                try {
                    client.assertEpoch(epoch);
                    if (disposed) throw new Error('PDF document is disposed');
                    return run(type, data);
                } catch (error) { return Promise.reject(error); }
            };
            function pageProxy(page) {
                return {
                    workerPageId: page.id,
                    getSize: () => page.size,
                    async drawImage(image, options) {
                        image.workerDrawName = await owned('pdfDrawImage', { id, page: page.id, image: image.id, options });
                    },
                    drawRectangle: options => owned('pdfRectangle', { id, page: page.id, options })
                };
            }
            return {
                workerDocumentId: id,
                assertUsable: () => { client.assertEpoch(epoch); if (disposed) throw new Error('PDF document is disposed'); },
                getPageCount: () => info.pageCount,
                async copyPages(source, indices) {
                    source.assertUsable();
                    return (await owned('pdfCopyPages', { id, source: source.workerDocumentId, indices })).map(pageProxy);
                },
                async addPage(page) {
                    const added = await owned('pdfAddPage', {
                        id, page: page?.workerPageId, size: Array.isArray(page) ? page : undefined
                    });
                    info.pageCount++;
                    return pageProxy(added);
                },
                embedPng: bytes => owned('pdfEmbed', { id, bytes, kind: 'embedPng' }),
                embedJpg: bytes => owned('pdfEmbed', { id, bytes, kind: 'embedJpg' }),
                setKeywords: value => owned('pdfMetadata', { id, method: 'setKeywords', value }),
                setTitle: value => owned('pdfMetadata', { id, method: 'setTitle', value }),
                setSubject: value => owned('pdfMetadata', { id, method: 'setSubject', value }),
                setCreator: value => owned('pdfMetadata', { id, method: 'setCreator', value }),
                attach: (bytes, name, options) => owned('pdfAttach', { id, bytes, name, options }),
                save: options => owned('pdfSave', { id, options }),
                dispose: async () => {
                    if (disposed || epoch !== client.epoch) return;
                    const disposal = owned('pdfDispose', { id });
                    disposed = true;
                    await disposal;
                }
            };
        }
        async function createDocument(type, data) {
            const epoch = client.epoch;
            const info = await run(type, data);
            client.assertEpoch(epoch);
            return documentProxy(info, epoch);
        }
        facade = {
            epoch: client.epoch,
            rgb: library.rgb, PDFName: library.PDFName,
            PDFDocument: {
                load: async (bytes, options) => createDocument('pdfLoad', { bytes, options }),
                create: async () => createDocument('pdfCreate', {})
            }
        };
        return facade;
    }
    root.InhouseDocumentProcessing = Object.freeze({ run, reset, pdfLib });
})(globalThis);
