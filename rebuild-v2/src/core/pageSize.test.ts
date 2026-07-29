import { describe, expect, it } from 'vitest';
import { A4_HEIGHT, A4_WIDTH } from './model';
import { formatPageSize, pageSizeLabel } from './pageSize';

describe('page sizes', () => {
  it('formats the default A4 page in millimetres and aspect ratio', () => {
    expect(formatPageSize(A4_WIDTH, A4_HEIGHT)).toEqual({
      standardName: 'A4',
      dimensionsLabel: '210 × 297 mm',
      ratioLabel: '1 : 1.414'
    });
  });

  it('handles landscape pages without changing their orientation', () => {
    expect(pageSizeLabel(A4_HEIGHT, A4_WIDTH)).toBe('A4 · 297 × 210 mm · 1.414 : 1');
  });

  it('keeps useful precision for non-standard imported pages', () => {
    expect(formatPageSize(1000, 1000)).toEqual({
      standardName: '',
      dimensionsLabel: '264.6 × 264.6 mm',
      ratioLabel: '1 : 1'
    });
  });
});
