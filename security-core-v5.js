(function attachInhouseSecurityCore(root) {
    'use strict';

    const MAX_PDF_BYTES = 64 * 1024 * 1024;
    const MAX_PDF_PAGES = 2000;
    const MAX_PDF_IMAGE_PIXELS = 40 * 1000 * 1000;
    const TRUSTED_PDF_ORIGINS = new Set([
        'https://www.googleapis.com',
        'https://drive.google.com',
        'https://drive.usercontent.google.com'
    ]);

    function byteLengthOfPdfData(data) {
        if (data instanceof ArrayBuffer) return data.byteLength;
        if (ArrayBuffer.isView(data)) return data.byteLength;
        if (typeof Blob !== 'undefined' && data instanceof Blob) return data.size;
        return null;
    }

    function assertSafePdfBlob(blob) {
        if (typeof Blob === 'undefined' || !(blob instanceof Blob)) {
            throw new TypeError('PDF source is not a Blob');
        }
        if (!Number.isFinite(blob.size) || blob.size <= 0 || blob.size > MAX_PDF_BYTES) {
            throw new RangeError('PDF data exceeds the safe size limit');
        }
        if (blob.type && !/^(application\/pdf|application\/octet-stream)(?:\s*;.*)?$/i.test(blob.type)) {
            throw new TypeError('PDF source has an unexpected content type');
        }
        return blob;
    }

    function isTrustedPdfUrl(value, locationHref = root.location?.href || 'https://inhousenotes.com/') {
        if (typeof value !== 'string' || !value) return false;
        if (value.startsWith('blob:')) return true;
        let parsed;
        try {
            parsed = new URL(value, locationHref);
        } catch (error) {
            return false;
        }
        if (parsed.protocol !== 'https:' && parsed.protocol !== 'http:') return false;
        let ownOrigin = '';
        try {
            ownOrigin = new URL(locationHref).origin;
        } catch (error) {}
        if (parsed.origin === ownOrigin) return true;
        return parsed.protocol === 'https:' && TRUSTED_PDF_ORIGINS.has(parsed.origin);
    }

    function hardenPdfDocumentParams(source, options = {}) {
        let params;
        if (typeof source === 'string') params = { url: source };
        else if (source instanceof ArrayBuffer || ArrayBuffer.isView(source)) params = { data: source };
        else if (source && typeof source === 'object' && !Array.isArray(source)) params = { ...source };
        else throw new TypeError('Unsupported PDF source');

        if (Object.prototype.hasOwnProperty.call(params, 'url')) {
            if (!isTrustedPdfUrl(params.url, options.locationHref)) throw new TypeError('Blocked untrusted PDF URL');
        }
        if (Object.prototype.hasOwnProperty.call(params, 'data')) {
            const byteLength = byteLengthOfPdfData(params.data);
            if (!Number.isFinite(byteLength) || byteLength <= 0 || byteLength > MAX_PDF_BYTES) {
                throw new RangeError('PDF data exceeds the safe size limit');
            }
        }
        if (!Object.prototype.hasOwnProperty.call(params, 'url')
            && !Object.prototype.hasOwnProperty.call(params, 'data')) {
            throw new TypeError('PDF source must contain data or a URL');
        }

        params.isEvalSupported = false;
        params.maxImageSize = Math.min(
            Number.isFinite(Number(params.maxImageSize)) ? Number(params.maxImageSize) : MAX_PDF_IMAGE_PIXELS,
            MAX_PDF_IMAGE_PIXELS
        );
        return params;
    }

    async function loadPdfDocument(pdfjs, source, options = {}) {
        if (!pdfjs || typeof pdfjs.getDocument !== 'function') throw new TypeError('PDF.js is unavailable');
        const params = hardenPdfDocumentParams(source, options);
        const task = pdfjs.getDocument(params);
        const document = await task.promise;
        if (!Number.isInteger(document?.numPages) || document.numPages < 1 || document.numPages > MAX_PDF_PAGES) {
            try {
                await task.destroy();
            } catch (error) {}
            throw new RangeError('PDF page count exceeds the safe limit');
        }
        return document;
    }

    const api = Object.freeze({
        MAX_PDF_BYTES,
        assertSafePdfBlob,
        hardenPdfDocumentParams,
        loadPdfDocument
    });
    root.InhouseSecurityCore = api;
    if (typeof module !== 'undefined' && module.exports) module.exports = api;
})(typeof globalThis !== 'undefined' ? globalThis : window);
