(function attachInhouseSecurityCore(root) {
    'use strict';

    const MAX_CALENDAR_HTML_CHARS = 512 * 1024;
    const MAX_PDF_BYTES = 64 * 1024 * 1024;
    const MAX_PDF_PAGES = 2000;
    const MAX_PDF_IMAGE_PIXELS = 40 * 1000 * 1000;
    const CALENDAR_ALLOWED_TAGS = new Set(['DIV', 'SPAN', 'BUTTON']);
    const CALENDAR_ALLOWED_CLASSES = new Set([
        'sp-day-section', 'today', 'sp-day-header', 'sp-day-label',
        'sp-add-btn', 'sp-events-list', 'sp-no-events', 'sp-event',
        'sp-event-time', 'sp-event-title'
    ]);
    const CALENDAR_ALLOWED_ATTRIBUTES = new Set([
        'class', 'data-day-key', 'data-sp-add', 'data-sp-event',
        'data-sp-day', 'style', 'type', 'title', 'aria-label'
    ]);
    const TRUSTED_PDF_ORIGINS = new Set([
        'https://www.googleapis.com',
        'https://drive.google.com',
        'https://drive.usercontent.google.com'
    ]);

    function sanitizeCalendarPanelHtml(html, documentRef = root.document) {
        if (typeof html !== 'string' || !html || !documentRef?.createElement) return '';
        if (html.length > MAX_CALENDAR_HTML_CHARS) return '';
        const template = documentRef.createElement('template');
        template.innerHTML = html;
        const content = template.content;

        Array.from(content.querySelectorAll('*')).forEach(element => {
            if (!CALENDAR_ALLOWED_TAGS.has(element.tagName)) {
                element.remove();
                return;
            }
            Array.from(element.attributes).forEach(attribute => {
                if (!CALENDAR_ALLOWED_ATTRIBUTES.has(attribute.name)) {
                    element.removeAttribute(attribute.name);
                }
            });
            const safeClasses = Array.from(element.classList)
                .filter(className => CALENDAR_ALLOWED_CLASSES.has(className));
            element.className = safeClasses.join(' ');
            if (element.hasAttribute('style')) {
                const colorMatch = element.getAttribute('style')
                    .match(/(?:^|;)\s*--sp-event-color\s*:\s*(#[0-9a-f]{6})\s*(?:;|$)/i);
                element.removeAttribute('style');
                if (colorMatch) element.style.setProperty('--sp-event-color', colorMatch[1]);
            }
            ['data-day-key', 'data-sp-add', 'data-sp-event', 'data-sp-day', 'title', 'aria-label']
                .forEach(attributeName => {
                    if (!element.hasAttribute(attributeName)) return;
                    const value = element.getAttribute(attributeName) || '';
                    element.setAttribute(attributeName, value.slice(0, 512));
                });
            if (element.tagName === 'BUTTON') element.setAttribute('type', 'button');
            else element.removeAttribute('type');
        });

        if (documentRef.createTreeWalker && root.NodeFilter) {
            const walker = documentRef.createTreeWalker(content, root.NodeFilter.SHOW_COMMENT);
            const comments = [];
            while (walker.nextNode()) comments.push(walker.currentNode);
            comments.forEach(comment => comment.remove());
        }

        content.querySelectorAll('.sp-day-section').forEach(section => {
            const list = section.querySelector('.sp-events-list') || section;
            const seen = new Set();
            list.querySelectorAll('.sp-event[data-sp-event]').forEach(button => {
                const id = button.getAttribute('data-sp-event') || '';
                if (!id) return;
                if (seen.has(id)) button.remove();
                else seen.add(id);
            });
        });
        const sanitized = template.innerHTML;
        return sanitized.length <= MAX_CALENDAR_HTML_CHARS ? sanitized : '';
    }

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
        MAX_CALENDAR_HTML_CHARS,
        MAX_PDF_BYTES,
        MAX_PDF_PAGES,
        sanitizeCalendarPanelHtml,
        assertSafePdfBlob,
        isTrustedPdfUrl,
        hardenPdfDocumentParams,
        loadPdfDocument
    });
    root.InhouseSecurityCore = api;
    if (typeof module !== 'undefined' && module.exports) module.exports = api;
})(typeof globalThis !== 'undefined' ? globalThis : window);
