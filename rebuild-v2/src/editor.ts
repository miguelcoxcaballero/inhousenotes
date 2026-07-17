// Editor: wires GestureHandler + PointerPipeline + DrawTools into a live
// editing session on top of a DocRenderer. Owns the toolbar UI.

import type { DocStore } from './core/store';
import type { DocRenderer } from './render/docRenderer';
import { GestureHandler } from './input/gestures';
import { PointerPipeline } from './input/pointerPipeline';
import { InputRouter } from './input/router';
import {
  PenTool, EraserStrokeTool, EraserAreaTool, LassoTool,
  defaultTools, type ToolDefinition, type DrawTool, type ToolEnv
} from './input/tools';
import { History } from './core/history';
import type { Img } from './core/model';
import { newId } from './core/ids';

export interface EditorAction {
  id: string;
  label: string;
  title: string;
  run: () => void;
}

export class Editor {
  private gesture: GestureHandler;
  private pointer: PointerPipeline;
  private history: History;
  private unsubHistory: () => void;
  private activeTool: DrawTool;
  private activeToolDef: ToolDefinition;
  private tools: Map<string, DrawTool>;
  private lassoTool: LassoTool;
  private toolbar: HTMLElement;
  private actionBar: HTMLElement;
  private colorInput: HTMLInputElement | null = null;
  private widthInput: HTMLInputElement | null = null;
  private deleteSelectionBtn: HTMLButtonElement | null = null;
  private theme: 'light' | 'dark' = loadTheme();
  private themeBtn: HTMLButtonElement | null = null;
  private readonly keyHandler = (event: KeyboardEvent) => this.onKeyDown(event);

  constructor(
    private viewportEl: HTMLElement,
    private store: DocStore,
    private renderer: DocRenderer,
    private onCreatePage: () => void,
    private actions: EditorAction[] = []
  ) {
    this.history = new History();
    this.unsubHistory = store.subscribe((applied) => this.history.record(applied));
    const env: ToolEnv = { store, renderer };
    this.lassoTool = new LassoTool(env);
    this.lassoTool.onSelectionChange = () => this.updateSelectionControls();

    this.tools = new Map<string, DrawTool>([
      ['pen', new PenTool(env, () => this.currentStyle())],
      ['highlighter', new PenTool(env, () => this.currentStyle())],
      ['eraserStroke', new EraserStrokeTool(env, () => this.activeToolDef.width)],
      ['eraserArea', new EraserAreaTool(env, () => this.activeToolDef.width)],
      ['lasso', this.lassoTool],
    ]);

    this.activeToolDef = defaultTools[0]!;
    this.activeTool = this.tools.get('pen')!;

    // Shared router: pen/mouse draw, fingers pan, two fingers pinch.
    const router = new InputRouter();
    this.gesture = new GestureHandler(
      viewportEl,
      { get: () => renderer.camera, set: (c) => renderer.setCamera(c) },
      (c) => renderer.setCamera(c),
      () => onCreatePage(),
      () => renderer.contentBounds(),
      router
    );

    this.pointer = new PointerPipeline(viewportEl, {
      onStrokeStart: (pageId, point) => this.activeTool.begin(pageId, point),
      onStrokeMove: (pageId, points, predicted) => this.activeTool.move(points, predicted),
      onStrokeEnd: (_pageId, point) => this.activeTool.end(point),
      onStrokeCancel: () => this.activeTool.cancel(),
    }, router);
    this.pointer.setRenderer((sx, sy) => renderer.pageAt(sx, sy));

    this.toolbar = this.buildToolbar();
    this.actionBar = this.buildActionBar();
    // Activate the default tool only now that this.toolbar is assigned —
    // setTool reads it to highlight the active button.
    this.setTool(this.activeToolDef.id);
    viewportEl.appendChild(this.actionBar);
    viewportEl.appendChild(this.toolbar);
    this.restoreToolbarPosition();
    this.enableToolbarDrag();
    this.applyTheme();
    window.addEventListener('keydown', this.keyHandler);
  }

  setTool(id: string): void {
    this.activeTool.cancel();
    const def = defaultTools.find((t) => t.id === id);
    if (def) this.activeToolDef = def;
    this.activeTool = this.tools.get(id) ?? this.tools.get('pen')!;
    this.toolbar.querySelectorAll<HTMLButtonElement>('[data-tool]').forEach((btn) => {
      btn.classList.toggle('active', btn.dataset.tool === id);
    });
    this.syncStyleInputs();
  }

  undo(): void { this.history.undo(this.store); }
  redo(): void { this.history.redo(this.store); }

  dispose(): void {
    this.activeTool.cancel();
    this.unsubHistory();
    this.gesture.destroy();
    this.pointer.destroy();
    window.removeEventListener('keydown', this.keyHandler);
    this.actionBar.remove();
    this.toolbar.remove();
    this.history.clear();
  }

  private currentStyle() {
    return {
      tool: this.activeToolDef.tool,
      color: this.activeToolDef.color,
      width: this.activeToolDef.width,
    };
  }

  private onKeyDown(event: KeyboardEvent): void {
    const target = event.target as HTMLElement | null;
    if (target?.closest('input, textarea, select, [contenteditable="true"]')) return;

    const key = event.key.toLowerCase();
    if ((event.ctrlKey || event.metaKey) && key === 'z') {
      event.preventDefault();
      if (event.shiftKey) this.redo();
      else this.undo();
      return;
    }
    if ((event.ctrlKey || event.metaKey) && key === 'y') {
      event.preventDefault();
      this.redo();
      return;
    }
    if (event.ctrlKey || event.metaKey || event.altKey) return;

    const toolByKey: Record<string, string> = {
      p: 'pen',
      h: 'highlighter',
      e: 'eraserStroke',
      a: 'eraserArea',
      l: 'lasso'
    };
    const tool = toolByKey[key];
    if (tool) {
      event.preventDefault();
      this.setTool(tool);
      return;
    }
    if (event.key === '[' || event.key === ']') {
      event.preventDefault();
      const delta = event.key === ']' ? 1 : -1;
      this.activeToolDef.width = Math.max(1, Math.min(36, this.activeToolDef.width + delta));
      this.syncStyleInputs();
    }
  }

  private buildToolbar(): HTMLElement {
    const bar = document.createElement('div');
    bar.className = 'toolbar';
    Object.assign(bar.style, {
      position: 'absolute', bottom: '24px', left: '50%',
      transform: 'translateX(-50%)',
      display: 'flex', gap: '8px', alignItems: 'center',
      background: 'rgba(255,255,255,0.95)',
      boxShadow: '0 2px 12px rgba(0,0,0,0.18)',
      borderRadius: '32px', padding: '8px 16px',
      zIndex: '100', userSelect: 'none',
    });

    const drag = document.createElement('button');
    drag.type = 'button';
    drag.dataset.dragHandle = 'true';
    drag.title = 'Move toolbar';
    drag.textContent = '::';
    Object.assign(drag.style, {
      width: '26px',
      height: '36px',
      border: 'none',
      borderRadius: '14px',
      background: 'transparent',
      cursor: 'grab',
      color: '#64748b',
      fontSize: '14px',
      fontWeight: '900'
    });
    bar.appendChild(drag);

    for (const def of defaultTools) {
      const btn = document.createElement('button');
      btn.dataset.tool = def.id;
      btn.title = def.name;
      btn.textContent = def.icon;
      Object.assign(btn.style, {
        width: '40px', height: '40px', border: 'none', borderRadius: '50%',
        background: 'transparent', cursor: 'pointer', fontSize: '20px',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        transition: 'background 0.15s',
      });
      btn.addEventListener('click', () => this.setTool(def.id));
      bar.appendChild(btn);
    }

    // Undo / redo
    const sep = document.createElement('div');
    Object.assign(sep.style, { width: '1px', height: '28px', background: '#e0e0e0', margin: '0 4px' });
    bar.appendChild(sep);

    for (const [label, fn] of [['↩', () => this.undo()], ['↪', () => this.redo()]] as const) {
      const btn = document.createElement('button');
      btn.textContent = label;
      btn.title = label === '↩' ? 'Undo' : 'Redo';
      Object.assign(btn.style, {
        width: '36px', height: '36px', border: 'none', borderRadius: '50%',
        background: 'transparent', cursor: 'pointer', fontSize: '18px',
      });
      btn.addEventListener('click', fn);
      bar.appendChild(btn);
    }

    this.colorInput = document.createElement('input');
    this.colorInput.type = 'color';
    this.colorInput.title = 'Color';
    Object.assign(this.colorInput.style, {
      width: '34px',
      height: '34px',
      border: 'none',
      borderRadius: '50%',
      padding: '0',
      background: 'transparent',
      cursor: 'pointer'
    });
    this.colorInput.addEventListener('input', () => {
      if (this.colorInput) this.activeToolDef.color = this.colorInput.value;
    });
    bar.appendChild(this.colorInput);

    this.widthInput = document.createElement('input');
    this.widthInput.type = 'range';
    this.widthInput.min = '1';
    this.widthInput.max = '36';
    this.widthInput.step = '1';
    this.widthInput.title = 'Width';
    Object.assign(this.widthInput.style, { width: '82px' });
    this.widthInput.addEventListener('input', () => {
      if (this.widthInput) this.activeToolDef.width = Number(this.widthInput.value);
    });
    bar.appendChild(this.widthInput);

    this.themeBtn = document.createElement('button');
    this.themeBtn.type = 'button';
    this.themeBtn.title = 'Toggle theme';
    this.themeBtn.textContent = this.theme === 'dark' ? 'Light' : 'Dark';
    Object.assign(this.themeBtn.style, {
      height: '36px',
      border: 'none',
      borderRadius: '18px',
      background: 'transparent',
      cursor: 'pointer',
      fontSize: '12px',
      fontWeight: '700',
      padding: '0 10px'
    });
    this.themeBtn.addEventListener('click', () => {
      this.theme = this.theme === 'dark' ? 'light' : 'dark';
      localStorage.setItem('ihn_editor_theme', this.theme);
      this.applyTheme();
    });
    bar.appendChild(this.themeBtn);

    const imageBtn = document.createElement('button');
    imageBtn.type = 'button';
    imageBtn.textContent = 'Image';
    imageBtn.title = 'Insert image';
    Object.assign(imageBtn.style, {
      height: '36px',
      border: 'none',
      borderRadius: '18px',
      background: 'transparent',
      cursor: 'pointer',
      fontSize: '12px',
      fontWeight: '700',
      padding: '0 10px'
    });
    imageBtn.addEventListener('click', () => this.pickImage());
    bar.appendChild(imageBtn);

    this.deleteSelectionBtn = document.createElement('button');
    this.deleteSelectionBtn.type = 'button';
    this.deleteSelectionBtn.textContent = 'Delete';
    this.deleteSelectionBtn.title = 'Delete lasso selection';
    this.deleteSelectionBtn.hidden = true;
    Object.assign(this.deleteSelectionBtn.style, {
      height: '36px',
      border: 'none',
      borderRadius: '18px',
      background: '#fff1f2',
      color: '#991b1b',
      cursor: 'pointer',
      fontSize: '12px',
      fontWeight: '700',
      padding: '0 10px'
    });
    this.deleteSelectionBtn.addEventListener('click', () => this.lassoTool.deleteSelection());
    bar.appendChild(this.deleteSelectionBtn);

    // Add CSS for active state
    const style = document.createElement('style');
    style.textContent = `.toolbar [data-tool].active { background: #e8eeff; }`;
    bar.appendChild(style);

    return bar;
  }

  private restoreToolbarPosition(): void {
    const raw = localStorage.getItem('ihn_toolbar_pos');
    if (!raw) return;
    try {
      const pos = JSON.parse(raw) as { x: number; y: number };
      if (!Number.isFinite(pos.x) || !Number.isFinite(pos.y)) return;
      this.toolbar.style.left = `${pos.x}px`;
      this.toolbar.style.top = `${pos.y}px`;
      this.toolbar.style.bottom = '';
      this.toolbar.style.transform = 'none';
    } catch {
      localStorage.removeItem('ihn_toolbar_pos');
    }
  }

  private enableToolbarDrag(): void {
    const handle = this.toolbar.querySelector<HTMLElement>('[data-drag-handle]');
    if (!handle) return;
    let start: { pointerId: number; dx: number; dy: number } | null = null;
    handle.addEventListener('pointerdown', (event) => {
      const rect = this.toolbar.getBoundingClientRect();
      start = { pointerId: event.pointerId, dx: event.clientX - rect.left, dy: event.clientY - rect.top };
      handle.setPointerCapture(event.pointerId);
      handle.style.cursor = 'grabbing';
      event.preventDefault();
    });
    handle.addEventListener('pointermove', (event) => {
      if (!start || start.pointerId !== event.pointerId) return;
      const host = this.viewportEl.getBoundingClientRect();
      const rect = this.toolbar.getBoundingClientRect();
      const x = clamp(event.clientX - host.left - start.dx, 8, Math.max(8, host.width - rect.width - 8));
      const y = clamp(event.clientY - host.top - start.dy, 8, Math.max(8, host.height - rect.height - 8));
      this.toolbar.style.left = `${x}px`;
      this.toolbar.style.top = `${y}px`;
      this.toolbar.style.bottom = '';
      this.toolbar.style.transform = 'none';
    });
    const finish = (event: PointerEvent) => {
      if (!start || start.pointerId !== event.pointerId) return;
      start = null;
      handle.style.cursor = 'grab';
      const host = this.viewportEl.getBoundingClientRect();
      const rect = this.toolbar.getBoundingClientRect();
      const x = rect.left - host.left;
      const y = rect.top - host.top;
      if (host.height - (y + rect.height) < 54) {
        this.toolbar.style.left = '50%';
        this.toolbar.style.top = '';
        this.toolbar.style.bottom = '24px';
        this.toolbar.style.transform = 'translateX(-50%)';
        localStorage.removeItem('ihn_toolbar_pos');
      } else {
        localStorage.setItem('ihn_toolbar_pos', JSON.stringify({ x, y }));
      }
    };
    handle.addEventListener('pointerup', finish);
    handle.addEventListener('pointercancel', finish);
  }

  private applyTheme(): void {
    const dark = this.theme === 'dark';
    this.viewportEl.dataset.theme = this.theme;
    this.viewportEl.style.background = dark ? '#111827' : '#f3f4f6';
    if (this.themeBtn) this.themeBtn.textContent = dark ? 'Light' : 'Dark';
    Object.assign(this.toolbar.style, {
      background: dark ? 'rgba(17,24,39,0.96)' : 'rgba(255,255,255,0.95)',
      color: dark ? '#f8fafc' : '#0f172a',
      boxShadow: dark ? '0 2px 16px rgba(0,0,0,0.35)' : '0 2px 12px rgba(0,0,0,0.18)'
    });
    this.toolbar.querySelectorAll<HTMLElement>('button').forEach((button) => {
      if (button === this.deleteSelectionBtn) return;
      button.style.color = dark ? '#f8fafc' : '#0f172a';
    });
    Object.assign(this.actionBar.style, {
      background: dark ? 'rgba(17,24,39,0.96)' : 'rgba(255,255,255,0.96)',
      borderColor: dark ? 'rgba(148,163,184,0.30)' : 'rgba(15,23,42,0.12)',
      color: dark ? '#f8fafc' : '#0f172a'
    });
    this.actionBar.querySelectorAll<HTMLButtonElement>('button').forEach((button) => {
      button.style.background = dark ? '#1f2937' : '#f8fafc';
      button.style.color = dark ? '#f8fafc' : '#0f172a';
      button.style.borderColor = dark ? 'rgba(148,163,184,0.30)' : 'rgba(15,23,42,0.14)';
    });
  }

  private syncStyleInputs(): void {
    if (this.colorInput) this.colorInput.value = normalizeColor(this.activeToolDef.color);
    if (this.widthInput) this.widthInput.value = String(this.activeToolDef.width);
  }

  private updateSelectionControls(): void {
    if (this.deleteSelectionBtn) this.deleteSelectionBtn.hidden = !this.lassoTool.selection;
  }

  private pickImage(): void {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = 'image/*';
    input.addEventListener('change', () => {
      const file = input.files?.[0];
      if (!file) return;
      const reader = new FileReader();
      reader.addEventListener('load', () => {
        if (typeof reader.result === 'string') this.insertImage(reader.result);
      });
      reader.readAsDataURL(file);
    });
    input.click();
  }

  private insertImage(src: string): void {
    const pageId = this.store.doc.pageOrder[0];
    if (!pageId) return;
    const page = this.store.doc.pages.get(pageId);
    if (!page) return;
    const image: Img = {
      id: newId(),
      src,
      x: page.width / 2,
      y: page.height / 2,
      width: Math.min(260, page.width * 0.5),
      height: Math.min(180, page.height * 0.25),
      rotation: 0
    };
    this.store.apply({
      type: 'splice-images',
      pageId,
      remove: [],
      add: [{ image, index: Number.MAX_SAFE_INTEGER }]
    });
  }

  private buildActionBar(): HTMLElement {
    const bar = document.createElement('div');
    bar.className = 'editor-action-bar';
    Object.assign(bar.style, {
      position: 'absolute',
      top: '18px',
      left: '18px',
      zIndex: '115',
      display: 'flex',
      gap: '6px',
      alignItems: 'center',
      padding: '6px',
      borderRadius: '8px',
      border: '1px solid rgba(15,23,42,0.12)',
      background: 'rgba(255,255,255,0.96)',
      boxShadow: '0 2px 10px rgba(15,23,42,0.12)',
      fontFamily: 'system-ui, sans-serif',
      userSelect: 'none'
    });

    for (const action of this.actions) {
      const btn = document.createElement('button');
      btn.type = 'button';
      btn.dataset.action = action.id;
      btn.textContent = action.label;
      btn.title = action.title;
      Object.assign(btn.style, {
        height: '30px',
        padding: '0 10px',
        borderRadius: '7px',
        border: '1px solid rgba(15,23,42,0.14)',
        background: '#f8fafc',
        color: '#0f172a',
        fontSize: '12px',
        fontWeight: '700',
        cursor: 'pointer',
        whiteSpace: 'nowrap'
      });
      btn.addEventListener('click', action.run);
      bar.appendChild(btn);
    }

    return bar;
  }
}

function normalizeColor(color: string): string {
  return /^#[0-9a-f]{6}$/i.test(color) ? color : '#1a1a1a';
}

function loadTheme(): 'light' | 'dark' {
  return localStorage.getItem('ihn_editor_theme') === 'dark' ? 'dark' : 'light';
}

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}
