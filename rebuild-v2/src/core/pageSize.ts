export interface PageSizeInfo {
  standardName: string;
  dimensionsLabel: string;
  ratioLabel: string;
}

const PX_PER_INCH = 96;
const MM_PER_INCH = 25.4;

function pxToMillimetres(px: number): number {
  return Math.max(1, Number.isFinite(px) ? px : 1) * MM_PER_INCH / PX_PER_INCH;
}

function formatMillimetres(px: number): string {
  const millimetres = pxToMillimetres(px);
  const rounded = Math.round(millimetres);
  return Math.abs(millimetres - rounded) < 0.25
    ? String(rounded)
    : millimetres.toFixed(1).replace(/\.0$/, '');
}

function standardPageName(widthMm: number, heightMm: number): string {
  const matches = (first: number, second: number): boolean =>
    Math.abs(widthMm - first) <= 1.5 && Math.abs(heightMm - second) <= 1.5;
  if (matches(210, 297) || matches(297, 210)) return 'A4';
  if (matches(148, 210) || matches(210, 148)) return 'A5';
  if (matches(215.9, 279.4) || matches(279.4, 215.9)) return 'US Letter';
  return '';
}

export function formatPageSize(width: number, height: number): PageSizeInfo {
  const safeWidth = Math.max(1, Number.isFinite(width) ? width : 1);
  const safeHeight = Math.max(1, Number.isFinite(height) ? height : 1);
  const landscape = safeWidth >= safeHeight;
  const longSideRatio = (landscape ? safeWidth / safeHeight : safeHeight / safeWidth)
    .toFixed(3)
    .replace(/\.?0+$/, '');

  return {
    standardName: standardPageName(pxToMillimetres(safeWidth), pxToMillimetres(safeHeight)),
    dimensionsLabel: `${formatMillimetres(safeWidth)} × ${formatMillimetres(safeHeight)} mm`,
    ratioLabel: landscape ? `${longSideRatio} : 1` : `1 : ${longSideRatio}`
  };
}

export function pageSizeLabel(width: number, height: number): string {
  const size = formatPageSize(width, height);
  return [size.standardName, size.dimensionsLabel, size.ratioLabel].filter(Boolean).join(' · ');
}
