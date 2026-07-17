import { describe, expect, it } from 'vitest';
import type { SyncStatus } from '../sync/syncMachine';
import { describeSyncStatus, formatRelativeTime } from './syncStatusBar';

const NOW = 1_700_000_000_000;

function status(partial: Partial<SyncStatus>): SyncStatus {
  return {
    state: 'idle',
    saving: false,
    driveEnabled: true,
    lastSaved: null,
    lastDriveSyncAt: null,
    error: null,
    ...partial
  };
}

describe('describeSyncStatus', () => {
  it('shows local dirty state with a sync action', () => {
    const display = describeSyncStatus(status({ state: 'localDirty', lastSaved: NOW - 60_000 }), NOW);
    expect(display.label).toBe('Unsynced changes');
    expect(display.detail).toBe('Saved locally 1m ago');
    expect(display.tone).toBe('warning');
    expect(display.primaryAction).toBe('sync');
  });

  it('shows local-only saves without a Drive action while offline', () => {
    const display = describeSyncStatus(status({
      state: 'localDirty',
      driveEnabled: false,
      lastSaved: NOW - 10_000
    }), NOW);
    expect(display.label).toBe('Saved locally');
    expect(display.primaryAction).toBeNull();
  });

  it('shows active Drive sync as busy', () => {
    const display = describeSyncStatus(status({ state: 'syncing', saving: true }), NOW);
    expect(display.label).toBe('Syncing');
    expect(display.tone).toBe('busy');
    expect(display.primaryAction).toBeNull();
  });

  it('shows conflicts with resolution actions', () => {
    const display = describeSyncStatus(status({ state: 'conflict' }), NOW);
    expect(display.label).toBe('Conflict');
    expect(display.tone).toBe('danger');
    expect(display.showConflictActions).toBe(true);
  });

  it('distinguishes local saves from Drive syncs', () => {
    expect(describeSyncStatus(status({ lastSaved: NOW - 10_000 }), NOW).label).toBe('Saved locally');
    expect(describeSyncStatus(status({ lastDriveSyncAt: NOW - 10_000 }), NOW).label).toBe('Synced');
  });

  it('surfaces sync errors', () => {
    const display = describeSyncStatus(status({ state: 'localDirty', error: 'Token expired' }), NOW);
    expect(display.label).toBe('Sync issue');
    expect(display.detail).toBe('Token expired');
    expect(display.primaryAction).toBe('sync');
  });
});

describe('formatRelativeTime', () => {
  it('formats recent timestamps compactly', () => {
    expect(formatRelativeTime(NOW - 2_000, NOW)).toBe('just now');
    expect(formatRelativeTime(NOW - 45_000, NOW)).toBe('45s ago');
    expect(formatRelativeTime(NOW - 5 * 60_000, NOW)).toBe('5m ago');
    expect(formatRelativeTime(NOW - 3 * 60 * 60_000, NOW)).toBe('3h ago');
  });
});
