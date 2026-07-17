import type { DocStore } from '../core/store';
import type { PersistController } from '../persist/persistController';
import type { DocRenderer } from '../render/docRenderer';
import type { JsonPage } from '../core/serial';
import { makeOverlay, buttonStyle, confirmAction, smallText } from './modal';

export class TimelinePanel {
  constructor(
    private host: HTMLElement,
    private store: DocStore,
    private persist: PersistController,
    private renderer: DocRenderer
  ) {}

  async open(): Promise<void> {
    await this.persist.compact();
    const modal = makeOverlay('Timeline');
    this.host.appendChild(modal.overlay);
    this.render(modal.body, modal.close);
  }

  private render(body: HTMLElement, close: () => void): void {
    body.innerHTML = '';
    const entries = [...this.persist.versions.entries].sort((a, b) => b.ts - a.ts);
    const originalId = this.persist.versions.entries[0]?.id ?? null;

    if (entries.length === 0) {
      body.appendChild(smallText('No saved versions yet.'));
      return;
    }

    const list = document.createElement('div');
    Object.assign(list.style, {
      display: 'grid',
      gap: '10px'
    });
    body.appendChild(list);

    let lastGroup = '';
    for (const entry of entries) {
      const group = formatMonth(entry.ts);
      if (group !== lastGroup) {
        lastGroup = group;
        const header = document.createElement('div');
        header.textContent = group;
        Object.assign(header.style, {
          marginTop: list.childElementCount ? '8px' : '0',
          fontSize: '12px',
          fontWeight: '900',
          color: '#475569',
          textTransform: 'uppercase',
          letterSpacing: '0.04em'
        });
        list.appendChild(header);
      }

      const row = document.createElement('div');
      Object.assign(row.style, {
        display: 'grid',
        gridTemplateColumns: '112px 1fr auto',
        gap: '12px',
        alignItems: 'center',
        padding: '10px',
        border: '1px solid #e2e8f0',
        borderRadius: '8px',
        background: '#fff'
      });

      const preview = renderPagePreview(entry.snapshot.pages[0] ?? null, 96, 132);
      preview.title = 'Preview';
      preview.style.cursor = 'zoom-in';
      preview.addEventListener('click', () => this.openPreview(entry.snapshot.pages[0] ?? null));
      row.appendChild(preview);

      const text = document.createElement('div');
      const title = document.createElement('div');
      title.textContent = entry.id === originalId ? 'Original' : entry.summary;
      Object.assign(title.style, { fontSize: '14px', fontWeight: '700', color: '#0f172a', marginBottom: '4px' });
      text.appendChild(title);
      const meta = document.createElement('div');
      meta.textContent = `${formatDateTime(entry.ts)} · ${entry.author.name || 'You'} · ${entry.snapshot.pages.length} page${entry.snapshot.pages.length === 1 ? '' : 's'}`;
      Object.assign(meta.style, { fontSize: '12px', color: '#64748b' });
      text.appendChild(meta);
      if (entry.isMilestone) {
        const mark = document.createElement('div');
        mark.textContent = entry.kind;
        Object.assign(mark.style, { marginTop: '6px', fontSize: '12px', color: '#1d4ed8', fontWeight: '700' });
        text.appendChild(mark);
      }
      if (entry.id === originalId) {
        const mark = document.createElement('div');
        mark.textContent = 'first saved version';
        Object.assign(mark.style, { marginTop: '6px', fontSize: '12px', color: '#0f766e', fontWeight: '700' });
        text.appendChild(mark);
      }
      row.appendChild(text);

      const restore = document.createElement('button');
      restore.type = 'button';
      restore.textContent = 'Restore';
      Object.assign(restore.style, buttonStyle('primary'));
      restore.addEventListener('click', async () => {
        const ok = await confirmAction(
          'Restore version',
          `Restore "${entry.summary}" from ${formatDateTime(entry.ts)}?`,
          'Restore'
        );
        if (!ok) return;
        this.persist.pendingVersionHint = { milestone: true, kind: 'restore', summary: `Restored ${formatDateTime(entry.ts)}` };
        this.store.apply(this.persist.versions.restoreOp(this.store.doc, entry), 'restore');
        this.renderer.rebuild();
        await this.persist.compact();
        close();
      });
      row.appendChild(restore);
      list.appendChild(row);
    }
  }

  private openPreview(page: JsonPage | null): void {
    const modal = makeOverlay('Version Preview');
    this.host.appendChild(modal.overlay);
    modal.body.style.display = 'grid';
    modal.body.style.placeItems = 'center';
    modal.body.appendChild(renderPagePreview(page, 360, 520));
  }
}

export function renderPagePreview(page: JsonPage | null, width: number, height: number): HTMLCanvasElement {
  const canvas = document.createElement('canvas');
  canvas.width = width;
  canvas.height = height;
  Object.assign(canvas.style, {
    width: `${width}px`,
    height: `${height}px`,
    borderRadius: '6px',
    border: '1px solid #cbd5e1',
    background: '#fff',
    boxShadow: '0 1px 3px rgba(15,23,42,0.10)'
  });
  const ctx = canvas.getContext('2d')!;
  ctx.fillStyle = '#fff';
  ctx.fillRect(0, 0, width, height);
  if (!page) return canvas;

  const scale = Math.min((width - 12) / page.width, (height - 12) / page.height);
  const ox = (width - page.width * scale) / 2;
  const oy = (height - page.height * scale) / 2;
  ctx.save();
  ctx.translate(ox, oy);
  ctx.scale(scale, scale);
  ctx.fillStyle = page.background.kind === 'template' && page.background.template === 'agenda' ? '#fffde7' : '#fff';
  ctx.fillRect(0, 0, page.width, page.height);
  if (page.background.kind === 'custom') {
    const img = new Image();
    img.onload = () => {
      ctx.save();
      ctx.translate(ox, oy);
      ctx.scale(scale, scale);
      ctx.drawImage(img, 0, 0, page.width, page.height);
      drawPreviewInk(ctx, page);
      ctx.restore();
    };
    img.src = page.background.src;
  }
  ctx.strokeStyle = '#e2e8f0';
  ctx.lineWidth = 1 / scale;
  ctx.strokeRect(0, 0, page.width, page.height);
  drawPreviewInk(ctx, page);
  ctx.restore();
  queuePreviewImages(ctx, page, ox, oy, scale);
  ctx.globalAlpha = 1;
  return canvas;
}

function drawPreviewInk(ctx: CanvasRenderingContext2D, page: JsonPage): void {
  for (const stroke of page.strokes) {
    if (stroke.points.length < 6) continue;
    ctx.globalAlpha = stroke.tool === 'highlighter' ? 0.35 : 1;
    ctx.strokeStyle = stroke.color;
    ctx.lineWidth = stroke.width;
    ctx.lineCap = 'round';
    ctx.lineJoin = 'round';
    ctx.beginPath();
    ctx.moveTo(stroke.points[0]!, stroke.points[1]!);
    for (let i = 3; i < stroke.points.length; i += 3) {
      ctx.lineTo(stroke.points[i]!, stroke.points[i + 1]!);
    }
    ctx.stroke();
  }
  ctx.globalAlpha = 1;
}

function queuePreviewImages(ctx: CanvasRenderingContext2D, page: JsonPage, ox: number, oy: number, scale: number): void {
  for (const image of page.images) {
    const img = new Image();
    img.onload = () => {
      ctx.save();
      ctx.translate(ox, oy);
      ctx.scale(scale, scale);
      ctx.translate(image.x, image.y);
      ctx.rotate(image.rotation);
      ctx.drawImage(img, -image.width / 2, -image.height / 2, image.width, image.height);
      ctx.restore();
      drawPreviewInk(ctx, page);
    };
    img.src = image.src;
  }
}

function formatDateTime(ts: number): string {
  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit'
  }).format(new Date(ts));
}

function formatMonth(ts: number): string {
  return new Intl.DateTimeFormat(undefined, { month: 'long', year: 'numeric' }).format(new Date(ts));
}
