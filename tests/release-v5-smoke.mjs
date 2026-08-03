import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';

const html = fs.readFileSync(new URL('../index.html', import.meta.url), 'utf8');
const live = fs.readFileSync(new URL('../live-collaboration-v5.js', import.meta.url), 'utf8');
const collaborationCore = fs.readFileSync(new URL('../collaboration-core-v5.js', import.meta.url), 'utf8');
const update = JSON.parse(fs.readFileSync(new URL('../android-update.json', import.meta.url), 'utf8'));
const notes = fs.readFileSync(new URL('../RELEASE_NOTES_v5.8.3.md', import.meta.url), 'utf8');
const androidBuilder = fs.readFileSync(new URL('../android app/html_to_apk_builder.py', import.meta.url), 'utf8');
const androidBuildScript = fs.readFileSync(new URL('../.github/scripts/build_android_apk.py', import.meta.url), 'utf8');
const androidWorkflow = fs.readFileSync(new URL('../.github/workflows/build-android.yml', import.meta.url), 'utf8');

assert.match(html, /const APP_VERSION = '5\.8\.3';/);
assert.equal((html.match(/data-app-version/g) || []).length, 3, 'two labels plus one binding are expected');
assert.match(html, /collaboration-core-v5\.js\?v=5\.8\.3/);
assert.match(html, /live-collaboration-v5\.js\?v=5\.8\.3/);
assert.ok(
  html.indexOf('jspdf.umd.min.js') > html.indexOf('</style>'),
  'PDF libraries must not block the first CSS/body paint'
);
assert.match(html, /document\.documentElement\.dataset\.theme = theme;/);
assert.match(html, /function showFastInitialView\(/);
assert.ok(
  html.indexOf('initDriveAuth();', html.indexOf('async function init()'))
    < html.indexOf('await loadFromStorage();', html.indexOf('async function init()')),
  'Drive loading must begin before hidden editor restoration'
);
const pullRefreshSource = html.slice(
  html.indexOf('function setupMobileHomePullToRefresh'),
  html.indexOf('function isDriveHomeAutoRefreshBlocked')
);
assert.doesNotMatch(pullRefreshSource, /max-width:\s*720px/, 'touch tablets must support pull-to-refresh');
assert.match(pullRefreshSource, /navigator\.maxTouchPoints/);
assert.match(androidBuilder, /def patch_startup_theme\(/);
assert.match(androidBuilder, /webView\.setBackgroundColor\(bootColor\)/);
assert.match(androidBuilder, /local Inhouse Notes launch shell/);
assert.doesNotMatch(
  androidBuilder.slice(androidBuilder.indexOf('def write_node_project'), androidBuilder.indexOf('def patch_manifest')),
  /"url": "https:\/\/inhousenotes\.com/,
  'Android must paint its bundled launch shell before navigating to the web app'
);
assert.match(androidBuildScript, /ANDROID_VERSION_NAME = "1\.0\.9"/);
assert.match(androidBuildScript, /ANDROID_VERSION_CODE = 10/);
assert.doesNotMatch(androidWorkflow, /Publish APK as app v4\.4\.24/);
assert.match(html, /data-update-progressbar/);
assert.match(html, /function formatAndroidUpdateMegabytes\(/);
assert.match(html, /function renderAndroidUpdateProgress\(/);
assert.match(html, /formatAndroidUpdateMegabytes\(downloadedBytes\)/);
assert.match(androidBuilder, /connection\.getContentLengthLong\(\)/);
assert.match(androidBuilder, /notifyAppUpdateProgress\(totalBytes, expectedBytes\)/);
assert.match(androidBuilder, /payload\.put\("downloadedBytes", downloadedBytes\)/);
assert.match(androidBuilder, /payload\.put\("totalBytes", totalBytes\)/);
assert.match(androidBuilder, /payload\.put\("percent", percent\)/);
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
assert.match(html, /class="share-input-group share-invite-controls"/);
assert.match(html, /grid-template-columns: minmax\(0, 1fr\) 112px;/);
assert.match(html, /#btn-send-share \{\s*grid-column: 1 \/ -1;/);
assert.match(html, /class="share-link-icon"/);
assert.doesNotMatch(html, /share-modal-subtitle/);
assert.doesNotMatch(html, /share-section-description/);
assert.doesNotMatch(html, /share-link-access-hint/);
assert.doesNotMatch(html, /share-copy-copy/);
assert.doesNotMatch(html, /function updateLinkAccessHint\(/);
assert.match(html, /const originalContent = copyBtn\.innerHTML;/);
assert.match(html, /copyBtn\.innerHTML = originalContent;/);
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
assert.match(live, /snapshot\.contentHash = ihnCanonicalDocumentHash/);
assert.match(live, /ihnLiveBroadcastQueued = true/);
assert.match(live, /scheduleLiveDocumentBroadcast\(\{ immediate: true, internal: true \}\)/);
assert.match(live, /let ihnLiveLastAppliedHash = '';/);
assert.match(live, /peer\.lastAckedHash === hash/);
assert.match(live, /snapshot-ack/);
assert.match(live, /function ihnSuperviseConnections\(/);
assert.match(live, /IHN_LIVE_CONNECT_TIMEOUT/);
assert.match(live, /ihnComputeReconnectDelay/);
assert.match(live, /startModifiedTime/);
assert.match(live, /pageToken/);
assert.doesNotMatch(live, /for \(let page = 0; page < 3;/);
assert.match(live, /ihnGetAuthoritativeSignalKeyForWrite/);
assert.match(
  live,
  /const canEditDocument = ihnCanEditLiveDocument\(\)[\s\S]{0,700}additiveById: canEditDocument/
);
assert.doesNotMatch(live, /ihnLiveLastHash/);
assert.match(live, /function getLiveCollaborationConnectionInfo\(/);
assert.match(live, /local-network/);
assert.match(live, /isMain:/);
assert.match(live, /function ihnFanOutAppliedEnvelope\(/);
assert.match(live, /function ihnStartLiveApplyDrain\(/);
assert.match(live, /snapshot-received/);
assert.match(live, /IHN_LIVE_APPLY_ACK_TIMEOUT/);
assert.match(live, /function ihnBuildPeerSnapshot\(/);
assert.match(live, /partial: true/);
assert.match(live, /snapshot-nack/);
assert.match(live, /IHN_LIVE_SNAPSHOT_CACHE_LIMIT/);
assert.match(live, /IHN_LIVE_RESUME_GRACE_MS/);
assert.match(live, /iceCandidatePoolSize: 4/);
assert.match(live, /async function flushLiveCollaborationBeforeExit\(/);
assert.match(live, /function publishLiveStrokePreview\(/);
assert.match(live, /function ihnHandleRealtimeStrokePacket\(/);
assert.match(live, /IHN_LIVE_STROKE_FRAME_MS = 18/);
assert.match(live, /IHN_LIVE_SUPERVISOR_MS = 180/);
assert.match(live, /IHN_LIVE_SIGNAL_POLL_MS = 220/);
assert.match(live, /IHN_LIVE_PING_INTERVAL = 250/);
assert.match(live, /IHN_LIVE_ROUTE_STALE_MS = 900/);
assert.match(live, /IHN_LIVE_NETWORK_PROBE_MS = 220/);
assert.match(live, /IHN_LIVE_ICE_GATHER_TIMEOUT = 90/);
assert.match(live, /IHN_LIVE_FAST_ICE_GATHER_TIMEOUT = 35/);
assert.match(live, /IHN_LIVE_ICE_CANDIDATE_SETTLE_MS = 45/);
assert.match(live, /IHN_LIVE_ICE_BATCH_RETRY_LIMIT = 4/);
assert.match(live, /function ihnQueueLocalIceCandidate\(/);
assert.match(live, /async function ihnFlushLocalIceCandidates\(/);
assert.match(live, /async function ihnApplyCandidateSignal\(/);
assert.match(live, /type: 'candidates'/);
assert.match(live, /ihnScheduleRapidSignalPolls\('offer posted'\)/);
assert.match(live, /function ihnProbePeerForFastRecovery\(/);
assert.match(live, /function ihnScheduleRapidSignalPolls\(/);
assert.match(live, /function ihnSendHealthPing\(/);
assert.match(live, /function ihnCheckNetworkRouteSignature\(/);
assert.match(live, /network transport changed/);
assert.match(live, /live route watchdog/);
assert.doesNotMatch(live, /peer\.pc\?\.restartIce\?\.\(\)/);
assert.match(live, /existingPeer\?\.status === 'open' && !offer\.fastRecovery/);
assert.match(live, /Perfect negotiation without a signalling server/);
assert.match(live, /peer online'[\s\S]{0,520}allowReverse: true/);
assert.match(live, /let ihnLiveSignalQueued = false;/);
assert.match(live, /const ihnLiveOffersInFlight = new Set\(\);/);
assert.match(live, /allowReverse: reverseRecovery/);
assert.match(live, /connectionState,/);
assert.match(html, /className = 'remote-live-overlay'/);
assert.match(html, /function applyRemoteLiveStrokePreview\(/);
assert.match(html, /publishLiveStrokePreview\(currentStrokePageId, currentStroke\)/);
assert.match(html, /publishLiveStrokePreview\(targetPage\.pageId, finalizedStroke, \{ final: true \}\)/);

assert.match(html, /const DRIVE_SYNC_KEYWORD = 'IH_SYNC:';/);
assert.match(html, /remoteSyncEnvelope\.contentHash === driveConfirmedContentHash/);
assert.match(html, /const localOnlyStrokes = structuralIdentityChanged \|\| !preserveLocalContent/);
assert.match(html, /const pageHasLocalMerges = preserveLocalContent &&/);
assert.match(html, /const preserveLocalAtApply = preserveLocalUnsynced/);
assert.match(html, /additiveById,/);
assert.match(
  html,
  /function chooseCollaborativeItemLocation[\s\S]{0,700}ihnChooseConcurrentItem/,
  'cross-page locations must resolve by causal item stamp and stable fingerprint'
);
assert.match(
  html,
  /function reconcileDuplicateItemsAcrossPages[\s\S]{0,6500}trackDeletedStrokeIds\(page, causalLosers\)/,
  'cross-page moves must choose one causal winner and tombstone losing locations'
);
assert.match(
  html,
  /const itemLocationResolution = await reconcileDuplicateItemsAcrossPages\(\{[\s\S]{0,700}hasLocalMerges = true;/,
  'remote merges must publish deterministic item-location reconciliation'
);
assert.match(
  html,
  /function removeCalendarImportedImages[\s\S]{0,900}trackDeletedStrokeIds\(page, removed\)/,
  'clearing legacy calendar cards must persist causal tombstones'
);
assert.match(html, /Saving the last document to Drive in the background/);
assert.match(html, /await backgroundExitSavePromise/);
assert.match(html, /Finishing the previous Drive save/);
assert.match(html, /This device\$\{overview\.isMain \? ' · Main device'/);
assert.match(html, /Google Drive<\/div>/);
assert.match(html, /connection\?\.transport/);
assert.doesNotMatch(html, />Admin mode</);
assert.doesNotMatch(html, /save-state-glyph/);
assert.match(html, /\.mode-toggle-status \.save-indicator::after \{\s*content: "";\s*width: 9px;\s*height: 9px;/);
assert.match(html, /function getSaveIndicatorSemanticState\(/);
assert.match(html, /function refreshSaveIndicatorState\(/);
assert.match(html, /Live connection<\/div>/);
assert.match(html, /driveContentVersion !== driveUploadedContentVersion/);
assert.match(html, /const PENDING_EXIT_UPLOAD_KEY = 'drive-pending-exit-upload-v1';/);
assert.match(html, /async function persistExitLocalCheckpoint\(/);
assert.match(html, /function getLifecycleLocalCheckpoint\(/);
assert.match(html, /function resumePendingExitUploadIfNeeded\(/);
assert.match(html, /function startLifecycleExitSave\(/);
assert.match(html, /function pauseCollabPollingForBackground\(/);
assert.match(html, /flushLiveCollaborationBeforeExit\(650\)/);
assert.match(html, /addEventListener\('online',[\s\S]{0,160}resumePendingExitUploadIfNeeded\(\)/);
const closeFlushStart = html.indexOf('function flushOnClose');
const closeFlushEnd = html.indexOf('// \u2500\u2500 \u00a7 4.17', closeFlushStart);
const closeFlushSource = html.slice(closeFlushStart, closeFlushEnd);
assert.match(closeFlushSource, /startLifecycleExitSave\(state\.driveFileId, localCheckpoint\)/);
assert.doesNotMatch(closeFlushSource, /keepalive: true/);
assert.doesNotMatch(closeFlushSource, /if \(state\.isReadOnly\) return/);
assert.match(html, /await localCheckpoint;/);
assert.doesNotMatch(
  html.slice(html.indexOf('const localCheckpoint = persistExitLocalCheckpoint'), html.indexOf('// Clear presence before leaving document')),
  /Promise\.race/,
  'Home must wait for the durable local checkpoint'
);
assert.match(html, /event\.returnValue = '';/);
assert.match(html, /clearPendingExitUpload\(state\.driveFileId/);
assert.match(html, /className = 'public-inline-preview'/);
assert.doesNotMatch(html, /public-inline-status/);
assert.doesNotMatch(html, /Open in Drive/);
assert.doesNotMatch(html, /public-preview-open/);
assert.match(html, /\.public-inline-preview iframe \{\s*position: absolute;\s*top: -52px;/);
assert.match(html, /#canvas-viewport\.public-preview-active \{\s*touch-action: auto !important;/);
assert.match(html, /body\.public-preview-mode \{\s*touch-action: auto;/);
assert.match(html, /frame\.setAttribute\('scrolling', 'yes'\)/);
assert.match(html, /params\.set\('name', metadata\.name/);
assert.match(html, /const sharedDriveName = \(urlParams\.get\('name'\)/);
assert.match(html, /openSharedFileWithoutSignIn\(fileId, resourceKey, \{ requestedRole, fileName \}\)/);
assert.match(html, /if \(!state\.isReadOnly && drawingTools\.includes\(state\.currentTool\) && shouldAcceptDrawInput\(e\)\)/);
assert.match(html, /function installPublicPreviewGestures\(/);
assert.match(html, /function setPublicPreviewZoom\(/);
assert.match(html, /publicPreviewPointers\.size >= 2/);
assert.match(html, /className = 'public-preview-stage'/);
assert.match(html, /className = 'public-preview-controls'/);
assert.match(html, /data-public-preview-action="fit"/);
assert.match(html, /event\.ctrlKey \|\| event\.metaKey/);
assert.match(html, /message\.append\(strong, document\.createTextNode\(' Sign in to edit and save\.'\)\)/);
assert.match(html, /editButton\.textContent = 'Sign in';/);
assert.match(html, /background: color-mix\(in srgb, var\(--accent-yellow\) 18%, var\(--bg-primary\)\)/);
assert.match(html, /public-read-only-mode/);
assert.match(html, /modeToggleLabel\.textContent = 'Read only';/);
assert.match(html, /isViewer && !isAnonymousPublicViewer/);
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
new vm.Script(collaborationCore, { filename: 'collaboration-core-v5.js' });
assert.match(collaborationCore, /function ihnMergeStructureMeta\(/);
assert.match(collaborationCore, /function ihnMergeFieldMeta\(/);
assert.match(collaborationCore, /function ihnFieldMetaHash\(/);
assert.match(collaborationCore, /function ihnNormalizeDeletionStamps\(/);
assert.match(collaborationCore, /function ihnMergeDeletionStamps\(/);
assert.match(collaborationCore, /function ihnDeletionWinsItem\(/);
assert.match(collaborationCore, /function ihnCanonicalDocumentHash\(/);
assert.match(collaborationCore, /function ihnComputeReconnectDelay\(/);
assert.match(html, /IH_STRUCT:/);
assert.match(html, /IH_FIELDS:/);
assert.equal(
  (html.match(/IH_FIELDS:\$\{encodedFields\}/g) || []).length,
  2,
  'both PDF creation paths must persist scalar collaboration fields'
);
assert.match(html, /remoteFields: remoteCollabFields/);
assert.match(html, /getCollabFieldSnapshot\(\)/);
assert.match(html, /persistPendingCollabPageStoreRewrite/);
assert.match(html, /db\.transaction\(\[PAGE_STORE, OPS_STORE, DB_STORE\], 'readwrite'\)/);
assert.match(html, /collabPageOrderMutationInProgress \|\| collabPageStoreRewritePending/);
assert.match(html, /generateLegacyDocumentPageId\(idbCacheKey, i\)/);
assert.match(html, /function generateLegacyDocumentItemId\(/);
assert.match(html, /deletedStrokeStamps:/);
assert.match(html, /ihnDeletionWinsItem\(mergedDeletionStamps, candidate\)/);
assert.match(html, /function acquireLocalPageStructureMutation\(/);
assert.match(html, /function acquireRemotePageMerge\(/);
assert.match(html, /releaseRemotePageMerge\(mergeToken\)/);
assert.match(html, /reconcileConcurrentCalendarPanels\(\{/);
assert.match(html, /const activePageStoreTransactions = new Set\(\);/);
assert.match(html, /let strokeOpsFlushPromise = null;/);
assert.match(html, /let activeStrokeOpsTransaction = null;/);
assert.match(html, /localPageStructureMutationToken === structureToken/);
assert.match(html, /remotePageMergeToken === structureToken/);
assert.match(html, /activePageStoreTransactions\.forEach\(tx =>/);
assert.match(html, /activeStrokeOpsTransactions\.forEach\(tx =>[\s\S]{0,160}tx\.abort\(\)/);
assert.match(
  html,
  /async function flushStrokeOpsQueue[\s\S]{0,2200}while \(strokeOpsFlushPromise\)[\s\S]{0,2200}strokeOpsFlushPromise = flushPromise/,
  'stroke-op persistence callers must coalesce onto one durable transaction'
);
assert.match(
  html,
  /async function appendStrokeOpsToIndexedDb[\s\S]{0,1800}structureVersion !== pageStructureVersion[\s\S]{0,1800}tx\.onabort = \(\) => finish\(false\)/,
  'stroke-op writes must reject a stale page mapping and remain abortable on document switches'
);
assert.ok(
  html.indexOf('loadDriveSession();', html.indexOf('async function init()'))
    < html.indexOf('await loadFromStorage();', html.indexOf('async function init()')),
  'the Drive file ID must still be restored before local migration'
);
assert.match(html, /function resolveHistoryPageIndex\(/);
assert.match(html, /function removeHistoryItems\(/);
assert.match(html, /fromPageId: fromPage\.pageId/);
assert.match(html, /strokeIds: movedStrokes\.map/);
assert.match(html, /pageIdToDelete = String\(state\.pages\[pageIndex\]\?\.pageId/);
assert.match(html, /pageStructureDragInProgress = true/);
assert.match(html, /invalidatePageStructureAsyncState\(\);\s*\/\/ Clear per-page IDB/);
assert.match(html, /importedPages\.forEach\(\(page, index\) => \{/);
assert.match(html, /strokeCollabFingerprint\(/);
assert.match(html, /noteCollabPageMoved\(movedPage\?\.pageId, toIndex\)/);
assert.match(html, /noteCollabPageDeleted\(removedPage\?\.pageId\)/);
assert.match(html, /remoteStructure: remoteCollabStructure/);

const restoreTombstoneStart = html.indexOf('function addRestoreTombstones');
const restoreTombstoneEnd = html.indexOf('async function restoreVersion', restoreTombstoneStart);
assert.ok(
  restoreTombstoneStart > 0 && restoreTombstoneEnd > restoreTombstoneStart,
  'restore tombstone reconciliation must be extractable'
);
const restoreTombstoneSource = html.slice(restoreTombstoneStart, restoreTombstoneEnd);
assert.match(restoreTombstoneSource, /const currentPageId = String\(currentPage\?\.pageId \|\| ''\);/);
assert.match(
  restoreTombstoneSource,
  /const targetPage = currentPageId\s*\? \(targetByPageId\.get\(currentPageId\) \|\| null\)\s*: \(restoredPages\[pageIndex\] \|\| null\);/,
  'restore tombstones may use an index fallback only for legacy pages without a pageId'
);
assert.doesNotMatch(
  restoreTombstoneSource,
  /targetByPageId\.get\(currentPage\.pageId\)\)\s*\|\|\s*restoredPages\[pageIndex\]/,
  'identified pages must never fall through to a different page at the same index'
);

const timelineAdoptionStart = html.indexOf('async function adoptVersionHistory');
const timelineAdoptionEnd = html.indexOf('function sanitizePageForStorage', timelineAdoptionStart);
assert.ok(
  timelineAdoptionStart > 0 && timelineAdoptionEnd > timelineAdoptionStart,
  'timeline adoption must be extractable'
);
const timelineAdoptionSource = html.slice(timelineAdoptionStart, timelineAdoptionEnd);
assert.match(timelineAdoptionSource, /const documentKey = options\.documentKey \|\| getTimelineDocumentKey\(\);/);
assert.match(timelineAdoptionSource, /const sessionToken = Number\.isFinite\(options\.sessionToken\)/);
assert.match(timelineAdoptionSource, /loadTimelineFromIndexedDb\(documentKey\)/);
assert.match(timelineAdoptionSource, /saveTimelineToIndexedDb\(mergedHistory, documentKey\)/);
assert.ok(
  (timelineAdoptionSource.match(/if \(!isCurrentDocument\(\)\)/g) || []).length >= 2,
  'timeline adoption must revalidate its captured document after every await'
);
assert.doesNotMatch(
  timelineAdoptionSource,
  /await[\s\S]{0,180}getTimelineDocumentKey\(\)/,
  'timeline adoption must not recalculate its persistence key after an await'
);

const remotePageApplyStart = html.indexOf('async function applyRemotePages');
const remotePageApplyEnd = html.indexOf('function setsEqual', remotePageApplyStart);
assert.ok(
  remotePageApplyStart > 0 && remotePageApplyEnd > remotePageApplyStart,
  'remote page reconciliation must be extractable'
);
const remotePageApplySource = html.slice(remotePageApplyStart, remotePageApplyEnd);
assert.match(remotePageApplySource, /const activePageIdBeforeReorder = String\(/);
assert.match(
  remotePageApplySource,
  /page => String\(page\?\.pageId \|\| ''\) === activePageIdBeforeReorder/,
  'remote reorder must find the active page by stable pageId'
);
assert.match(
  remotePageApplySource,
  /state\.activePageIndex = remappedActivePageIndex >= 0/,
  'remote reorder must update the active index after remapping'
);
assert.match(
  remotePageApplySource,
  /remotePageStructureApplyInProgress = true;[\s\S]{0,1800}ensureAllPagesLoadedForStructureChange\(\{[\s\S]{0,300}assertContext: assertRemoteMergeContext[\s\S]{0,900}flushStrokeOpsQueue\(\{\s*structureToken: mergeToken/,
  'remote structure reconciliation must block new edits before checkpointing the old numeric mapping'
);
assert.match(
  remotePageApplySource,
  /const currentMergedFieldMetadata = ihnMergeFieldMeta\([\s\S]{0,900}state\.collabFields = currentMergedFieldMetadata/,
  'page fields must be re-merged after asynchronous page hydration'
);

const deferredPullStart = html.indexOf('async function flushDeferredRemotePull');
const deferredPullEnd = html.indexOf('function isRemoteMetaNewerThanTracked', deferredPullStart);
assert.ok(deferredPullStart > 0 && deferredPullEnd > deferredPullStart, 'deferred Drive pull must be extractable');
const deferredPullSource = html.slice(deferredPullStart, deferredPullEnd);
assert.match(deferredPullSource, /const pendingGeneration = deferredRemotePullGeneration;/);
assert.match(deferredPullSource, /if \(!result \|\| result\.error \|\| result\.deferred \|\| result\.busy\)/);
assert.match(
  deferredPullSource,
  /if \(deferredRemotePullGeneration === pendingGeneration\) \{\s*deferredRemotePullPending = false;/,
  'only the exact successfully applied Drive notification may be retired'
);

const fieldsCodecStart = html.indexOf('function encodeCollabFieldsForKeywords');
const fieldsCodecEnd = html.indexOf('// ── § 4.4', fieldsCodecStart);
assert.ok(fieldsCodecStart > 0 && fieldsCodecEnd > fieldsCodecStart, 'IH_FIELDS codec must be extractable');
const sampleFields = {
  v: 1,
  clock: 42,
  fields: {
    'doc:exportName': { value: 'Diseño · 東京', stamp: { clock: 41, actor: 'peer-a' } },
    'page:p1:sidePanel': { value: null, stamp: { clock: 42, actor: 'peer-b' } }
  }
};
const fieldsCodecContext = vm.createContext({
  console,
  btoa,
  atob,
  escape,
  unescape,
  encodeURIComponent,
  decodeURIComponent,
  getCollabFieldSnapshot: () => sampleFields,
  ihnNormalizeFieldMeta: value => value
});
vm.runInContext(
  html.slice(fieldsCodecStart, fieldsCodecEnd),
  fieldsCodecContext,
  { filename: 'collaboration-fields-codec.js' }
);
const encodedFields = fieldsCodecContext.encodeCollabFieldsForKeywords();
const decodedFields = fieldsCodecContext.decodeCollabFieldsFromKeywords(
  `prefix;IH_FIELDS:${encodedFields};IH_SYNC:later-metadata`
);
assert.equal(JSON.stringify(decodedFields), JSON.stringify(sampleFields), 'IH_FIELDS must round-trip Unicode and null values');

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
  ihnStableStringify(value) {
    const normalize = current => {
      if (Array.isArray(current)) return current.map(normalize);
      if (!current || typeof current !== 'object') return current;
      return Object.fromEntries(
        Object.keys(current).sort().map(key => [key, normalize(current[key])])
      );
    };
    return JSON.stringify(normalize(value));
  },
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
const concurrentTimelineLeft = [{
  id: 'shared-coalesce-id',
  ts: 10,
  author: { email: 'same@example.com', name: 'Same account' },
  contentHash: 'left-content',
  deviceId: 'device-a',
  pages: basePage
}];
const concurrentTimelineRight = [{
  id: 'shared-coalesce-id',
  ts: 10,
  author: { email: 'same@example.com', name: 'Same account' },
  contentHash: 'right-content',
  deviceId: 'device-b',
  pages: [{ ...basePage[0], pageWidth: 211 }]
}];
const concurrentTimelineAB = timelineContext.mergeVersionHistories(
  concurrentTimelineLeft,
  concurrentTimelineRight
);
const concurrentTimelineBA = timelineContext.mergeVersionHistories(
  concurrentTimelineRight,
  concurrentTimelineLeft
);
assert.equal(concurrentTimelineAB.length, 2, 'same-account devices must keep both concurrent timeline branches');
assert.equal(
  JSON.stringify(concurrentTimelineAB),
  JSON.stringify(concurrentTimelineBA),
  'timeline ID collisions must resolve independently of merge arrival order'
);
const convergedSameContentTimeline = [{
  id: 'same-origin',
  ts: 20,
  author: { email: 'same@example.com', name: 'Same account' },
  contentHash: 'converged-content',
  deviceId: 'device-a',
  pages: basePage
}, {
  id: 'same-origin',
  ts: 20,
  author: { email: 'same@example.com', name: 'Same account' },
  contentHash: 'converged-content',
  deviceId: 'device-b',
  pages: basePage
}];
const convergedTimelineOnce = timelineContext.pruneTimelineHistory(convergedSameContentTimeline);
const convergedTimelineTwice = timelineContext.pruneTimelineHistory(convergedTimelineOnce);
assert.equal(convergedTimelineOnce.length, 1, 'equivalent device branches should collapse once converged');
assert.equal(
  JSON.stringify(convergedTimelineOnce),
  JSON.stringify(convergedTimelineTwice),
  'timeline normalization must remain idempotent after logical deduplication'
);

const localStructureAcquireStart = html.indexOf('async function acquireLocalPageStructureMutation');
const localStructureAcquireEnd = html.indexOf('function assertLocalPageStructureContext', localStructureAcquireStart);
const localStructureAcquireSource = html.slice(localStructureAcquireStart, localStructureAcquireEnd);
assert.match(localStructureAcquireSource, /expectedContext = captureLocalPageStructureDocumentContext\(\)/);
assert.match(localStructureAcquireSource, /isLocalPageStructureDocumentContextCurrent\(expectedContext\)/);
assert.match(html, /beginDocumentSession\(\{ preserveLocalStructureToken: structureToken \}\)/);

assert.match(
  remotePageApplySource,
  /materializableFallbackId[\s\S]{0,500}structureResolution\.recoveredFallbackPageId = materializableFallbackId/,
  'an all-deleted fallback must select a page body that can actually be materialized'
);

const driveMetaStart = html.indexOf('function normalizeDriveVersion');
const driveMetaEnd = html.indexOf('function isDriveHydratedForCurrentFile', driveMetaStart);
const driveMetaSource = html.slice(driveMetaStart, driveMetaEnd);
assert.match(driveMetaSource, /compareDriveVersions/);
assert.match(driveMetaSource, /revisions\/\$\{encodeURIComponent\(revisionId\)\}/);
assert.match(driveMetaSource, /revisionUrl\.searchParams\.set\('alt', 'media'\)/);
assert.match(driveMetaSource, /driveExactRevisionUnavailableFiles\.add\(fileId\)/);
assert.match(driveMetaSource, /headRevisionId,modifiedTime,version,md5Checksum,size/);

const md5Start = html.indexOf('const MD5_SHIFT_AMOUNTS');
const md5End = html.indexOf('function getPublicDriveResourceHeaders', md5Start);
const md5Context = vm.createContext({ Blob, Uint8Array, Uint32Array });
vm.runInContext(
  `${html.slice(md5Start, md5End)}; this.md5BlobHex = md5BlobHex;`,
  md5Context
);
assert.equal(
  await md5Context.md5BlobHex(new Blob(['abc'])),
  '900150983cd24fb0d6963f7d28e17f72',
  'the incremental Drive checksum must match the MD5 standard vector'
);

const pullStart = html.indexOf('async function pullRemoteChanges');
const pullEnd = html.indexOf('async function updateLocalRevisionId', pullStart);
const pullSource = html.slice(pullStart, pullEnd);
assert.match(pullSource, /const metaAtApplyBoundary = await fetchDriveRevisionMeta\(fileIdAtStart\)/);
assert.match(pullSource, /metaAfterApply = await fetchDriveRevisionMeta\(fileIdAtStart\)/);
assert.match(pullSource, /if \(!driveRevisionMetaMatches\(metaData, metaAfterApply\)/);
assert.match(pullSource, /if \(!backgroundRendered\)/);
assert.match(pullSource, /documentKey: `drive:\$\{fileIdAtStart\}`/);

const exportFetchStart = html.indexOf('async function fetchLatestDrivePdfForExport');
const exportFetchEnd = html.indexOf('async function exportDrivePdf', exportFetchStart);
const exportFetchSource = html.slice(exportFetchStart, exportFetchEnd);
assert.doesNotMatch(
  exportFetchSource,
  /applyDriveRemoteMeta/,
  'an export observation must never advance the applied Drive content baseline'
);
assert.match(exportFetchSource, /fetchStableDrivePdfSnapshot/);
assert.match(html, /decodeCollabStructureFromKeywords\(keywords, pageIds = \[\], options = \{\}\)/);
assert.match(html, /if \(options\.throwOnInvalid\) throw error;/);
assert.match(
  html,
  /page\.sidePanel\.title = titleEl\.textContent;[\s\S]{0,180}markPageDirty\(pageIndex, 'full'\);/,
  'side-panel title edits must be durable in PAGE_STORE'
);

assert.equal(update.publishedAppVersion, '5.8.3');
assert.equal(update.version, '1.0.9');
assert.equal(update.versionCode, 10);
assert.equal(update.apkSizeBytes, 3159541);
assert.match(update.releaseNotes, /v5\.8\.3/);
assert.match(notes, /percentage/i);
assert.match(notes, /APK size in MB/i);
assert.match(notes, /real byte counts/i);

console.log('v5.8.3 smoke checks passed.');
