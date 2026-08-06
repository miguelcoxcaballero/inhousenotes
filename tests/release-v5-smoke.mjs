import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';

const indexHtml = fs.readFileSync(new URL('../index.html', import.meta.url), 'utf8');
const app = fs.readFileSync(new URL('../app-v5.js', import.meta.url), 'utf8');
const boot = fs.readFileSync(new URL('../boot-v5.js', import.meta.url), 'utf8');
const runtimeCore = fs.readFileSync(new URL('../runtime-core-v5.js', import.meta.url), 'utf8');
const securityCore = fs.readFileSync(new URL('../security-core-v5.js', import.meta.url), 'utf8');
const timelineCore = fs.readFileSync(new URL('../timeline-core-v5.js', import.meta.url), 'utf8');
const html = [indexHtml, app, boot, runtimeCore, securityCore, timelineCore].join('\n');
const live = fs.readFileSync(new URL('../live-collaboration-v5.js', import.meta.url), 'utf8');
const collaborationCore = fs.readFileSync(new URL('../collaboration-core-v5.js', import.meta.url), 'utf8');
const scannerHtml = fs.readFileSync(new URL('../scanner/index.html', import.meta.url), 'utf8');
const scannerStyles = fs.readFileSync(new URL('../scanner/styles.css', import.meta.url), 'utf8');
const scanner = fs.readFileSync(new URL('../scanner/script.js', import.meta.url), 'utf8');
const scannerConfig = fs.readFileSync(new URL('../scanner/configLoader.js', import.meta.url), 'utf8');
const scannerLightweight = fs.readFileSync(new URL('../scanner/processing/lightweight.js', import.meta.url), 'utf8');
const update = JSON.parse(fs.readFileSync(new URL('../android-update.json', import.meta.url), 'utf8'));
const notes = fs.readFileSync(new URL('../RELEASE_NOTES_v5.11.14.md', import.meta.url), 'utf8');
const androidLoader = fs.readFileSync(new URL('../.github/android/app-loader.html', import.meta.url), 'utf8');
const androidBuilder = fs.readFileSync(new URL('../android app/html_to_apk_builder.py', import.meta.url), 'utf8');
const androidBuildScript = fs.readFileSync(new URL('../.github/scripts/build_android_apk.py', import.meta.url), 'utf8');
const androidWorkflow = fs.readFileSync(new URL('../.github/workflows/build-android.yml', import.meta.url), 'utf8');
const pagesWorkflow = fs.readFileSync(new URL('../.github/workflows/deploy-pages.yml', import.meta.url), 'utf8');

assert.match(indexHtml, /const APP_VERSION = '5\.11\.14';/);
assert.match(app, /const APP_VERSION = appVersionMatch\[1\];/);
assert.equal((html.match(/data-app-version/g) || []).length, 3, 'two labels plus one binding are expected');
assert.match(indexHtml, /collaboration-core-v5\.js\?v=5\.11\.14/);
assert.match(indexHtml, /live-collaboration-v5\.js\?v=5\.11\.14/);
assert.match(indexHtml, /app-v5\.js\?v=5\.11\.14/);
assert.match(indexHtml, /timeline-core-v5\.js\?v=5\.11\.14/);
assert.match(indexHtml, /Content-Security-Policy/);
assert.match(indexHtml, /script-src-attr 'none'/);
assert.match(indexHtml, /frame-src 'self'/);
assert.ok(
  indexHtml.indexOf('id="btn-scan-page"') > indexHtml.indexOf('id="btn-set-cover"'),
  'Scan Page must be directly available below Photo'
);
assert.match(indexHtml, /id="scanner-editor-overlay"/);
assert.match(indexHtml, /id="scanner-editor-frame"[^>]*allow="camera"/);
assert.match(app, /function openScannerEditor\(/);
assert.match(app, /function handleScannerBridgeMessage\(/);
assert.match(app, /function insertScannedPagesAt\(/);
assert.match(app, /reindexPagesAfterInsert\(insertAt, state\.pages\.length, scannedPages\.length\)/);
assert.match(app, /noteCollabPageAdded\(page\.pageId, pageIndex\)/);
assert.match(app, /markAllPagesDirty\('full'\);[\s\S]{0,100}scheduleSave\(true\);/);
assert.match(scannerHtml, /window\.__scannerEmbedMode/);
assert.match(scannerHtml, /id="closeEmbedBtn"/);
assert.match(scannerHtml, /id="addToDocumentBtn"/);
assert.match(scannerHtml, /id="downloadEmbedStencilBtn"/);
assert.match(scannerStyles, /html\[data-embed="true"\] #appContainer/);
assert.match(scanner, /postEmbeddedMessage\("ihn-scanner-ready"\)/);
assert.match(scanner, /postEmbeddedMessage\("ihn-scanner-pages"/);
assert.match(scanner, /async function addScannedPagesToDocument\(/);
assert.match(scanner, /await applyStencilToContext\(/);
assert.doesNotMatch(scannerHtml, /opencv\.js|__cvReady|Loading Core/);
assert.match(scannerHtml, /processing\/lightweight\.js\?v=5\.11\.14/);
assert.match(scannerLightweight, /function estimateCalibrationStrip\(/);
assert.match(scannerLightweight, /function detectMarkerGuidedStencil\(/);
assert.match(scannerLightweight, /function traceYellowFrame\(/);
assert.match(scannerLightweight, /function traceLowerStencilRails\(/);
assert.match(scannerLightweight, /const boxTopExpected = 1 - 1 \/ 27/);
assert.match(scannerLightweight, /const boxBottomExpected = 1 - 0\.5 \/ 27/);
assert.match(scannerLightweight, /const ANALYSIS_FAST_MAX = 608/);
assert.match(scannerLightweight, /sideMatchesAnchor/);
assert.match(scannerLightweight, /function mapFramePoint\(/);
assert.match(scannerLightweight, /const useCurvedFrame = !!frame\?\.paths/);
assert.match(scannerLightweight, /method: "marker-guided"/);
assert.match(scanner, /detection\.method === "marker-guided"/);
assert.match(scannerHtml, /id="processAnimationLayer"/);
assert.match(scannerStyles, /\.processing-animation-layer\.active/);
assert.match(scanner, /const PROCESSING_PHASES = Object\.freeze/);
assert.match(scanner, /frame: "Detecting yellow frame"/);
assert.match(scanner, /function createProcessingAnimator\(/);
assert.match(scanner, /drawPreviewTriangle\(pctx, source/);
assert.match(scanner, /scanner-processing-phase/);
assert.match(scanner, /const yellowBox = "#ffea00"/);
assert.match(scanner, /drawYellowBoxCanonical/);
assert.match(scannerLightweight, /function calibrateFromReference/);
assert.match(pagesWorkflow, /name: Deploy GitHub Pages/);
assert.match(pagesWorkflow, /pages: write/);
assert.match(pagesWorkflow, /id-token: write/);
assert.match(pagesWorkflow, /group: pages-production/);
assert.match(pagesWorkflow, /cancel-in-progress: false/);
assert.match(pagesWorkflow, /uses: actions\/configure-pages@v5/);
assert.match(pagesWorkflow, /uses: actions\/upload-pages-artifact@v4/);
assert.match(pagesWorkflow, /uses: actions\/github-script@v8/);
assert.match(pagesWorkflow, /timeout-minutes: 6/);
assert.match(pagesWorkflow, /Date\.now\(\) \+ 5 \* 60 \* 1000/);
assert.match(pagesWorkflow, /the deployment was left active and was not cancelled/);
assert.match(pagesWorkflow, /Retired stale Pages deployment/);
assert.match(pagesWorkflow, /Created fresh Pages deployment/);
assert.doesNotMatch(pagesWorkflow, /Resuming Pages deployment/);
assert.match(pagesWorkflow, /Prepare lean production site/);
assert.match(pagesWorkflow, /--exclude '\*\.apk'/);
assert.match(pagesWorkflow, /path: \$\{\{ runner\.temp \}\}\/pages-site/);
assert.equal((indexHtml.match(/integrity="sha384-/g) || []).length, 3);
assert.match(html, /\.presence-avatar-wrapper\.p2p-connecting::before/);
assert.match(html, /@keyframes presence-peer-connecting/);
assert.match(html, /\.presence-peer-badge/);
assert.match(html, /@media \(prefers-reduced-motion: reduce\)/);
assert.match(html, /const peerConnected = !!connection\?\.directConnected;/);
assert.match(html, /const peerConnecting = !peerConnected && !!\(u\.isOnline \|\| connection\?\.recovering\);/);
assert.match(html, /peerBadge\.innerHTML = getPresencePeerLinkSvg\(\);/);
assert.match(live, /const directConnected = !own[\s\S]{0,140}peer\?\.channel\?\.readyState === 'open';/);
assert.match(live, /const IHN_LIVE_ERASE_FRAME_MS = 18;/);
assert.match(live, /function publishLiveErasePreview\(/);
assert.match(live, /type: 'live-erase'/);
assert.match(live, /function ihnHandleRealtimeErasePacket\(/);
assert.match(live, /applyRemoteLiveErasePreview\(sanitizedPacket\)/);
assert.match(live, /finalBatch: !!options\.final/);
assert.match(live, /function ihnSanitizeLiveStrokeStamp\(/);
assert.match(live, /typeof commitRemoteLiveStroke === 'function'/);
assert.match(live, /hasPendingRemoteLiveStrokeCommits\(\)/);
assert.match(app, /function commitRemoteLiveStroke\(/);
assert.match(app, /function flushRemoteLiveStrokeCommits\(/);
assert.match(app, /function clearRemoteLiveStrokePreviews\(actorId = '', throughSentAt = Infinity, authoritativePages = null\)/);
assert.match(html, /function applyRemoteLiveErasePreview\(/);
assert.match(html, /function clearRemoteLiveErasePreviews\(/);
assert.match(html, /function getRemoteLiveErasePreviewStrokes\(/);
assert.match(html, /for \(const stroke of getRemoteLiveErasePreviewStrokes\(page\)\)/);
assert.match(html, /publishLiveErasePreview\(currentErasePageId, currentEraseGestureId, changeSet\)/);
assert.match(html, /publishLiveErasePreview\(currentErasePageId, currentEraseGestureId, \{\}, \{ final: true \}\)/);
assert.ok(
  html.indexOf('jspdf.umd.min.js') > html.indexOf('</style>'),
  'PDF libraries must not block the first CSS/body paint'
);
assert.match(boot, /documentRef\.documentElement\.dataset\.theme = theme;/);
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
assert.match(androidBuildScript, /ANDROID_VERSION_NAME = "1\.0\.10"/);
assert.match(androidBuildScript, /ANDROID_VERSION_CODE = 11/);
assert.match(androidLoader, /html, body \{ margin: 0; width: 100%; height: 100%;/);
assert.match(androidLoader, /position: fixed;\s*inset: 0;\s*display: grid;\s*place-items: center;/);
assert.match(html, /#toolbar \{[\s\S]{0,500}--toolbar-ink: #1a1a1a;/);
assert.match(html, /#toolbar \{[\s\S]{0,900}color: var\(--toolbar-ink\);/);
assert.match(html, /\.tool-btn \{[\s\S]{0,300}color: var\(--toolbar-ink, var\(--text-primary\)\);/);
assert.match(html, /#toolbar-handle \{[\s\S]{0,400}color: var\(--toolbar-muted-ink\);/);
assert.match(html, /#toolbar \.size-dot \{[\s\S]{0,180}background: var\(--toolbar-ink\);/);
assert.match(html, /#toolbar \.tool-btn\.active \.color-dot \{\s*border-color: var\(--toolbar-ink\);/);
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
assert.match(html, /deferPageWrites: true/);
assert.match(html, /const deferPageWrites = !!options\.deferPageWrites/);
assert.match(html, /const DRIVE_OPEN_CACHE_STORE = 'drive-open-cache';/);
assert.match(html, /function driveOpenCacheMatchesCard\(/);
assert.match(html, /function getPreparedDriveOpenCache\(/);
assert.match(html, /const preparedOpen = getPreparedDriveOpenCache\(file\);/);
assert.match(html, /cachedModified >= cardModified/);
assert.match(html, /cachedAnalysis: cachedOpen\.analysis \|\| null/);
assert.match(html, /const hasCachedFirstPageSize =/);
assert.match(html, /setTimeout\(\(\) => revalidateCachedDriveOpen\(file, cachedOpen, sessionToken\), 0\)/);
assert.match(html, /blob\.inhouseOpenCacheAnalysis =/);
assert.match(html, /pdfResult\.inhouseOpenCacheAnalysis =/);
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
assert.match(live, /IHN_LIVE_RENDEZVOUS_TTL/);
assert.match(live, /function ihnPublishRendezvous\(/);
assert.match(live, /function ihnApplyRendezvousSignal\(/);
assert.match(live, /function ihnIsFastRecoveryReason\(/);
assert.match(live, /function ihnRestoreCachedPeers\(/);
assert.match(live, /remote network rendezvous/);
assert.match(live, /const IHN_LIVE_MAILBOX_PREFIX = 'ihn_lm1_';/);
assert.match(live, /const IHN_LIVE_FAST_MAILBOX_DELAYS = \[0, 70, 150/);
assert.match(live, /function ihnPublishOfferMailbox\(/);
assert.match(live, /function ihnScheduleDirectCommentPoll\(/);
assert.match(live, /async function ihnFetchAndProcessSignalComment\(/);
assert.match(live, /async function ihnConsumeSignalMailboxesFromProperties\(/);
assert.match(live, /function ihnMailboxPropertyFits\(/);
assert.match(html, /ihnConsumeSignalMailboxesFromProperties\(appProperties\)/);
assert.match(live, /const mustConfirmCandidate = hasCachedKey && ihnLiveCryptoKeyNeedsConfirmation/);
assert.match(live, /Android often fires a real Wi-Fi\/AP switch/);
assert.match(live, /ihnProbePeerForFastRecovery\(peerId, peer, reason\)/);
assert.match(live, /async function flushLiveCollaborationBeforeExit\(/);
assert.match(live, /function publishLiveStrokePreview\(/);
assert.match(live, /function ihnHandleRealtimeStrokePacket\(/);
assert.match(live, /IHN_LIVE_STROKE_FRAME_MS = 18/);
assert.match(live, /IHN_LIVE_SUPERVISOR_MS = 250/);
assert.match(live, /IHN_LIVE_SIGNAL_POLL_MS = 2500/);
assert.match(live, /IHN_LIVE_PING_INTERVAL = 500/);
assert.match(live, /IHN_LIVE_ROUTE_STALE_MS = 2500/);
assert.match(live, /IHN_LIVE_ROUTE_PROBE_MAX_MISSES = 2/);
assert.match(live, /IHN_LIVE_NETWORK_PROBE_MS = 220/);
assert.match(live, /Math\.min\(5000, Math\.max\(IHN_LIVE_NETWORK_PROBE_MS/);
assert.match(live, /selectedPair\.currentRoundTripTime/);
assert.match(live, /IHN_LIVE_ICE_GATHER_TIMEOUT = 650/);
assert.match(live, /IHN_LIVE_FAST_ICE_GATHER_TIMEOUT = 280/);
assert.match(live, /IHN_LIVE_ICE_CANDIDATE_SETTLE_MS = 70/);
assert.ok(
  live.indexOf('const iceReady = ihnWaitForIce') < live.indexOf('await pc.setLocalDescription(offerDescription)'),
  'offer ICE listeners must be installed before gathering starts'
);
assert.match(live, /IHN_LIVE_ICE_BATCH_RETRY_LIMIT = 4/);
assert.match(live, /function ihnQueueLocalIceCandidate\(/);
assert.match(live, /async function ihnFlushLocalIceCandidates\(/);
assert.match(live, /async function ihnApplyCandidateSignal\(/);
assert.match(live, /type: 'candidates'/);
assert.match(live, /ihnScheduleRapidSignalPolls\('offer posted'\)/);
assert.match(live, /function ihnProbePeerForFastRecovery\(/);
assert.match(live, /function ihnScheduleRapidSignalPolls\(/);
assert.match(live, /function ihnSendHealthPing\(/);
assert.match(live, /function ihnFlushRealtimeBacklogToPeer\(/);
assert.match(live, /function ihnDocumentIsVisible\(/);
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
assert.match(html, /const PDF_NORMALIZED_METADATA_KEYWORD = 'IH_NORM:1';/);
assert.match(html, /metadataAlreadyNormalized = keywords\.includes/);
assert.match(html, /embeddedStrokes && !metadataAlreadyNormalized/);
assert.match(html, /function getPdfOverlayContentBounds\(/);
assert.match(html, /pdfLibOverlayCache\.set\(i, \{ hash: pageHash, pngBytes, bounds: overlayBounds \}\)/);
assert.match(html, /const templateBackgroundImageCache = new Map\(\);/);
assert.match(html, /function canBuildCurrentPdfWithPdfLib\(/);
assert.ok((html.match(/useObjectStreams: true/g) || []).length >= 2);
assert.doesNotMatch(html, /useObjectStreams: false/);
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
assert.doesNotMatch(html, /Saving the last document to Drive in the background/);
assert.match(html, /#loading-overlay\.home-save-transition/);
assert.match(html, /function showHomeSaveLoading\(/);
const goHomeStart = html.indexOf('const goHome = async () =>');
const goHomeEnd = html.indexOf("driveHomeBtn.addEventListener('click'", goHomeStart);
const goHomeSource = html.slice(goHomeStart, goHomeEnd);
assert.match(goHomeSource, /showHomeSaveLoading\(/);
assert.match(goHomeSource, /const saveCompleted = await backgroundExitSavePromise;[\s\S]{0,700}showDriveHome\(\);/);
assert.match(goHomeSource, /Could not finish saving\. The document is still open/);
assert.doesNotMatch(goHomeSource, /showDriveHome\(\);[\s\S]{0,500}Saving the last document to Drive/);
assert.match(html, /await backgroundExitSavePromise/);
assert.doesNotMatch(html, /Finishing (the )?previous (Drive )?save/i);
const openDriveStart = html.indexOf('async function openDriveFile(file)');
const openDriveEnd = html.indexOf('function updateViewerModeUI()', openDriveStart);
const openDriveSource = html.slice(openDriveStart, openDriveEnd);
assert.match(openDriveSource, /resumeDetachedExitUploadAfterOpen\(detachedExitSaves\)/);
assert.doesNotMatch(openDriveSource, /await lifecycleExitSavePromise|await backgroundExitSavePromise/);
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
assert.match(
  closeFlushSource,
  /if \(!hasPendingLocalSave\(\)\) \{[\s\S]{0,350}scheduleLocalStorageBackup\(payload, \{ immediate: true \}\)/,
  'close handling must not publish metadata while page bodies are still pending'
);
assert.match(goHomeSource, /await Promise\.all\(\[localCheckpoint, driveTokenReady, driveRenameReady\]\)/);
assert.doesNotMatch(goHomeSource, /Promise\.race/, 'Home must wait for the durable local checkpoint');
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

const documentHtml = html.slice(html.indexOf('<!DOCTYPE html>'));
const inlineScriptMatches = [...documentHtml.matchAll(/<script(?:\s[^>]*)?>([\s\S]*?)<\/script>/gi)]
  .filter(match => match[1].trim());
const inlineScripts = inlineScriptMatches.map(match => match[1]);
assert.equal(inlineScripts.length, 1, 'only the non-executable version data block may remain inline');
assert.match(
  inlineScriptMatches[0][0],
  /type="application\/x-inhouse-config"[^>]*id="inhouse-app-version"/,
  'the remaining inline block must be inert version data'
);
for (let index = 0; index < inlineScripts.length; index += 1) {
  new vm.Script(inlineScripts[index], { filename: `index-inline-${index + 1}.js` });
}
new vm.Script(app, { filename: 'app-v5.js' });
new vm.Script(boot, { filename: 'boot-v5.js' });
new vm.Script(runtimeCore, { filename: 'runtime-core-v5.js' });
new vm.Script(securityCore, { filename: 'security-core-v5.js' });
new vm.Script(timelineCore, { filename: 'timeline-core-v5.js' });
new vm.Script(live, { filename: 'live-collaboration-v5.js' });
new vm.Script(scanner, { filename: 'scanner/script.js' });
assert.match(live, /actorId: `\$\{ihnGetLivePeerId\(\)\}:\$\{ihnGetLiveTabId\(\)\}`/);
assert.match(live, /let ihnLiveSequence = Date\.now\(\)/);
new vm.Script(collaborationCore, { filename: 'collaboration-core-v5.js' });
const pdfBoundsStart = html.indexOf('function getPdfOverlayContentBounds');
const pdfBoundsEnd = html.indexOf('function canBuildCurrentPdfWithPdfLib', pdfBoundsStart);
const pdfBoundsContext = vm.createContext({});
vm.runInContext(html.slice(pdfBoundsStart, pdfBoundsEnd), pdfBoundsContext, {
  filename: 'pdf-overlay-bounds-v5.js'
});
const sparseBounds = pdfBoundsContext.getPdfOverlayContentBounds(
  { images: [] },
  [{ tool: 'pen', width: 4, points: [{ x: 100, y: 200 }, { x: 140, y: 240 }] }],
  794,
  1123,
  2
);
assert.ok(sparseBounds.width < 60 && sparseBounds.height < 60, 'sparse overlay must be tightly cropped');
assert.equal(
  pdfBoundsContext.getPdfOverlayContentBounds(
    { images: [] },
    [{ tool: 'eraser-stroke', width: 40, points: [{ x: 10, y: 10 }, { x: 50, y: 50 }] }],
    794,
    1123,
    2
  ),
  null,
  'non-rendered eraser history must not create a transparent PDF overlay'
);
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

const fieldsCodecStart = html.indexOf('const COLLAB_KEYWORD_SECTION_MAX_CHARS');
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

const timelineStart = html.indexOf('const TIMELINE_SCHEMA_VERSION = timelineArchiveCore.ARCHIVE_SCHEMA_VERSION;');
const timelineEnd = html.indexOf('async function encodeVersionHistoryForKeywords', timelineStart);
assert.ok(timelineStart > 0 && timelineEnd > timelineStart, 'timeline implementation must be extractable');
const timelineCoreContext = vm.createContext({ console });
vm.runInContext(timelineCore, timelineCoreContext, { filename: 'timeline-core-v5.js' });
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
  ensureAllPagesLoadedForStructureChange: async () => true,
  timelineArchiveCore: timelineCoreContext.InhouseTimelineCore
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

const storageSaveStart = html.indexOf('async function saveToStorage');
const storageSaveEnd = html.indexOf('async function loadFromStorage', storageSaveStart);
const storageSaveSource = html.slice(storageSaveStart, storageSaveEnd);
const storageEvents = [];
const storageSaveContext = vm.createContext({
  console,
  Date,
  state: { lastSavedAt: null },
  pageDirty: new Set(),
  hasPendingLegacyPageMigration: () => false,
  captureDocumentPersistenceContext: () => ({ session: 1 }),
  isDocumentPersistenceContextCurrent: () => true,
  flushStrokeOpsQueue: async () => { storageEvents.push('ops'); return true; },
  flushDirtyPages: async () => { storageEvents.push('pages'); return true; },
  buildMetaPayload: () => { storageEvents.push('metadata'); return { pages: [] }; },
  saveToIndexedDb: async () => { storageEvents.push('indexeddb'); return true; },
  persistTimelineHistory: () => Promise.resolve(),
  scheduleLocalStorageBackup: () => { storageEvents.push('backup'); return true; },
  updateSaveTimestamp: () => storageEvents.push('timestamp')
});
vm.runInContext(`${storageSaveSource}; this.saveToStorage = saveToStorage;`, storageSaveContext);
const storageSaveResult = await storageSaveContext.saveToStorage();
assert.equal(storageSaveResult.ok, true);
assert.ok(
  storageEvents.indexOf('pages') < storageEvents.indexOf('metadata'),
  'page bodies must commit before their metadata checkpoint'
);

const queueSaveStart = html.indexOf('async function queueSave');
const queueSaveEnd = html.indexOf('function flushOnClose', queueSaveStart);
const queueSaveSource = html.slice(queueSaveStart, queueSaveEnd);
const queueStatuses = [];
const queueSaveContext = vm.createContext({
  console: { error: () => {} },
  state: { isReadOnly: false },
  recordSaveDebug: () => {},
  saveToStorage: async () => { throw new Error('simulated storage failure'); },
  showPendingSaveStatus: () => {},
  showStatus: message => queueStatuses.push(message),
  isDriveSyncPending: () => false,
  hasSmoothInteraction: () => false,
  reconcileSavedIndicator: () => {}
});
vm.runInContext(
  `let saveInProgress = false;
   let saveQueued = false;
   let activeLocalSaveController = null;
   ${queueSaveSource}
   this.queueSave = queueSave;
   this.isSaveInProgress = () => saveInProgress;`,
  queueSaveContext
);
await queueSaveContext.queueSave();
assert.equal(queueSaveContext.isSaveInProgress(), false, 'a rejected save must release the save lock');
assert.ok(queueStatuses.includes('Save failed'));

const pageSaveStart = html.indexOf('async function savePageToIndexedDb');
const pageSaveEnd = html.indexOf('async function loadPageFromIndexedDb', pageSaveStart);
const pageSaveSource = html.slice(pageSaveStart, pageSaveEnd);
let releasePageDatabase;
let pageTransactionStarts = 0;
const pageDatabaseGate = new Promise(resolve => { releasePageDatabase = resolve; });
const pageUnderTest = { pageId: 'page-1', strokes: [], images: [] };
const pageDirty = new Set([0]);
const pageDirtyMode = new Map([[0, 'full']]);
const pageDirtyGeneration = new Map([[0, 1]]);
const pageSaveContext = vm.createContext({
  console,
  state: { pages: [pageUnderTest] },
  pageDirty,
  pageDirtyMode,
  pageDirtyGeneration,
  localPageStructureMutationToken: null,
  remotePageMergeToken: null,
  collabPageOrderMutationInProgress: false,
  collabPageStoreRewritePending: false,
  pageStructureVersion: 1,
  activePageStoreTransactions: new Set(),
  activePageStoreWrites: 0,
  PAGE_STORE: 'pages',
  getDocumentSessionToken: () => 1,
  isDocumentSessionTokenValid: token => token === 1,
  cloneSanitizedPageForStorage: page => ({ ...page }),
  openNotebookDb: () => pageDatabaseGate
});
vm.runInContext(`${pageSaveSource}; this.savePageToIndexedDb = savePageToIndexedDb;`, pageSaveContext);
const pendingPageSave = pageSaveContext.savePageToIndexedDb(0, pageUnderTest);
pageDirtyGeneration.set(0, 2);
releasePageDatabase({ transaction: () => { pageTransactionStarts += 1; throw new Error('must not start'); } });
assert.equal(await pendingPageSave, false, 'an edit made during a page snapshot must keep that page pending');
assert.equal(pageTransactionStarts, 0, 'a stale page snapshot must not start an IndexedDB transaction');
assert.equal(pageDirtyMode.get(0), 'full');

const originalDbStart = html.indexOf('function openOriginalPdfDb');
const originalDbEnd = html.indexOf('async function saveOriginalPdfBytes', originalDbStart);
const originalDbSource = html.slice(originalDbStart, originalDbEnd);
let originalDbOpenCalls = 0;
const originalDbContext = vm.createContext({
  console: { warn: () => {} },
  window: { indexedDB: true },
  indexedDB: { open: () => { originalDbOpenCalls += 1; throw new Error('unavailable'); } },
  clearTimeout,
  setTimeout
});
vm.runInContext(
  `const ORIG_PDF_DB_NAME = 'test';
   const ORIG_PDF_DB_VERSION = 1;
   const ORIG_PDF_STORE = 'originals';
   const DRIVE_OPEN_CACHE_STORE = 'cache';
   const INDEXED_DB_BLOCKED_TIMEOUT_MS = 1;
   let _origPdfDbPromise = null;
   ${originalDbSource}
   this.openOriginalPdfDb = openOriginalPdfDb;`,
  originalDbContext
);
assert.equal(await originalDbContext.openOriginalPdfDb(), null);
assert.equal(await originalDbContext.openOriginalPdfDb(), null);
assert.equal(originalDbOpenCalls, 2, 'a failed IndexedDB open must be retryable');

const folderPickerSource = html.slice(
  html.indexOf('function renderFolderPickerList'),
  html.indexOf('async function confirmFolderPicker')
);
const driveBinSource = html.slice(
  html.indexOf('async function refreshDriveBin'),
  html.indexOf('function toggleBinItemMenu')
);
const scannerListSource = scanner.slice(
  scanner.indexOf('function renderList'),
  scanner.indexOf('function select', scanner.indexOf('function renderList'))
);
assert.doesNotMatch(folderPickerSource, /\$\{folder\.name/);
assert.match(folderPickerSource, /textContent = folder\.name/);
assert.doesNotMatch(driveBinSource, /\$\{file\.name/);
assert.match(driveBinSource, /textContent = file\.name/);
assert.doesNotMatch(scannerListSource, /\$\{p\.name/);
assert.match(scannerListSource, /textContent = p\.name/);
const sidePanelSanitizerStart = html.indexOf('const CALENDAR_ALLOWED_TAGS');
const sidePanelSanitizerEnd = html.indexOf('function byteLengthOfPdfData', sidePanelSanitizerStart);
const sidePanelSanitizerSource = html.slice(sidePanelSanitizerStart, sidePanelSanitizerEnd);
assert.match(sidePanelSanitizerSource, /documentRef\.createElement\('template'\)/);
assert.match(sidePanelSanitizerSource, /CALENDAR_ALLOWED_ATTRIBUTES/);
assert.match(sidePanelSanitizerSource, /element\.removeAttribute\(attribute\.name\)/);
assert.match(sidePanelSanitizerSource, /--sp-event-color/);
assert.doesNotMatch(
  html.slice(html.indexOf('function buildSidePanelElement'), html.indexOf('function refreshPageSidePanelLayout')),
  /body\.innerHTML = panelData\.html/,
  'persisted calendar HTML must be sanitized before it reaches the live DOM'
);
const driveTokenRequestSource = html.slice(
  html.indexOf('function ensureDriveToken'),
  html.indexOf('function signInDrive', html.indexOf('function ensureDriveToken'))
);
assert.match(driveTokenRequestSource, /if \(driveTokenRequestPromise\)/);
assert.match(driveTokenRequestSource, /request\.then\(clearRequest, clearRequest\)/);
assert.match(driveTokenRequestSource, /requestGeneration !== driveTokenRequestGeneration/);
assert.match(html, /async function fetchWithDeadline\(/);
assert.match(html, /fetchWithDeadline\(\s*'https:\/\/oauth2\.googleapis\.com\/token'/);
assert.match(html, /const INDEXED_DB_BLOCKED_TIMEOUT_MS = 1500;/);
assert.match(html, /const INDEXED_DB_OPEN_TIMEOUT_MS = 5000;/);
assert.match(html, /const EMBEDDED_METADATA_WORKER_TIMEOUT_MS = 30000;/);
assert.match(html, /const EMBEDDED_METADATA_MAX_ENCODED_CHARS = 48 \* 1024 \* 1024;/);
assert.match(html, /const EMBEDDED_METADATA_MAX_DECOMPRESSED_BYTES = 96 \* 1024 \* 1024;/);
assert.match(html, /if \(!workerUnavailable\) throw workerErr;/);
assert.match(html, /const stride = Math\.max\(1, Math\.floor\(text\.length \/ 4096\)\);/);
assert.match(html, /documentSessionId \+= 1;\s*embeddedMetadataCache\.clear\(\);/);
assert.match(html, /if \(isDocumentSessionTokenValid\(cacheSessionToken\)\) \{\s*embeddedMetadataCache\.set/);
assert.match(html, /if \(raw\.length > 16384\) return null;/);
assert.match(html, /const COLLAB_KEYWORD_SECTION_MAX_CHARS = 8 \* 1024 \* 1024;/);
assert.match(html, /const CALENDAR_KEYWORD_SECTION_MAX_CHARS = 64 \* 1024;/);
assert.equal(
  (html.match(/calB64\.length > CALENDAR_KEYWORD_SECTION_MAX_CHARS/g) || []).length,
  2,
  'both local-open and remote-merge calendar metadata parsers must be bounded'
);
assert.match(html, /if \(Number\(content\.byteLength\) > maxOriginalBytes\) return null;/);
assert.match(html, /const PDF_ASSEMBLY_WORKER_TIMEOUT_MS = 120000;/);
assert.match(html, /function failPdfAssemblyWorker\(/);
assert.match(html, /let pdfExportInProgress = false;/);
assert.match(html, /Timed out preparing PDF for Android/);
assert.match(html, /if \(inputObjectUrl && state\.activePdfObjectUrl !== inputObjectUrl\) \{\s*try \{ URL\.revokeObjectURL\(inputObjectUrl\);/);
assert.match(html, /const timeoutMs = Number\.isFinite\(options\.timeoutMs\)[\s\S]{0,120}: 20000;/);
assert.match(scanner, /Image decode timed out/);
assert.match(scanner, /Stencil image decode timed out/);
assert.match(scanner, /Canvas encoding failed/);
assert.match(scanner, /Could not apply crop/);
assert.match(scanner, /Scanner PDF export failed:/);
assert.match(scanner, /if \(exportedPages === 0\) throw new Error\("No page could be decoded"\)/);
assert.match(scannerConfig, /setTimeout\(\(\) => controller\.abort\(\), 1500\)/);
const htmlLines = html.split(/\r?\n/);
const unguardedTransactions = [];
htmlLines.forEach((line, index) => {
  if (!/\bdb\.transaction\(/.test(line)) return;
  const nearbySource = htmlLines.slice(Math.max(0, index - 12), index + 1).join('\n');
  if (!/try\s*\{/.test(nearbySource)) unguardedTransactions.push(index + 1);
});
assert.deepEqual(
  unguardedTransactions,
  [],
  'every IndexedDB transaction must degrade to a result instead of rejecting from a closed database'
);
assert.match(html, /return allSaved && pageDirty\.size === 0;/);
const exitCheckpointSource = html.slice(
  html.indexOf('async function persistExitLocalCheckpoint'),
  html.indexOf('function getLifecycleLocalCheckpoint')
);
assert.ok(
  exitCheckpointSource.indexOf('flushLegacyPagesCacheForCheckpoint()')
    < exitCheckpointSource.indexOf('buildMetaPayload(savedAt)'),
  'Home/close checkpoints must persist deferred pages before publishing metadata'
);
assert.match(html, /function hasPendingLegacyPageMigration\(/);
assert.match(html, /if \(!cachedLegacyPage\) \{\s*index \+= 1;/);
assert.match(html, /let legacyPageMigrationPromise = null;/);
assert.match(html, /let legacyPageMigrationCheckpointPromise = null;/);
assert.match(html, /if \(legacyPageMigrationCheckpointPromise\) \{\s*return legacyPageMigrationCheckpointPromise;/);
const remoteMergeCheckpointSource = html.slice(
  html.indexOf("if (changed) {\n                // Publish metadata only after every merged page body is durable."),
  html.indexOf('if (!hasLocalMerges)', html.indexOf("if (changed) {\n                // Publish metadata only after every merged page body is durable."))
);
assert.ok(remoteMergeCheckpointSource.length > 0, 'remote merge durable checkpoint must exist');
assert.ok(
  remoteMergeCheckpointSource.indexOf('flushDirtyPages({')
    < remoteMergeCheckpointSource.indexOf('buildMetaPayload(savedAt)'),
  'remote merges must persist page bodies before publishing their metadata'
);
assert.match(remoteMergeCheckpointSource, /if \(!indexedDbSaved && !backupSaved\)/);

assert.equal(update.publishedAppVersion, '5.11.14');
assert.equal(update.version, '1.0.10');
assert.equal(update.versionCode, 11);
assert.equal(update.apkSizeBytes, 3159597);
assert.match(update.releaseNotes, /v5\.11\.14/);
assert.match(notes, /physical page corners/i);
assert.match(notes, /adaptive precision/i);
assert.match(notes, /four printed colour circles/i);
assert.match(notes, /six supplied photos/i);
assert.match(notes, /15 deterministic/i);

console.log('v5.11.14 smoke checks passed.');
