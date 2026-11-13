// Static test server (TESTING §3.1).
//   node tests/v6/server.mjs --root <tree> [--overlay <dir>] [--port 4401]
// `--overlay` is looked up first (generated files of `build/`), then `--root`.
// Everything is served with `Cache-Control: no-store`, like tests/e2e/server.mjs of 5.18.0.
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const MIME = {
  '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8', '.json': 'application/json; charset=utf-8', '.svg': 'image/svg+xml',
  '.pdf': 'application/pdf', '.apk': 'application/vnd.android.package-archive', '.wasm': 'application/wasm',
  '.png': 'image/png', '.jpg': 'image/jpeg', '.jpeg': 'image/jpeg', '.webp': 'image/webp', '.ico': 'image/x-icon',
  '.woff2': 'font/woff2', '.woff': 'font/woff', '.ttf': 'font/ttf', '.txt': 'text/plain; charset=utf-8',
  '.webmanifest': 'application/manifest+json', '.map': 'application/json'
};

function resolveIn(dir, pathname) {
  if (!dir) return null;
  const candidate = path.resolve(dir, `.${pathname}`);
  if (candidate !== dir && !candidate.startsWith(dir + path.sep)) return null;
  try { return fs.statSync(candidate).isFile() ? candidate : null; } catch { return null; }
}

/** Starts a server. `setTree({root, overlay})` swaps the served tree in place (5.18.0 → v6 upgrade tests). */
export function startServer({ root, overlay = null, port = 0, host = '127.0.0.1' } = {}) {
  const tree = { root: path.resolve(root || '.'), overlay: overlay ? path.resolve(overlay) : null };
  const server = http.createServer((request, response) => {
    let pathname;
    try { pathname = decodeURIComponent(new URL(request.url || '/', 'http://x').pathname); } catch { pathname = '/'; }
    if (pathname.endsWith('/')) pathname += 'index.html';
    const file = resolveIn(tree.overlay, pathname) || resolveIn(tree.root, pathname);
    if (!file) { response.writeHead(404, { 'Content-Type': 'text/plain', 'Cache-Control': 'no-store' }).end('Not found'); return; }
    const headers = { 'Content-Type': MIME[path.extname(file).toLowerCase()] || 'application/octet-stream', 'Cache-Control': 'no-store' };
    if (path.basename(file) === 'service-worker.js') headers['Service-Worker-Allowed'] = '/';
    response.writeHead(200, headers);
    if (request.method === 'HEAD') response.end(); else fs.createReadStream(file).pipe(response);
  });
  return new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(port, host, () => resolve({
      port: server.address().port,
      url: `http://${host}:${server.address().port}`,
      setTree(next) { tree.root = path.resolve(next.root || tree.root); tree.overlay = next.overlay ? path.resolve(next.overlay) : null; },
      close: () => new Promise(done => { server.closeAllConnections?.(); server.close(() => done()); })
    }));
  });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const args = process.argv.slice(2), opt = name => { const i = args.indexOf(`--${name}`); return i < 0 ? undefined : args[i + 1]; };
  const here = path.dirname(fileURLToPath(import.meta.url));
  const server = await startServer({ root: opt('root') || path.resolve(here, '..', '..'), overlay: opt('overlay'), port: Number(opt('port') || process.env.PORT) || 4401 });
  process.stdout.write(`Inhouse test server listening on ${server.port}\n`);
  for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => server.close().then(() => process.exit(0)));
}
