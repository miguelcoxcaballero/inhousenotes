// Thin APK host. Shared updater code and release policy live in inhouse-services-updater.
// A + tiny changelog by default. B only on explicit user request. Sync all apps after updater edits.
(async () => {
  const config = await (await fetch('config.json')).json();
  const quiet = new URLSearchParams(location.search).get('quiet') === '1';
  config.close = () => window.InhouseNative?.closeUpdater?.();
  if (quiet) config.onOffer = value => window.InhouseUpdateHost?.offer(JSON.stringify(value));
  const updater = InhouseServicesUpdater.create(config); window.InhouseUpdater = updater;
  let pending = ''; try { pending = window.InhouseNative?.getPendingUpdateManifest?.() || ''; } catch (_) {}
  if (!quiet) {
    window.addEventListener('focus', () => updater.refreshInstallation());
    document.addEventListener('visibilitychange', () => { if (!document.hidden) updater.refreshInstallation(); });
  }
  if (!quiet && updater.refreshInstallation()) return;
  if (pending) { try { if (!updater.offerPending(JSON.parse(pending))) config.close(); } catch (_) { config.close(); } }
  else if (quiet) updater.start();
  else { updater.settings(); if (new URLSearchParams(location.search).get('settings') !== '1') updater.check({ manual: true }); }
})();
