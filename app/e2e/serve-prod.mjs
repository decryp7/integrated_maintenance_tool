// Serves the production build (dist/app/browser) with the exact Content-Security-Policy that the
// Tauri webview enforces, read from src-tauri/tauri.conf.json so the two can never drift.
// Every Playwright project tests against it (playwright.config.ts). Usage: node e2e/serve-prod.mjs [port]
import { createReadStream, readFileSync, statSync } from 'node:fs';
import { createServer } from 'node:http';
import { dirname, extname, join, normalize, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const appDir = resolve(here, '..');
const root = join(appDir, 'dist', 'app', 'browser');
const port = Number(process.argv[2] ?? process.env.PORT ?? 4301);

const tauriConf = JSON.parse(readFileSync(join(appDir, 'src-tauri', 'tauri.conf.json'), 'utf8'));
const csp = tauriConf?.app?.security?.csp;
if (typeof csp !== 'string' || !csp.trim()) {
  console.error('tauri.conf.json has no app.security.csp string; refusing to serve without the production CSP.');
  process.exit(1);
}

const TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json',
  '.ico': 'image/x-icon',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.woff': 'font/woff',
  '.woff2': 'font/woff2',
  '.txt': 'text/plain; charset=utf-8',
};

function resolveFile(urlPath) {
  const decoded = decodeURIComponent(urlPath.split('?')[0]);
  const candidate = normalize(join(root, decoded));
  if (candidate !== root && !candidate.startsWith(root + sep)) return null;
  try {
    if (statSync(candidate).isFile()) return candidate;
  } catch {
    /* fall through to the SPA entry point */
  }
  return extname(decoded) ? null : join(root, 'index.html');
}

createServer((req, res) => {
  const file = resolveFile(req.url ?? '/');
  if (!file) {
    res.writeHead(404, { 'Content-Security-Policy': csp }).end('Not found');
    return;
  }
  res.writeHead(200, {
    'Content-Type': TYPES[extname(file)] ?? 'application/octet-stream',
    'Content-Security-Policy': csp,
    'Cache-Control': 'no-store',
  });
  createReadStream(file).pipe(res);
}).listen(port, () => console.log(`prod build with Tauri CSP on http://localhost:${port}\nCSP: ${csp}`));
