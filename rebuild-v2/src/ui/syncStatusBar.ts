import type { SyncStatus } from '../sync/syncMachine';

export type SyncStatusTone = 'neutral' | 'ok' | 'warning' | 'busy' | 'danger';

export interface SyncStatusDisplay {
  tone: SyncStatusTone;
  label: string;
  detail: string;
  primaryAction: 'sync' | null;
  showConflictActions: boolean;
}

export function describeSyncStatus(status: SyncStatus, now = Date.now()): SyncStatusDisplay {
  if (status.error) {
    return {
      tone: 'danger',
      label: 'Sync issue',
      detail: status.error,
      primaryAction: status.state === 'localDirty' || status.state === 'pdfStale' ? 'sync' : null,
      showConflictActions: status.state === 'conflict'
    };
  }

  if (status.saving || status.state === 'syncing') {
    return {
      tone: 'busy',
      label: 'Syncing',
      detail: 'Uploading changes to Drive',
      primaryAction: null,
      showConflictActions: false
    };
  }

  switch (status.state) {
    case 'localDirty':
      return {
        tone: 'warning',
        label: 'Unsynced changes',
        detail: status.lastSaved ? `Saved locally ${formatRelativeTime(status.lastSaved, now)}` : 'Waiting to sync',
        primaryAction: 'sync',
        showConflictActions: false
      };
    case 'pdfStale':
      return {
        tone: 'warning',
        label: 'PDF pending',
        detail: 'Document data is saved; PDF refresh is queued',
        primaryAction: 'sync',
        showConflictActions: false
      };
    case 'conflict':
      return {
        tone: 'danger',
        label: 'Conflict',
        detail: 'Remote changes need review',
        primaryAction: null,
        showConflictActions: true
      };
    case 'idle':
      if (status.lastDriveSyncAt) {
        return {
          tone: 'ok',
          label: 'Synced',
          detail: `Drive updated ${formatRelativeTime(status.lastDriveSyncAt, now)}`,
          primaryAction: null,
          showConflictActions: false
        };
      }
      if (status.lastSaved) {
        return {
          tone: 'ok',
          label: 'Saved locally',
          detail: formatRelativeTime(status.lastSaved, now),
          primaryAction: null,
          showConflictActions: false
        };
      }
      return {
        tone: 'neutral',
        label: 'Ready',
        detail: 'No changes yet',
        primaryAction: null,
        showConflictActions: false
      };
  }
}

export function formatRelativeTime(ts: number, now = Date.now()): string {
  const seconds = Math.max(0, Math.round((now - ts) / 1000));
  if (seconds < 5) return 'just now';
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.round(hours / 24);
  return `${days}d ago`;
}

interface SyncStatusBarOptions {
  onSyncNow: () => void;
  onAcceptRemote: () => void;
  onKeepLocal: () => void;
}

const TONE_COLOR: Record<SyncStatusTone, string> = {
  neutral: '#64748b',
  ok: '#0f766e',
  warning: '#b45309',
  busy: '#2563eb',
  danger: '#b91c1c'
};

export class SyncStatusBar {
  readonly el: HTMLDivElement;
  private dot: HTMLSpanElement;
  private label: HTMLSpanElement;
  private detail: HTMLSpanElement;
  private syncButton: HTMLButtonElement;
  private acceptRemoteButton: HTMLButtonElement;
  private keepLocalButton: HTMLButtonElement;
  private current: SyncStatus | null = null;
  private timer: ReturnType<typeof setInterval>;

  constructor(
    container: HTMLElement,
    private options: SyncStatusBarOptions
  ) {
    this.el = document.createElement('div');
    this.el.className = 'sync-status-bar';
    this.el.setAttribute('role', 'status');
    this.el.setAttribute('aria-live', 'polite');
    Object.assign(this.el.style, {
      position: 'absolute',
      top: '18px',
      right: '18px',
      zIndex: '120',
      display: 'flex',
      alignItems: 'center',
      gap: '8px',
      maxWidth: 'min(420px, calc(100vw - 36px))',
      minHeight: '38px',
      padding: '7px 9px',
      borderRadius: '8px',
      border: '1px solid rgba(15,23,42,0.12)',
      background: 'rgba(255,255,255,0.96)',
      boxShadow: '0 2px 10px rgba(15,23,42,0.12)',
      fontFamily: 'system-ui, sans-serif',
      userSelect: 'none',
      pointerEvents: 'auto'
    });

    this.dot = document.createElement('span');
    Object.assign(this.dot.style, {
      width: '8px',
      height: '8px',
      borderRadius: '50%',
      flex: '0 0 auto'
    });
    this.el.appendChild(this.dot);

    const text = document.createElement('span');
    Object.assign(text.style, {
      display: 'grid',
      minWidth: '0',
      gap: '1px'
    });
    this.label = document.createElement('span');
    Object.assign(this.label.style, {
      fontSize: '13px',
      fontWeight: '700',
      lineHeight: '16px',
      color: '#0f172a',
      whiteSpace: 'nowrap'
    });
    this.detail = document.createElement('span');
    Object.assign(this.detail.style, {
      fontSize: '12px',
      lineHeight: '14px',
      color: '#475569',
      overflow: 'hidden',
      textOverflow: 'ellipsis',
      whiteSpace: 'nowrap'
    });
    text.append(this.label, this.detail);
    this.el.appendChild(text);

    this.syncButton = this.makeButton('Sync');
    this.syncButton.addEventListener('click', () => this.options.onSyncNow());
    this.el.appendChild(this.syncButton);

    this.acceptRemoteButton = this.makeButton('Remote');
    this.acceptRemoteButton.title = 'Use remote version';
    this.acceptRemoteButton.addEventListener('click', () => this.options.onAcceptRemote());
    this.el.appendChild(this.acceptRemoteButton);

    this.keepLocalButton = this.makeButton('Local');
    this.keepLocalButton.title = 'Keep local version';
    this.keepLocalButton.addEventListener('click', () => this.options.onKeepLocal());
    this.el.appendChild(this.keepLocalButton);

    container.appendChild(this.el);
    this.timer = setInterval(() => {
      if (this.current) this.update(this.current);
    }, 30000);
  }

  update(status: SyncStatus): void {
    this.current = status;
    const display = describeSyncStatus(status);
    const color = TONE_COLOR[display.tone];
    this.dot.style.background = color;
    this.dot.style.boxShadow = display.tone === 'busy' ? `0 0 0 3px ${color}22` : 'none';
    this.label.textContent = display.label;
    this.detail.textContent = display.detail;
    this.el.dataset.tone = display.tone;

    this.syncButton.hidden = display.primaryAction !== 'sync';
    this.acceptRemoteButton.hidden = !display.showConflictActions;
    this.keepLocalButton.hidden = !display.showConflictActions;
  }

  dispose(): void {
    clearInterval(this.timer);
    this.el.remove();
  }

  private makeButton(label: string): HTMLButtonElement {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.textContent = label;
    Object.assign(btn.style, {
      height: '28px',
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
    return btn;
  }
}
