// Shared modal scaffold. Styled with the legacy design tokens (see
// ui/legacyTheme.ts) so every panel — timeline, pages, share, calendar —
// matches the original app's look.

export function makeOverlay(title: string): {
  overlay: HTMLDivElement;
  panel: HTMLDivElement;
  body: HTMLDivElement;
  close: () => void;
  onClose: (handler: () => void) => () => void;
} {
  const overlay = document.createElement('div');
  overlay.setAttribute('role', 'dialog');
  overlay.setAttribute('aria-modal', 'true');
  Object.assign(overlay.style, {
    position: 'absolute',
    inset: '0',
    zIndex: '1500',
    display: 'grid',
    placeItems: 'center',
    background: 'rgba(0,0,0,0.32)',
    backdropFilter: 'blur(2px)',
    fontFamily: "'DM Sans', -apple-system, BlinkMacSystemFont, sans-serif"
  });

  const panel = document.createElement('div');
  Object.assign(panel.style, {
    width: 'min(920px, calc(100vw - 32px))',
    maxHeight: 'min(760px, calc(100vh - 32px))',
    display: 'grid',
    gridTemplateRows: 'auto 1fr',
    borderRadius: 'var(--radius-lg, 16px)',
    background: 'var(--bg-secondary, #fff)',
    color: 'var(--text-primary, #1a1a1a)',
    border: 'none',
    boxShadow: 'var(--shadow-medium, 0 4px 30px rgba(0,0,0,0.12))',
    overflow: 'hidden'
  });
  overlay.appendChild(panel);

  const head = document.createElement('div');
  Object.assign(head.style, {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
    gap: '12px',
    padding: '16px 20px',
    borderBottom: '1px solid rgba(0,0,0,0.06)'
  });
  panel.appendChild(head);

  const h = document.createElement('h2');
  h.textContent = title;
  Object.assign(h.style, {
    margin: '0',
    fontSize: '1.05rem',
    fontWeight: '600',
    lineHeight: '20px',
    color: 'var(--text-primary, #1a1a1a)'
  });
  head.appendChild(h);

  const closeBtn = document.createElement('button');
  closeBtn.type = 'button';
  closeBtn.textContent = 'Close';
  Object.assign(closeBtn.style, buttonStyle());
  head.appendChild(closeBtn);

  const body = document.createElement('div');
  Object.assign(body.style, {
    minHeight: '0',
    overflow: 'auto',
    padding: '18px 20px',
    background: 'var(--bg-primary, #f5f5f0)'
  });
  panel.appendChild(body);

  const closeHandlers = new Set<() => void>();
  let closed = false;
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === 'Escape') close();
  };
  const close = () => {
    if (closed) return;
    closed = true;
    document.removeEventListener('keydown', onKeyDown);
    overlay.remove();
    for (const handler of closeHandlers) handler();
    closeHandlers.clear();
  };
  const onClose = (handler: () => void) => {
    closeHandlers.add(handler);
    return () => closeHandlers.delete(handler);
  };
  closeBtn.addEventListener('click', close);
  overlay.addEventListener('click', (event) => {
    if (event.target === overlay) close();
  });
  document.addEventListener('keydown', onKeyDown);
  return { overlay, panel, body, close, onClose };
}

export interface PromptTextOptions {
  label: string;
  initialValue?: string;
  placeholder?: string;
  inputType?: 'text' | 'url' | 'email';
  confirmLabel?: string;
}

export function promptText(title: string, options: PromptTextOptions): Promise<string | null> {
  return new Promise((resolve) => {
    const modal = makeOverlay(title);
    modal.panel.style.width = 'min(460px, calc(100vw - 32px))';
    let settled = false;
    const finish = (value: string | null) => {
      if (settled) return;
      settled = true;
      resolve(value);
      modal.close();
    };
    modal.onClose(() => finish(null));

    const form = document.createElement('form');
    Object.assign(form.style, { display: 'grid', gap: '14px' });
    const label = document.createElement('label');
    label.textContent = options.label;
    Object.assign(label.style, { display: 'grid', gap: '7px', fontSize: '0.85rem', fontWeight: '600' });
    const input = document.createElement('input');
    input.type = options.inputType ?? 'text';
    input.value = options.initialValue ?? '';
    input.placeholder = options.placeholder ?? '';
    input.required = true;
    Object.assign(input.style, inputStyle());
    label.appendChild(input);
    form.appendChild(label);

    const actions = document.createElement('div');
    Object.assign(actions.style, { display: 'flex', justifyContent: 'flex-end', gap: '10px' });
    const cancel = document.createElement('button');
    cancel.type = 'button';
    cancel.textContent = 'Cancel';
    Object.assign(cancel.style, buttonStyle());
    cancel.addEventListener('click', () => finish(null));
    const confirm = document.createElement('button');
    confirm.type = 'submit';
    confirm.textContent = options.confirmLabel ?? 'Continue';
    Object.assign(confirm.style, buttonStyle('primary'));
    actions.append(cancel, confirm);
    form.appendChild(actions);
    form.addEventListener('submit', (event) => {
      event.preventDefault();
      const value = input.value.trim();
      if (value) finish(value);
    });
    modal.body.appendChild(form);
    document.body.appendChild(modal.overlay);
    queueMicrotask(() => {
      input.focus();
      input.select();
    });
  });
}

export function confirmAction(title: string, message: string, confirmLabel = 'Continue'): Promise<boolean> {
  return new Promise((resolve) => {
    const modal = makeOverlay(title);
    modal.panel.style.width = 'min(460px, calc(100vw - 32px))';
    let settled = false;
    const finish = (value: boolean) => {
      if (settled) return;
      settled = true;
      resolve(value);
      modal.close();
    };
    modal.onClose(() => finish(false));
    const messageEl = document.createElement('div');
    messageEl.textContent = message;
    Object.assign(messageEl.style, { fontSize: '0.9rem', lineHeight: '1.45' });
    const actions = document.createElement('div');
    Object.assign(actions.style, { display: 'flex', justifyContent: 'flex-end', gap: '10px', marginTop: '18px' });
    const cancel = document.createElement('button');
    cancel.type = 'button';
    cancel.textContent = 'Cancel';
    Object.assign(cancel.style, buttonStyle());
    cancel.addEventListener('click', () => finish(false));
    const confirm = document.createElement('button');
    confirm.type = 'button';
    confirm.textContent = confirmLabel;
    Object.assign(confirm.style, buttonStyle('primary'));
    confirm.addEventListener('click', () => finish(true));
    actions.append(cancel, confirm);
    modal.body.append(messageEl, actions);
    document.body.appendChild(modal.overlay);
    queueMicrotask(() => confirm.focus());
  });
}

export function showMessage(title: string, message: string): void {
  const modal = makeOverlay(title);
  modal.panel.style.width = 'min(460px, calc(100vw - 32px))';
  const messageEl = document.createElement('div');
  messageEl.textContent = message;
  Object.assign(messageEl.style, { fontSize: '0.9rem', lineHeight: '1.45', whiteSpace: 'pre-wrap' });
  modal.body.appendChild(messageEl);
  document.body.appendChild(modal.overlay);
}

export function buttonStyle(kind: 'primary' | 'plain' | 'danger' = 'plain'): Partial<CSSStyleDeclaration> {
  return {
    height: '36px',
    padding: '0 16px',
    borderRadius: 'var(--radius-md, 12px)',
    border: 'none',
    background:
      kind === 'primary'
        ? 'var(--accent-yellow, #ffde00)'
        : kind === 'danger'
          ? 'rgba(232,16,16,0.10)'
          : 'var(--bg-primary, #f5f5f0)',
    color: kind === 'danger' ? 'var(--accent-red, #E81010)' : 'var(--text-primary, #1a1a1a)',
    fontFamily: 'inherit',
    fontSize: '0.85rem',
    fontWeight: '600',
    cursor: 'pointer',
    whiteSpace: 'nowrap',
    boxShadow: 'var(--shadow-soft, 0 2px 20px rgba(0,0,0,0.08))'
  };
}

export function smallText(text: string): HTMLDivElement {
  const el = document.createElement('div');
  el.textContent = text;
  Object.assign(el.style, {
    color: 'var(--text-secondary, #666)',
    fontSize: '0.8rem',
    lineHeight: '16px'
  });
  return el;
}

function inputStyle(): Partial<CSSStyleDeclaration> {
  return {
    boxSizing: 'border-box',
    width: '100%',
    height: '42px',
    padding: '0 12px',
    borderRadius: 'var(--radius-sm, 8px)',
    border: '1px solid rgba(0,0,0,0.14)',
    background: 'var(--bg-secondary, #fff)',
    color: 'var(--text-primary, #1a1a1a)',
    font: 'inherit',
    outline: 'none'
  };
}
