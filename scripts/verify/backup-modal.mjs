#!/usr/bin/env node
// Backup modal GUI check (feat/local-mode): engine in local mode with the
// docs/commands.org "Local mode + backup" fixture ingested; chromium on :9333.
// Usage: node scripts/verify/backup-modal.mjs [--port 3131] [--out DIR]
import fs from 'node:fs';
import path from 'node:path';
const argv = process.argv.slice(2);
const opt = (n, d) => { const i = argv.indexOf(`--${n}`); return i >= 0 && i + 1 < argv.length ? argv[i + 1] : d; };
const PORT = Number(opt('port', process.env.PORT ?? '3131')); const CDP = Number(opt('cdp', '9333'));
const APP = `http://localhost:${PORT}/`; const SEARCH = opt('search', process.env.SEARCH ?? 'k:30040'); const OUT = opt('out', '/tmp/backup-shots'); fs.mkdirSync(OUT, { recursive: true });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let page = null; for (let i = 0; i < 40 && !page; i++) { try { const tabs = await (await fetch(`http://localhost:${CDP}/json`)).json(); page = tabs.find((t) => t.type === 'page'); } catch {} if (!page) await sleep(250); }
if (!page) { console.error('FAIL cdp'); process.exit(2); }
const ws = new WebSocket(page.webSocketDebuggerUrl); await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
let seq = 0; const pending = new Map(); ws.onmessage = (m) => { const d = JSON.parse(m.data); if (d.id && pending.has(d.id)) { pending.get(d.id)(d); pending.delete(d.id); } };
const send = (method, params = {}) => new Promise((r) => { const id = ++seq; pending.set(id, r); ws.send(JSON.stringify({ id, method, params })); });
const ev = async (expr) => { const r = await send('Runtime.evaluate', { expression: expr, awaitPromise: true, returnByValue: true }); if (r.result?.exceptionDetails) return { __err: r.result.exceptionDetails.text }; return r.result?.result?.value ?? null; };
const rectOf = async (js) => ev(`(()=>{const e=(${js}); if(!e) return null; e.scrollIntoView({block:'center'}); const r=e.getBoundingClientRect(); if(!r.width||!r.height) return null; return {x:r.x+r.width/2,y:r.y+r.height/2}})()`);
const mouse = (type, x, y, extra = {}) => send('Input.dispatchMouseEvent', { type, x, y, button: 'left', clickCount: 1, ...extra });
const clickAt = async (r) => { if (!r) return false; await mouse('mouseMoved', r.x, r.y, { button: 'none' }); await mouse('mousePressed', r.x, r.y); await mouse('mouseReleased', r.x, r.y); return true; };
const clickJs = async (js) => clickAt(await rectOf(js));
const byText = (sel, re) => `[...document.querySelectorAll(${JSON.stringify(sel)})].find(b=>${re}.test(b.textContent.replace(/\\s+/g,' ').trim()))`;
const count = (sel) => ev(`document.querySelectorAll(${JSON.stringify(sel)}).length`);
const waitFor = async (sel, tries = 30, every = 300) => { for (let i = 0; i < tries; i++) { if ((await count(sel)) > 0) return true; await sleep(every); } return false; };
let shotN = 0; const shot = async (name) => { const r = await send('Page.captureScreenshot', { format: 'png' }); if (r.result?.data) fs.writeFileSync(path.join(OUT, `${String(++shotN).padStart(2, '0')}-${name}.png`), Buffer.from(r.result.data, 'base64')); };
const results = []; const check = (name, ok, detail = '') => { results.push({ name, ok }); console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? ' — ' + detail : ''}`); return ok; };
await send('Runtime.enable'); await send('Page.enable');
await send('Emulation.setDeviceMetricsOverride', { width: 1280, height: 900, deviceScaleFactor: 1, mobile: false });
await send('Page.navigate', { url: APP }); await sleep(3500);
await ev(`(()=>{document.querySelectorAll('.rf-action--ghost, .rf-close, .dt-skip').forEach(b=>b.click());})()`); await sleep(800);
const pill = await ev(`[...document.querySelectorAll('button.pill--btn')].map(b=>b.textContent.trim()).join('|')`);
check('pill-local', /local/.test(pill), pill);
await clickJs(`document.querySelector('.rail--research')`); await sleep(900);
await ev(`(()=>{const i=document.querySelector('.search-panel input, input[placeholder*=exact], .search-input input'); const s=Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set; s.call(i,'k:30040'); i.dispatchEvent(new Event('input',{bubbles:true})); i.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true}));})()`); await sleep(2500);
await waitFor('.result-title'); await clickJs(`document.querySelector('.result-title')`); await sleep(1200);
await clickJs(byText('[role=dialog] button', /Read publication/)); await sleep(3000);
check('reader-open', await waitFor('.pane__body--reader'));
await clickJs(`document.querySelector('button[title^="Tools"]')`); await sleep(700);
await clickJs(byText('button', /back up to relays/)); 
check('modal-open', await waitFor('.bk-modal .bk-list li', 30, 300));
await sleep(500);
const rows = await count('.bk-list .bk-row');
check('rows', rows === 8, `${rows} rows`);
const sel = await ev(`document.querySelector('.bk-bar span').textContent`);
check('deleted-unpicked', /^7 of 8/.test(sel), sel);
check('local-note', (await count('.bk-note')) === 1);
await shot('modal');
// kind facet: uncheck 9802
await clickJs(byText('.bk-facet label', /^9802/)); await sleep(300);
const rows2 = await count('.bk-list .bk-row');
check('kind-facet', rows2 === 6, `${rows2} rows after unticking 9802`);
await clickJs(byText('.bk-bar button', /^none$/)); await sleep(200);
const sel2 = await ev(`document.querySelector('.bk-bar span').textContent`);
check('none', /^0 of 6/.test(sel2), sel2);
const hidden = await ev(`document.querySelector('.bk-foot .bk-hint')?.textContent ?? ''`);
check('hidden-picked-note', /1 selected but hidden/.test(hidden), hidden);
await clickJs(byText('.bk-bar button', /^all$/)); await sleep(200);
const go = await ev(`document.querySelector('.bk-go').textContent.trim()`);
check('send-label', /broadcast 6 events → 3 relays/.test(go), go);
await shot('filtered');
ws.close();
const failed = results.filter((r) => !r.ok).length;
console.log(`\n${results.length - failed}/${results.length} passed · screenshots in ${OUT}`);
process.exit(failed ? 1 : 0);
