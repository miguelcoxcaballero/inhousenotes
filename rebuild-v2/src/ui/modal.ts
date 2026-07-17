// Shared modal scaffold. Styled with the legacy design tokens (see
// ui/legacyTheme.ts) so every panel — timeline, pages, share, calendar —
// matches the original app's look.

export function makeOverlay(title: string): { overlay: HTMLDivElement; panel: HTMLDivElement; body: HTMLDivElement; close: () => void } {
  const overlay = document.createElement('div');
  Object.assign(overlay.style, {
    position: 'absolute',
    inset: '0',
    zIndex: '200',
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

  const close = () => overlay.remove();
  closeBtn.addEventListener('click', close);
  overlay.addEventListener('click', (event) => {
    if (event.target === overlay) close();
  });
  return { overlay, panel, body, close };
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
