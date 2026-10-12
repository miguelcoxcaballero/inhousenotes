/* Printable scanner SVGs retain their original format. Canvas previews use
 * the same native renderer as editor pages; scan ink remains a separate layer.
 */
(function (root) {
  "use strict";
  const SP = (root.ScannerPro = root.ScannerPro || {});
  const STENCIL_BG_SVG = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 21 29.7" width="21cm" height="29.7cm">
      <defs>
        <filter id="softBlur" x="-50%" y="-50%" width="200%" height="200%">
          <feGaussianBlur in="SourceGraphic" stdDeviation="0.02"/>
        </filter>
        <pattern id="dotGrid" x="1.75" y="1.68" width="0.5" height="0.5" patternUnits="userSpaceOnUse">
          <circle cx="0.25" cy="0.25" r="0.035" fill="#a8a8a8" filter="url(#softBlur)"/>
        </pattern>
      </defs>
      <rect x="0" y="0" width="21" height="29.7" fill="white"/>
      <rect x="1.5" y="1.43" width="18" height="27" rx="0.03" ry="0.03"
            fill="none" stroke="#f0db4c" stroke-width="0.06"/>
      <rect x="1.75" y="1.68" width="17.5" height="25.5" fill="url(#dotGrid)"/>
      <g stroke="#f0db4c" stroke-width="0.06" fill="none">
        <rect x="2.0" y="27.43" width="8.0" height="0.5" rx="0.03" ry="0.03"/>
        <path d="M 2.5 27.43 v 0.5 M 3.0 27.43 v 0.5 M 3.5 27.43 v 0.5
                 M 4.0 27.43 v 0.5 M 4.5 27.43 v 0.5 M 5.0 27.43 v 0.5
                 M 5.5 27.43 v 0.5 M 6.0 27.43 v 0.5 M 6.5 27.43 v 0.5
                 M 7.0 27.43 v 0.5 M 7.5 27.43 v 0.5 M 8.0 27.43 v 0.5
                 M 8.5 27.43 v 0.5 M 9.0 27.43 v 0.5 M 9.5 27.43 v 0.5"/>
        <rect x="11.0" y="27.43" width="8.0" height="0.5" rx="0.03" ry="0.03"/>
        <path d="M 11.5 27.43 v 0.5 M 12.0 27.43 v 0.5 M 12.5 27.43 v 0.5
                 M 13.0 27.43 v 0.5 M 13.5 27.43 v 0.5 M 14.0 27.43 v 0.5
                 M 14.5 27.43 v 0.5 M 15.0 27.43 v 0.5 M 15.5 27.43 v 0.5
                 M 16.0 27.43 v 0.5 M 16.5 27.43 v 0.5 M 17.0 27.43 v 0.5
                 M 17.5 27.43 v 0.5 M 18.0 27.43 v 0.5 M 18.5 27.43 v 0.5"/>
      </g>
      <g stroke="#f0db4c" stroke-width="0.06">
        <circle cx="10.125" cy="27.68" r="0.125" fill="#ff0000"/>
        <circle cx="10.375" cy="27.68" r="0.125" fill="#000000"/>
        <circle cx="10.625" cy="27.68" r="0.125" fill="#0000ff"/>
        <circle cx="10.875" cy="27.68" r="0.125" fill="#6eff12"/>
      </g>
    </svg>`;

  const STENCIL_OVERLAY_SVG = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 21 29.7" width="21cm" height="29.7cm">
  <rect x="1.5" y="1.43" width="18" height="27" rx="0.03" ry="0.03"
        fill="none" stroke="#f0db4c" stroke-width="0.06"/>
  <g stroke="#f0db4c" stroke-width="0.06" fill="none">
    <rect x="2.0" y="27.43" width="8.0" height="0.5" rx="0.03" ry="0.03"/>
    <path d="M 2.5 27.43 v 0.5 M 3.0 27.43 v 0.5 M 3.5 27.43 v 0.5
             M 4.0 27.43 v 0.5 M 4.5 27.43 v 0.5 M 5.0 27.43 v 0.5
             M 5.5 27.43 v 0.5 M 6.0 27.43 v 0.5 M 6.5 27.43 v 0.5
             M 7.0 27.43 v 0.5 M 7.5 27.43 v 0.5 M 8.0 27.43 v 0.5
             M 8.5 27.43 v 0.5 M 9.0 27.43 v 0.5 M 9.5 27.43 v 0.5"/>
    <rect x="11.0" y="27.43" width="8.0" height="0.5" rx="0.03" ry="0.03"/>
    <path d="M 11.5 27.43 v 0.5 M 12.0 27.43 v 0.5 M 12.5 27.43 v 0.5
             M 13.0 27.43 v 0.5 M 13.5 27.43 v 0.5 M 14.0 27.43 v 0.5
             M 14.5 27.43 v 0.5 M 15.0 27.43 v 0.5 M 15.5 27.43 v 0.5
             M 16.0 27.43 v 0.5 M 16.5 27.43 v 0.5 M 17.0 27.43 v 0.5
             M 17.5 27.43 v 0.5 M 18.0 27.43 v 0.5 M 18.5 27.43 v 0.5"/>
  </g>
  <g stroke="#f0db4c" stroke-width="0.06">
    <circle cx="10.125" cy="27.68" r="0.125" fill="#ff0000"/>
    <circle cx="10.375" cy="27.68" r="0.125" fill="#000000"/>
    <circle cx="10.625" cy="27.68" r="0.125" fill="#0000ff"/>
    <circle cx="10.875" cy="27.68" r="0.125" fill="#6eff12"/>
  </g>
</svg>`;

  const geometry = Object.freeze({
    widthMM: 210, heightMM: 297,
    frame: Object.freeze({ x: 15, y: 14.3, width: 180, height: 270, radius: .3, stroke: .6, color: "#f0db4c" }),
    bullets: Object.freeze({ x: 20, y: 19.3, pitch: 5, columns: 35, rows: 51, radius: .35, blur: .2, color: "#a8a8a8" }),
    boxes: Object.freeze({ starts: Object.freeze([20, 110]), y: 274.3, width: 80, height: 5, pitch: 5 }),
    markers: Object.freeze({ centers: Object.freeze([101.25, 103.75, 106.25, 108.75]), y: 276.8, radius: 1.25,
      colors: Object.freeze(["#ff0000", "#000000", "#0000ff", "#6eff12"]) })
  });

  function svg(options = {}) {
    if (options.dots === false) return STENCIL_OVERLAY_SVG;
    return options.background === true ? STENCIL_BG_SVG
      : STENCIL_BG_SVG.replace('<rect x="0" y="0" width="21" height="29.7" fill="white"/>', "");
  }

  function render(context, width, height, options = {}) {
    if (!root.InhouseBulletTemplate) throw new Error("The native page template renderer is unavailable.");
    return root.InhouseBulletTemplate.render(context, width, height, {
      ...options, background: options.background === true
    });
  }

  SP.ScanTemplate = Object.freeze({ geometry, svg, render });
})(typeof window === "undefined" ? globalThis : window);
