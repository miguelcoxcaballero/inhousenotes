import { A4_HEIGHT, A4_WIDTH, createPage } from '../core/model';
import type { Background } from '../core/model';
import { pageSizeLabel } from '../core/pageSize';
import type { DocStore } from '../core/store';
import type { DocRenderer } from '../render/docRenderer';
import { serializePage } from '../core/serial';
import { makeOverlay, buttonStyle, confirmAction, smallText } from './modal';

export class ManagePagesPanel {
  constructor(
    private host: HTMLElement,
    private store: DocStore,
    private renderer: DocRenderer
  ) {}

  open(): void {
    const modal = makeOverlay('Pages');
    this.host.appendChild(modal.overlay);
    this.render(modal.body);
  }

  private render(body: HTMLElement): void {
    body.innerHTML = '';
    const top = document.createElement('div');
    Object.assign(top.style, { display: 'flex', gap: '8px', alignItems: 'center', marginBottom: '12px' });
    const add = document.createElement('button');
    add.type = 'button';
    add.textContent = 'Add Page';
    Object.assign(add.style, buttonStyle('primary'));
    add.addEventListener('click', () => {
      const page = createPage();
      this.store.apply({ type: 'add-page', page: serializePage(page), index: this.store.doc.pageOrder.length });
      this.renderer.rebuild();
      this.render(body);
    });
    top.appendChild(add);
    const clear = document.createElement('button');
    clear.type = 'button';
    clear.textContent = 'Clear Annotations';
    Object.assign(clear.style, buttonStyle('danger'));
    clear.addEventListener('click', async () => {
      const ok = await confirmAction(
        'Clear annotations',
        'Clear all strokes and inserted images from this document?',
        'Clear'
      );
      if (!ok) return;
      const pages = this.store.doc.pageOrder
        .map((id) => this.store.doc.pages.get(id))
        .filter((page): page is NonNullable<typeof page> => !!page)
        .map((page) => {
          const serial = serializePage(page);
          serial.tombstones = [
            ...new Set([
              ...serial.tombstones,
              ...serial.strokes.map((stroke) => stroke.id),
              ...serial.images.map((image) => image.id)
            ])
          ];
          serial.strokes = [];
          serial.images = [];
          return serial;
        });
      this.store.apply({ type: 'replace-doc', pages });
      this.renderer.rebuild();
      this.render(body);
    });
    top.appendChild(clear);
    top.appendChild(smallText(`${this.store.doc.pageOrder.length} page${this.store.doc.pageOrder.length === 1 ? '' : 's'}`));
    body.appendChild(top);

    const defaultSize = smallText(`Default page · ${pageSizeLabel(A4_WIDTH, A4_HEIGHT)}`);
    Object.assign(defaultSize.style, {
      display: 'block',
      marginBottom: '12px',
      padding: '9px 0',
      borderTop: '1px solid rgba(0,0,0,0.07)',
      borderBottom: '1px solid rgba(0,0,0,0.07)',
      fontVariantNumeric: 'tabular-nums'
    });
    body.appendChild(defaultSize);

    const list = document.createElement('div');
    Object.assign(list.style, { display: 'grid', gap: '8px' });
    body.appendChild(list);

    this.store.doc.pageOrder.forEach((pageId, index) => {
      const page = this.store.doc.pages.get(pageId);
      if (!page) return;
      const row = document.createElement('div');
      Object.assign(row.style, {
        display: 'grid',
        gridTemplateColumns: 'minmax(34px, 44px) minmax(0, 1fr)',
        gap: '10px',
        alignItems: 'center',
        padding: '10px',
        background: '#fff',
        border: '1px solid #e2e8f0',
        borderRadius: '8px'
      });

      const num = document.createElement('div');
      num.textContent = String(index + 1);
      Object.assign(num.style, { fontWeight: '800', color: '#0f172a', textAlign: 'center' });
      row.appendChild(num);

      const settings = document.createElement('div');
      const name = document.createElement('div');
      name.textContent = pageSizeLabel(page.width, page.height);
      name.title = `${Math.round(page.width)} × ${Math.round(page.height)} px`;
      Object.assign(name.style, { fontSize: '13px', fontWeight: '700', color: '#0f172a', marginBottom: '6px' });
      settings.appendChild(name);
      const select = document.createElement('select');
      for (const tpl of ['default', 'agenda', 'diary'] as const) {
        const opt = document.createElement('option');
        opt.value = tpl;
        opt.textContent = tpl;
        select.appendChild(opt);
      }
      select.value = page.background.kind === 'template' ? page.background.template : 'default';
      Object.assign(select.style, { height: '30px', borderRadius: '6px', border: '1px solid #cbd5e1', padding: '0 8px' });
      select.addEventListener('change', () => {
        const background: Background = { kind: 'template', template: select.value as 'default' | 'agenda' | 'diary' };
        this.store.apply({ type: 'set-page-background', pageId, background });
      });
      settings.appendChild(select);
      row.appendChild(settings);

      const actions = document.createElement('div');
      Object.assign(actions.style, {
        gridColumn: '1 / -1',
        display: 'flex',
        gap: '6px',
        flexWrap: 'wrap',
        justifyContent: 'flex-end'
      });
      actions.appendChild(this.actionButton('Up', index === 0, () => {
        this.store.apply({ type: 'move-page', pageId, toIndex: index - 1 });
        this.renderer.rebuild();
        this.render(body);
      }));
      actions.appendChild(this.actionButton('Down', index === this.store.doc.pageOrder.length - 1, () => {
        this.store.apply({ type: 'move-page', pageId, toIndex: index + 1 });
        this.renderer.rebuild();
        this.render(body);
      }));
      actions.appendChild(this.actionButton('Insert After', false, () => {
        const newPage = createPage({ width: page.width, height: page.height, background: { ...page.background } });
        this.store.apply({ type: 'add-page', page: serializePage(newPage), index: index + 1 });
        this.renderer.rebuild();
        this.render(body);
      }));
      actions.appendChild(this.actionButton('Delete', this.store.doc.pageOrder.length <= 1, async () => {
        const ok = await confirmAction('Delete page', `Delete page ${index + 1}?`, 'Delete');
        if (!ok) return;
        this.store.apply({ type: 'remove-page', pageId });
        this.renderer.rebuild();
        this.render(body);
      }, 'danger'));
      row.appendChild(actions);
      list.appendChild(row);
    });
  }

  private actionButton(label: string, disabled: boolean, run: () => void, kind: 'plain' | 'danger' = 'plain'): HTMLButtonElement {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.textContent = label;
    btn.disabled = disabled;
    Object.assign(btn.style, buttonStyle(kind));
    if (disabled) {
      btn.style.opacity = '0.45';
      btn.style.cursor = 'default';
    }
    btn.addEventListener('click', run);
    return btn;
  }
}
