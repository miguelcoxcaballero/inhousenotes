// GitHub as the updater sees it (shared updater.js): channel catalogs from raw.githubusercontent.com with the
// api.github.com "contents" fallback, and release APK downloads. Catalogs live in `catalogs` keyed by
// "<owner>/<repo>/<path>"; `setCatalog` builds a valid one from a list of releases.
export const NOTES_REPOSITORY = 'miguelcoxcaballero/inhousenotes';

export class GitHub {
  constructor() { this.reset(); }
  reset() {
    this.catalogs = new Map();   // "<repo>/<path>" → JSON value, or {status} to fail
    this.assets = new Map();     // "/<repo>/releases/download/<tag>/<file>" → Buffer
    this.rawStale = false;       // raw.githubusercontent.com still serves the previous catalog (propagation delay)
    this.previous = new Map();
    this.log = [];
    for (const channel of ['A', 'B']) this.setCatalog(channel, []);
    return this;
  }

  /** A release entry with the fields the updater validates. */
  release({ version, versionCode, repository = NOTES_REPOSITORY, sha256 = 'a'.repeat(64), size = 6815752, changelog = 'Test release.', releasedAt = '2026-10-20T14:36:40+00:00' }) {
    return { version, versionCode, apkUrl: `https://github.com/${repository}/releases/download/android-v${version}/inhouse-notes-release-v${version}.apk`, apkSha256: sha256, apkSizeBytes: size, changelog, releasedAt };
  }
  /** Replaces the catalog of a channel; the first release is `latest`. */
  setCatalog(channel, releases, repository = NOTES_REPOSITORY) {
    const key = `${repository}/updates/${channel}.json`;
    if (this.catalogs.has(key)) this.previous.set(key, this.catalogs.get(key));
    const list = releases.map(r => r.apkUrl ? r : this.release({ ...r, repository }));
    this.catalogs.set(key, { schemaVersion: 1, channel, latest: list[0] || null, releases: list });
  }
  /** Makes a path answer with an HTTP error (`status`) instead of content. */
  fail(path, status = 500, repository = NOTES_REPOSITORY) { this.catalogs.set(`${repository}/${path}`, { __status: status }); }

  handle(request) {
    const url = typeof request.url === 'string' ? new URL(request.url) : request.url, host = url.hostname, parts = url.pathname.split('/').filter(Boolean);
    this.log.push({ method: request.method, host, path: url.pathname });
    const json = (value, status = 200) => ({ status, headers: { 'Content-Type': 'application/json; charset=utf-8' }, body: Buffer.from(JSON.stringify(value)) });
    const missing = () => json({ message: 'Not Found', documentation_url: 'https://docs.github.com/rest' }, 404);
    if (host === 'raw.githubusercontent.com') {
      // /<owner>/<repo>/<ref>/<path>
      const key = `${parts[0]}/${parts[1]}/${parts.slice(3).join('/')}`;
      const value = this.rawStale && this.previous.has(key) ? this.previous.get(key) : this.catalogs.get(key);
      if (!value) return { status: 404, headers: { 'Content-Type': 'text/plain; charset=utf-8' }, body: Buffer.from('404: Not Found') };
      if (value.__status) return { status: value.__status, headers: { 'Content-Type': 'text/plain' }, body: Buffer.alloc(0) };
      return { status: 200, headers: { 'Content-Type': 'text/plain; charset=utf-8' }, body: Buffer.from(JSON.stringify(value, null, 2) + '\n') };
    }
    if (host === 'api.github.com') {
      // /repos/<owner>/<repo>/contents/<path>?ref=main
      if (parts[0] !== 'repos' || parts[3] !== 'contents') return missing();
      const path = parts.slice(4).join('/'), value = this.catalogs.get(`${parts[1]}/${parts[2]}/${path}`);
      if (!value) return missing();
      if (value.__status) return json({ message: 'error' }, value.__status);
      const text = JSON.stringify(value, null, 2) + '\n';
      return json({ type: 'file', encoding: 'base64', name: path.split('/').pop(), path, size: Buffer.byteLength(text), sha: 'mock', content: Buffer.from(text).toString('base64').replace(/(.{60})/g, '$1\n') });
    }
    const asset = this.assets.get(url.pathname);
    if (asset) return { status: 200, headers: { 'Content-Type': 'application/vnd.android.package-archive' }, body: asset };
    return { status: 404, headers: { 'Content-Type': 'text/html; charset=utf-8' }, body: Buffer.from('<!doctype html><title>Mock GitHub</title>') };
  }
}
