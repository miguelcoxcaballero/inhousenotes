// Minimal static server for the v6 tree: `node scripts/serve.mjs --root . --overlay build --port 4300`.
// Files in --overlay (the generated build/ output) win over --root (ARCHITECTURE §3.1). The e2e runner uses
// tests/v6/server.mjs (W1.2, same flags) when it exists and this file otherwise; `createStaticServer` is also
// used by specs that switch the served tree at runtime (14b-sw-upgrade).
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const TYPES = {
  '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8', '.json': 'application/json; charset=utf-8', '.svg': 'image/svg+xml',
  '.wasm': 'application/wasm', '.png': 'image/png', '.ttf': 'font/ttf', '.pdf': 'application/pdf'
};

// `site` is read on every request, so a test may replace `site.roots` (highest priority first) or set
// `site.deny(pathname)` while the server runs.
export function createStaticServer(site) {
  return http.createServer((request, response) => {
    let pathname;
    try { pathname = decodeURIComponent(new URL(request.url || '/', 'http://localhost').pathname); } catch (_) { pathname = '/'; }
    if (pathname.endsWith('/')) pathname += 'index.html';
    const file = site.deny?.(pathname) ? null : site.roots.map(root => path.resolve(root, `.${pathname}`))
      .find((candidate, index) => candidate.startsWith(path.resolve(site.roots[index]) + path.sep)
        && fs.existsSync(candidate) && fs.statSync(candidate).isFile());
    if (!file) { response.writeHead(404, { 'Cache-Control': 'no-store' }).end('Not found'); return; }
    response.writeHead(200, {
      'Content-Type': TYPES[path.extname(file).toLowerCase()] || 'application/octet-stream',
      'Cache-Control': 'no-store'
    });
    fs.createReadStream(file).pipe(response);
  });
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const arg = (name, fallback) => (process.argv.includes(name) ? process.argv[process.argv.indexOf(name) + 1] : fallback);
  const port = Number(arg('--port', process.env.PORT || 4300));
  const roots = [arg('--overlay', ''), arg('--root', '.')].filter(Boolean);
  const server = createStaticServer({ roots });
  server.listen(port, '127.0.0.1', () => process.stdout.write(`Inhouse Notes 6 server on http://127.0.0.1:${port}/ (${roots.join(' over ')})\n`));
  for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => server.close(() => process.exit(0)));
}
