import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';

const html = fs.readFileSync(new URL('../index.html', import.meta.url), 'utf8');
const live = fs.readFileSync(new URL('../live-collaboration-v5.js', import.meta.url), 'utf8');
const update = JSON.parse(fs.readFileSync(new URL('../android-update.json', import.meta.url), 'utf8'));
const notes = fs.readFileSync(new URL('../RELEASE_NOTES_v5.2.1.md', import.meta.url), 'utf8');

assert.match(html, /const APP_VERSION = '5\.2\.1';/);
assert.equal((html.match(/data-app-version/g) || []).length, 3, 'two labels plus one binding are expected');
assert.match(html, /const DB_VERSION = 4;/);
assert.match(html, /const TIMELINE_STORE = 'timeline-history';/);
assert.match(html, /function normalizeTimelineHistory\(/);
assert.match(html, /async function persistTimelineHistory\(/);
assert.match(html, /async function captureTimelineRecoveryPoint\(/);
assert.match(html, /Before restoring \$\{whenStr\}/);
assert.match(html, /backgroundImage: page\.backgroundImage/);
assert.match(html, /calendarPageConfig:/);
assert.match(html, /legacyCoverStrokes:/);

assert.match(html, /const DRIVE_BUILD_IDLE_WINDOW_MS = 180;/);
assert.match(html, /optimizeForUpload: true/);
assert.match(html, /function schedulePreparedDrivePdf\(/);
assert.match(html, /async function prepareDrivePdfBlob\(/);
assert.match(html, /sourceBlob: blob/);
assert.match(html, /function getEmbeddedMetadataWorker\(/);
assert.match(html, /parseEmbeddedMetadataPayload\(encodedData, true\)/);
assert.match(html, /awaitAllPages: false/);
assert.doesNotMatch(
  html.slice(html.indexOf('async function importPDFData'), html.indexOf('async function importPDF(e)')),
  /const fetchRes = await fetch\(pdfData\);\s*const buf = await fetchRes\.arrayBuffer\(\);[\s\S]{0,500}pdfjsLib\.getDocument/,
  'opening must not copy a URL PDF before handing it to PDF.js'
);

assert.match(html, /allowFileDiscovery: false/);
assert.match(html, /const requestedRole = roleSelect\?\.value === 'writer' \? 'writer' : 'reader';/);
assert.match(html, /params\.set\('role', requestedRole\)/);
assert.match(html, /params\.set\('mode', 'edit'\)/);
assert.match(html, /Editor link copied/);
assert.match(html, /function showSharedEditorSignInGate\(/);
assert.match(html, /Sign in to edit/);
assert.doesNotMatch(
  html.slice(html.indexOf("copyBtn.addEventListener('click'"), html.indexOf('// Setup event listeners')),
  /setDriveLinkPermission\(state\.driveFileId, 'reader'\)/,
  'copying an editor link must not silently downgrade it to viewer'
);
assert.match(html, /no sign-in required/);
assert.match(html, /resourcekey/);
assert.match(html, /openPublicDrivePreview\(/);
assert.doesNotMatch(html, /Sign in to open shared document\./);

assert.match(html, /startLiveCollaboration\(\)/);
assert.match(html, /scheduleLiveDocumentBroadcast\(\)/);
assert.match(html, /liveCollabUpdatePeers\(deduped\)/);
assert.match(live, /new BroadcastChannel\(/);
assert.match(live, /RTCPeerConnection/);
assert.match(live, /crypto\.subtle\.encrypt/);
assert.match(live, /ihn_live_key_v1/);
assert.match(live, /comments/);
assert.match(live, /bufferedAmountLowThreshold/);
assert.match(live, /applyRemotePages/);
assert.match(live, /snapshot\.contentHash = timelineSnapshotHash/);
assert.match(live, /ihnLiveBroadcastQueued = true/);
assert.match(live, /if \(ihnLiveBroadcastQueued\) scheduleLiveDocumentBroadcast\(\{ immediate: true \}\)/);
assert.match(live, /envelope\.contentHash === ihnLiveLastHash/);
assert.match(live, /function getLiveCollaborationConnectionInfo\(/);
assert.match(live, /local-network/);
assert.match(live, /isMain:/);

assert.match(html, /const DRIVE_SYNC_KEYWORD = 'IH_SYNC:';/);
assert.match(html, /remoteSyncEnvelope\.contentHash === driveConfirmedContentHash/);
assert.match(html, /const localOnlyStrokes = structuralIdentityChanged \|\| !preserveLocalUnsynced/);
assert.match(html, /const pageHasLocalMerges = preserveLocalUnsynced &&/);
assert.match(html, /Saving the last document to Drive in the background/);
assert.match(html, /await backgroundExitSavePromise/);
assert.match(html, /Finishing the previous Drive save/);
assert.match(html, /This device\$\{overview\.isMain \? ' · Main device'/);
assert.match(html, /Google Drive<\/div>/);
assert.match(html, /connection\?\.transport/);
assert.doesNotMatch(html, />Admin mode</);
assert.match(html, /const PENDING_EXIT_UPLOAD_KEY = 'drive-pending-exit-upload-v1';/);
assert.match(html, /async function persistExitLocalCheckpoint\(/);
assert.match(html, /function resumePendingExitUploadIfNeeded\(/);
assert.match(html, /await localCheckpoint;/);
assert.doesNotMatch(
  html.slice(html.indexOf('const localCheckpoint = persistExitLocalCheckpoint'), html.indexOf('// Clear presence before leaving document')),
  /Promise\.race/,
  'Home must wait for the durable local checkpoint'
);
assert.match(html, /event\.returnValue = '';/);
assert.match(html, /clearPendingExitUpload\(state\.driveFileId/);
assert.match(html, /className = 'public-inline-preview'/);
assert.match(html, /fingerprintPublicPdfBlob/);
assert.match(html, /setInterval\(poll, PUBLIC_DRIVE_API_KEY \? 1800 : 2500\)/);
assert.match(html, /startLiveCollaboration\(\);\s*showStatus\(/);
assert.match(html, /Public document opened/);
const publicOpenStart = html.indexOf('function openPublicDrivePreview');
const publicOpenEnd = html.indexOf('function startPublicSharedPolling', publicOpenStart);
assert.doesNotMatch(
  html.slice(publicOpenStart, publicOpenEnd),
  /getPublicPreviewOverlay\(/,
  'anonymous fallback must remain inside the standard editor UI'
);
assert.match(live, /document\.getElementById\('public-inline-preview'\)/);

const inlineScripts = [...html.matchAll(/<script(?:\s[^>]*)?>([\s\S]*?)<\/script>/gi)]
  .filter(match => match.index > 6500 && match[1].trim())
  .map(match => match[1]);
for (let index = 0; index < inlineScripts.length; index += 1) {
  new vm.Script(inlineScripts[index], { filename: `index-inline-${index + 1}.js` });
}
new vm.Script(live, { filename: 'live-collaboration-v5.js' });
assert.match(live, /actorId: `\$\{ihnGetLivePeerId\(\)\}:\$\{ihnGetLiveTabId\(\)\}`/);
assert.match(live, /let ihnLiveSequence = Date\.now\(\)/);

const timelineStart = html.indexOf('const TIMELINE_SCHEMA_VERSION = 2;');
const timelineEnd = html.indexOf('async function encodeVersionHistoryForKeywords', timelineStart);
assert.ok(timelineStart > 0 && timelineEnd > timelineStart, 'timeline implementation must be extractable');
const timelineContext = vm.createContext({
  console,
  A4_WIDTH: 210,
  A4_HEIGHT: 297,
  state: { versionHistory: [], calendarPageConfig: null, exportName: 'Document' },
  simpleHash(value) {
    let hash = 0;
    for (let index = 0; index < value.length; index += 1) hash = ((hash << 5) - hash + value.charCodeAt(index)) | 0;
    return Math.abs(hash).toString(36).slice(0, 8);
  },
  normalizeTemplateKind: value => value || 'blank',
  normalizePageDimension: (value, fallback) => Number.isFinite(Number(value)) ? Number(value) : fallback,
  getPresenceClientId: () => 'test-device',
  driveUserProfile: null,
  persistTimelineHistory: async () => true,
  ensureAllPagesLoadedForStructureChange: async () => true
});
vm.runInContext(html.slice(timelineStart, timelineEnd), timelineContext, { filename: 'timeline-v5.js' });
const basePage = [{ pageId: 'page-a', strokes: [], images: [], backgroundSource: 'template', pageWidth: 210, pageHeight: 297 }];
assert.notEqual(
  timelineContext.timelineSnapshotHash(basePage, null, 'First name'),
  timelineContext.timelineSnapshotHash(basePage, null, 'Second name'),
  'document name changes must be tracked'
);
const mergedTimeline = timelineContext.mergeVersionHistories([
  { id: 'old-copy', ts: 1, author: { email: 'a@example.com' }, contentHash: 'same', pages: basePage },
  { id: 'milestone-a', ts: 2, author: { email: 'a@example.com' }, contentHash: 'milestone', kind: 'manual', isMilestone: true, pages: basePage }
], [
  { id: 'new-copy', ts: 3, author: { email: 'a@example.com' }, contentHash: 'same', pages: basePage },
  { id: 'milestone-b', ts: 4, author: { email: 'a@example.com' }, contentHash: 'milestone', kind: 'restore', isMilestone: true, pages: basePage }
]);
assert.deepEqual(Array.from(mergedTimeline, entry => entry.id), ['milestone-a', 'new-copy', 'milestone-b']);

assert.equal(update.publishedAppVersion, '5.2.1');
assert.equal(update.version, '1.0.7', 'Android wrapper version is intentionally unchanged');
assert.match(update.releaseNotes, /v5\.2\.1/);
assert.match(notes, /preserves the selected role/i);
assert.match(notes, /real Inhouse Notes editor/i);
assert.match(notes, /OAuth/i);

console.log('v5.2.1 smoke checks passed.');
