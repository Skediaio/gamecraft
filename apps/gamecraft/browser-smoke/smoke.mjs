#!/usr/bin/env node
// Browser smoke check of the GameCraft window: serves a Trunk build, opens it in headless
// Chromium on WebGPU, waits for frames, holds W for two seconds, and fails on console errors,
// WebGPU validation messages, panics, the page's error card, a uniform picture or a walk that
// changes nothing. Node 22 or later (built-in WebSocket and fetch), no packages. Adapted from
// VisionCraft's apps/visioncraft/browser-smoke/smoke.mjs (MIT).
//
//   node apps/gamecraft/browser-smoke/smoke.mjs --dist apps/gamecraft/dist \
//     [--chrome /usr/bin/chromium-browser] [--out out] [--size 1280x720] [--query "quality=low"]
//
// Writes <out>/window-browser.png (after the first frames) and <out>/window-browser-walked.png
// (after the walk); exits 1 when a check failed.
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';

const args = process.argv.slice(2);
const opt = (name, dflt) => {
  const i = args.indexOf(`--${name}`);
  return i < 0 ? dflt : args[i + 1];
};
const DIST = path.resolve(opt('dist', 'apps/gamecraft/dist'));
const CHROME = opt('chrome', '/usr/bin/chromium-browser');
const OUT = path.resolve(opt('out', 'out'));
const QUERY = opt('query', '');
const [W, H] = opt('size', '1280x720').split('x').map(Number);
const LOAD_TIMEOUT_S = Number(opt('timeout', '240'));
const FRAMES = Number(opt('frames', '60'));
fs.mkdirSync(OUT, { recursive: true });

const sleep = ms => new Promise(r => setTimeout(r, ms));
const t0 = Date.now();
const log = (...m) => console.log(`[${((Date.now() - t0) / 1000).toFixed(1)}s]`, ...m);

// --- static server: the build at / ---
const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm', '.json': 'application/json',
  '.gamecraft': 'application/json' };
const server = http.createServer((req, res) => {
  const url = decodeURIComponent(new URL(req.url, 'http://x').pathname);
  const file = path.join(DIST, url === '/' ? 'index.html' : url);
  if (!file.startsWith(DIST)) { res.writeHead(403).end(); return; }
  fs.stat(file, (err, st) => {
    if (err || !st.isFile()) { res.writeHead(404).end(); return; }
    res.writeHead(200, { 'content-type': TYPES[path.extname(file)] ?? 'application/octet-stream', 'content-length': st.size });
    fs.createReadStream(file).pipe(res);
  });
});
await new Promise(r => server.listen(0, '127.0.0.1', r));
const PAGE = `http://127.0.0.1:${server.address().port}/${QUERY ? `?${QUERY}` : ''}`;

// --- Chromium over the DevTools protocol ---
const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'gamecraft-smoke-'));
const PORT = 9300 + Math.floor(Math.random() * 600);
const chrome = spawn(CHROME, ['--headless=new', '--no-sandbox', `--user-data-dir=${profile}`, `--remote-debugging-port=${PORT}`,
  '--enable-unsafe-webgpu', '--enable-features=Vulkan', '--use-angle=vulkan', '--ignore-gpu-blocklist',
  // A headless page is never in front: keep its animation frames and timers running.
  '--disable-renderer-backgrounding', '--disable-background-timer-throttling', '--disable-backgrounding-occluded-windows',
  `--window-size=${W},${H}`, '--no-first-run', 'about:blank'], { stdio: 'ignore' });
const finish = code => {
  chrome.kill();
  server.close();
  fs.rmSync(profile, { recursive: true, force: true });
  process.exit(code);
};
let tabs;
for (let i = 0; i < 80 && !tabs; i++) {
  try { tabs = await (await fetch(`http://127.0.0.1:${PORT}/json`)).json(); } catch { await sleep(250); }
}
if (!tabs) { log('FAIL Chromium did not start:', CHROME); finish(1); }
const ws = new WebSocket(tabs.find(t => t.type === 'page').webSocketDebuggerUrl);
await new Promise(r => { ws.onopen = r; });
let nextId = 0;
const pending = new Map();
const listeners = [];
ws.onmessage = e => {
  const m = JSON.parse(e.data);
  if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); } else if (m.method) listeners.forEach(l => l(m));
};
const cdp = (method, params = {}) => new Promise((resolve, reject) => {
  pending.set(++nextId, m => (m.error ? reject(new Error(`${method}: ${m.error.message}`)) : resolve(m.result)));
  ws.send(JSON.stringify({ id: nextId, method, params }));
});
const evaluate = async expression => {
  const r = await cdp('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
  if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? r.exceptionDetails.text);
  return r.result.value;
};

// --- what the page says ---
const consoleLines = [];
listeners.push(m => {
  if (m.method === 'Runtime.consoleAPICalled') {
    const text = m.params.args.map(a => a.value ?? a.description ?? '').join(' ');
    consoleLines.push({ level: m.params.type, text });
  } else if (m.method === 'Runtime.exceptionThrown') {
    consoleLines.push({ level: 'exception', text: m.params.exceptionDetails.exception?.description ?? m.params.exceptionDetails.text });
  } else if (m.method === 'Log.entryAdded') {
    consoleLines.push({ level: m.params.entry.level, text: `[${m.params.entry.source}] ${m.params.entry.text}` });
  }
});
// Errors: anything the page logs as an error, a panic, or WebGPU complaining.
const BAD = /panicked|RuntimeError|unreachable|Validation Error|GPUValidationError|GPU error|device was lost|out of memory|OperationError/i;
const isBad = l => l.level === 'error' || l.level === 'exception' || BAD.test(l.text);

await cdp('Runtime.enable');
await cdp('Log.enable');
await cdp('Page.enable');
await cdp('Emulation.setDeviceMetricsOverride', { width: W, height: H, deviceScaleFactor: 1, mobile: false });
await cdp('Emulation.setFocusEmulationEnabled', { enabled: true });
// Frames: each presented frame takes one canvas texture.
await cdp('Page.addScriptToEvaluateOnNewDocument', { source: `
  window.__gcFrames = 0;
  if (window.GPUCanvasContext) {
    const get = GPUCanvasContext.prototype.getCurrentTexture;
    GPUCanvasContext.prototype.getCurrentTexture = function () { window.__gcFrames++; return get.call(this); };
  }` });

const checks = [];
const check = (name, ok, detail = '') => { checks.push({ name, ok, detail }); log(ok ? 'ok  ' : 'FAIL', name, detail); };
const screenshot = async name => {
  const { data } = await cdp('Page.captureScreenshot', { format: 'png' });
  const file = path.join(OUT, name);
  fs.writeFileSync(file, Buffer.from(data, 'base64'));
  log('wrote', file);
  return data;
};
const frames = () => evaluate('window.__gcFrames');

log('opening', PAGE);
await cdp('Page.navigate', { url: PAGE });
let state = { frames: 0, failed: false };
for (let i = 0; i < LOAD_TIMEOUT_S * 4; i++) {
  await sleep(250);
  try {
    state = await evaluate('({ frames: window.__gcFrames, failed: !!window.__gcFailed })');
  } catch { continue; }
  if (state.failed || state.frames >= FRAMES) break;
}
check('the page shows no error card', !state.failed,
  state.failed ? await evaluate("document.getElementById('gc-card').innerText") : '');
check(`at least ${FRAMES} frames were presented`, state.frames >= FRAMES, `${state.frames} frames`);
const title = await evaluate('document.title');
check('the title names the world', title.startsWith('GameCraft: '), JSON.stringify(title));
const first = await screenshot('window-browser.png');

// Walk: hold W for two seconds. No click: the keys work without the pointer lock (the page
// focuses the canvas), and the walk is what is checked here, not the mouse look.
const key = type => cdp('Input.dispatchKeyEvent', { type, key: 'w', code: 'KeyW', windowsVirtualKeyCode: 87, nativeVirtualKeyCode: 87 });
const before = await frames();
await key('keyDown');
await sleep(2000);
await key('keyUp');
await sleep(500);
const walkedFrames = (await frames()) - before;
check('frames kept coming during the walk', walkedFrames > 10, `${walkedFrames} frames in 2.5 s`);
const second = await screenshot('window-browser-walked.png');
check('the walk changed the picture', first !== second);

const bad = consoleLines.filter(isBad);
check('no errors in the console', bad.length === 0, bad.map(l => `${l.level}: ${l.text}`).join(' | ').slice(0, 2000));
log('console:', consoleLines.map(l => `${l.level}: ${l.text}`).join('\n  '));
const failed = checks.filter(c => !c.ok);
log(failed.length ? `${failed.length} checks failed` : 'all checks passed');
finish(failed.length ? 1 : 0);
