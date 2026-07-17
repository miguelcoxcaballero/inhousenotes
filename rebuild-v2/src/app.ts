// Main application orchestrator.
// Owns the document lifecycle, wires up the renderer, persistence,
// sync, and collaborative editing subsystems.

import type { Doc } from './core/model';
import { createDoc, createPage } from './core/model';
import { serializePage } from './core/serial';
import { DocStore } from './core/store';
import { PersistController } from './persist/persistController';
import { openDb } from './persist/idb';
import { loadDoc, listDocs, deleteDoc, DocRecord } from './persist/docRepo';
import { DocRenderer } from './render/docRenderer';
import { SyncMachine } from './sync/syncMachine';
import { CollabClient } from './sync/collab';
import { GoogleAuth, DriveClient, type DriveFile } from './sync/driveClient';
import { loadDoc as loadDocFromDrive } from './sync/sidecar';
import { Editor } from './editor';
import { SyncStatusBar } from './ui/syncStatusBar';
import { TimelinePanel } from './ui/timelinePanel';
import { ManagePagesPanel } from './ui/managePagesPanel';
import { CalendarPanel } from './ui/calendarPanel';
import { ShareModal } from './ui/shareModal';
import { downloadBlob, exportDocPdf } from './export/pdf';
import { importPdfPages, importPdfUrl, type ImportedPdfPage } from './import/pdfImport';
import { CalendarClient } from './sync/calendarClient';

export class App {
  // Public subsystems — null until a document is open
  renderer: DocRenderer | null = null;
  store: DocStore | null = null;
  persist: PersistController | null = null;
  sync: SyncMachine | null = null;
  collab: CollabClient | null = null;
  editor: Editor | null = null;
  statusBar: SyncStatusBar | null = null;

  /** Auth for Drive operations. Initialized lazily on first openDrive. */
  readonly auth = new GoogleAuth();
  private drive: DriveClient | null = null;

  private db: IDBDatabase | null = null;
  private viewport: HTMLElement | null = null;
  private disposed = false;

  /** Current document ID, null when no document is open. */
  get docId(): string | null {
    return this.store?.doc.id ?? null;
  }

  // ── Lifecycle ──────────────────────────────────────────────────────────────

  /**
   * Open an existing document from IndexedDB.
   * Shows the home screen if docId is not found.
   */
  async open(docId: string): Promise<void> {
    if (this.disposed) return;
    await this.ensureDb();

    const loaded = await loadDoc(this.db!, docId);
    if (!loaded) {
      console.warn(`Document not found: ${docId}`);
      return;
    }

    await this.initWithDoc(loaded.doc);
  }

  /**
   * Open a document from Google Drive by file ID.
   * Loads the doc from Drive, then opens it locally (caching in IDB).
   */
  async openDrive(fileId: string, resourceKey?: string | null): Promise<void> {
    if (this.disposed) return;
    await this.ensureDb();
    await this.ensureDrive();

    const blob = await this.drive!.downloadMedia(fileId, { resourceKey });
    const text = await blob.text();
    const doc = loadDocFromDrive(text);
    if (!doc) {
      console.error(`Failed to load document from Drive: ${fileId}`);
      return;
    }

    await this.initWithDoc(doc, fileId);
  }

  /**
   * Create a new document with the given name.
   * Returns the new document ID.
   */
  async createNew(name: string): Promise<string> {
    if (this.disposed) throw new Error('App disposed');
    await this.ensureDb();

    const doc = createDoc({ name });

    // Save the new document to IDB
    const { saveDocSnapshot } = await import('./persist/docRepo');
    await saveDocSnapshot(this.db!, doc);

    await this.initWithDoc(doc);
    return doc.id;
  }

  async createFromPdf(file: File, name = file.name.replace(/\.pdf$/i, '') || 'PDF'): Promise<string> {
    if (this.disposed) throw new Error('App disposed');
    await this.ensureDb();
    const pages = await importPdfPages(file);
    const doc = createDoc({ name, pages });
    const { saveDocSnapshot } = await import('./persist/docRepo');
    await saveDocSnapshot(this.db!, doc);
    await this.initWithDoc(doc);
    return doc.id;
  }

  /** Clean up all subsystems. Safe to call multiple times. */
  async dispose(): Promise<void> {
    this.disposed = true;
    await this.closeDocument();
    this.db?.close();
    this.db = null;
  }

  private async closeDocument(): Promise<void> {
    await this.collab?.dispose();
    this.collab = null;

    this.sync?.dispose();
    this.sync = null;

    await this.persist?.dispose();
    this.persist = null;

    this.editor?.dispose();
    this.editor = null;

    this.statusBar?.dispose();
    this.statusBar = null;

    this.renderer?.dispose();
    this.renderer = null;

    this.store = null;
    this.viewport = null;
  }

  // ── Document list (home screen) ───────────────────────────────────────────

  /** Fetch all documents from IDB for the home screen. */
  async listDocuments(): Promise<DocRecord[]> {
    await this.ensureDb();
    return listDocs(this.db!);
  }

  /** Delete a document from IDB. */
  async removeDocument(docId: string): Promise<void> {
    await this.ensureDb();
    await deleteDoc(this.db!, docId);
  }

  driveClientId(): string | null {
    return this.auth.getClientId();
  }

  setDriveClientId(clientId: string): void {
    this.auth.setClientId(clientId);
  }

  isDriveSignedIn(): boolean {
    return this.auth.isSignedIn();
  }

  hasDriveSession(): boolean {
    return this.auth.hasStoredTokens();
  }

  async signInDrive(): Promise<void> {
    await this.auth.signIn();
    await this.ensureDrive();
  }

  signOutDrive(): void {
    this.auth.signOut();
  }

  async listDriveDocuments(search = ''): Promise<DriveFile[]> {
    await this.ensureDrive();
    const folder = await this.drive!.getOrCreateAppFolder();
    const result = await this.drive!.listFiles({ folderId: folder.id, mimeType: 'application/json', pageSize: 100 });
    const needle = search.trim().toLowerCase();
    return result.files
      .filter((file) => file.name.endsWith('.ihn.json'))
      .filter((file) => !needle || file.name.toLowerCase().includes(needle))
      .sort((a, b) => String(b.modifiedTime).localeCompare(String(a.modifiedTime)));
  }

  // ── Private helpers ───────────────────────────────────────────────────────

  private async ensureDb(): Promise<void> {
    if (!this.db) {
      this.db = await openDb();
    }
  }

  private async ensureDrive(): Promise<void> {
    if (!this.drive) {
      this.drive = new DriveClient(this.auth, (err) => console.error('Drive error:', err));
    }
  }

  private async initWithDoc(doc: Doc, driveFileId?: string): Promise<void> {
    // Dispose previous document first
    if (this.store) {
      await this.closeDocument();
    }

    // Create store and renderer
    this.store = new DocStore(doc);
    await this.ensureDb();

    if (!this.viewport) {
      this.viewport = this.mountViewport();
    }

    this.renderer = new DocRenderer(this.viewport, this.store);

    // Initialize persistence
    this.persist = new PersistController(this.db!, this.store);
    await this.persist.start();

    // Initialize sync and collab (drive integration)
    await this.ensureDrive();
    if (this.drive) {
      this.sync = new SyncMachine(this.store, this.persist, this.drive, { name: doc.meta.name, fileId: driveFileId });
      this.statusBar = new SyncStatusBar(this.viewport, {
        onSyncNow: () => void this.sync?.start(),
        onAcceptRemote: () => void this.sync?.acceptRemote(),
        onKeepLocal: () => void this.sync?.keepLocal()
      });
      this.statusBar.update(this.sync.status);
      this.sync.onStatusChange((status) => this.statusBar?.update(status));
      this.collab = new CollabClient(this.store, this.sync, this.drive, doc.id, driveFileId, doc.meta.name);
      this.collab.start();
    }

    // Wire editor (gestures + pointer pipeline + tools + document panels)
    const timeline = new TimelinePanel(this.viewport, this.store, this.persist, this.renderer);
    const pages = new ManagePagesPanel(this.viewport, this.store, this.renderer);
    const calendar = new CalendarPanel(this.viewport, this.store, this.renderer, new CalendarClient(this.auth));
    const share = new ShareModal(this.viewport, this.sync);
    this.editor = new Editor(
      this.viewport,
      this.store,
      this.renderer,
      async () => {
        // Overscroll-to-create-page
        const page = createPage();
        this.store!.apply({
          type: 'add-page',
          page: serializePage(page),
          index: this.store!.doc.pageOrder.length
        });
      },
      [
        { id: 'home', label: 'Home', title: 'Back to document list', run: () => this.goHome() },
        { id: 'pages', label: 'Pages', title: 'Manage pages', run: () => pages.open() },
        { id: 'timeline', label: 'Timeline', title: 'Version history', run: () => void timeline.open() },
        { id: 'calendar', label: 'Calendar', title: 'Calendar pages and events', run: () => calendar.open() },
        { id: 'import', label: 'Import', title: 'Import PDF', run: () => void this.importPdf() },
        { id: 'importUrl', label: 'PDF URL', title: 'Import PDF from URL', run: () => void this.importPdfFromUrl() },
        { id: 'export', label: 'Export', title: 'Export PDF', run: () => void this.exportPdf() },
        { id: 'share', label: 'Share', title: 'Share with Drive', run: () => share.open() }
      ]
    );
  }

  private async importPdf(): Promise<void> {
    if (!this.store || !this.renderer) return;
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = 'application/pdf';
    input.addEventListener('change', async () => {
      const file = input.files?.[0];
      if (!file || !this.store || !this.renderer) return;
      try {
        await this.applyImportedPages(await importPdfPages(file));
      } catch (err) {
        window.alert(err instanceof Error ? err.message : String(err));
      }
    });
    input.click();
  }

  private async importPdfFromUrl(): Promise<void> {
    const url = window.prompt('PDF URL:');
    if (!url?.trim() || !this.store || !this.renderer) return;
    try {
      await this.applyImportedPages(await importPdfUrl(url.trim()));
    } catch (err) {
      window.alert(err instanceof Error ? err.message : String(err));
    }
  }

  private async exportPdf(): Promise<void> {
    if (!this.store) return;
    const name = `${this.store.doc.meta.name || 'inhouse-notes'}.pdf`;
    downloadBlob(await exportDocPdf(this.store.doc), name);
  }

  private async applyImportedPages(pages: ImportedPdfPage[]): Promise<void> {
    if (!this.store || !this.renderer) return;
    const serialPages = pages.map(serializePage);
    if (serialPages.length === 0) return;
    if (isEmptyStarterDoc(this.store.doc)) {
      this.store.apply({ type: 'replace-doc', pages: serialPages });
    } else {
      for (const page of serialPages) {
        this.store.apply({ type: 'add-page', page, index: this.store.doc.pageOrder.length });
      }
    }
    this.renderer.rebuild();
  }

  private goHome(): void {
    const url = new URL(window.location.href);
    url.searchParams.delete('docId');
    url.searchParams.delete('fileId');
    window.location.href = url.toString();
  }

  private mountViewport(): HTMLElement {
    let el = document.getElementById('app-viewport');
    if (!el) {
      el = document.createElement('div');
      el.id = 'app-viewport';
      el.style.position = 'absolute';
      el.style.inset = '0';
      el.style.overflow = 'hidden';
      el.style.touchAction = 'none';

      const app = document.getElementById('app');
      if (app) {
        app.appendChild(el);
      } else {
        document.body.appendChild(el);
      }
    }
    return el;
  }
}

function isEmptyStarterDoc(doc: Doc): boolean {
  if (doc.pageOrder.length !== 1) return false;
  const page = doc.pages.get(doc.pageOrder[0]!);
  return !!page && page.strokeOrder.length === 0 && page.imageOrder.length === 0;
}
