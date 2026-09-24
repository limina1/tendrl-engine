#!/usr/bin/env node
// Step 3 GUI check: the request card's "relink N terms ▸" → RelinkModal (diff
// rows, tinted refs, event preview, armed sign) → the reader shows the refs
// and their hover card. Assumes a scratch engine seeded by
// vocab-request-seed.sh + vocab-definitions.sh (2 definitions on §1, NOT yet
// relinked), chromium on :9333.
// Usage: node scripts/verify/vocab-relink.mjs --root <30040 coord> --search 'id:<index id>' [--out DIR]
import fs from 'node:fs';
import path from 'node:path';
const argv = process.argv.slice(2);
const opt = (n, d) => { const i = argv.indexOf(`--${n}`); return i >= 0 && i + 1 < argv.length ? argv[i + 1] : d; };
const PORT = Number(opt('port', process.env.PORT ?? '3131')); const CDP = Number(opt('cdp', '9333'));
const APP = `http://localhost:${PORT}/`; const SEARCH = opt('search', process.env.SEARCH ?? 'k:30040');
const OUT = opt('out', '/tmp/vocab-relink-shots'); fs.mkdirSync(OUT, { recursive: true });
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
await ev(`(()=>{const m=document.querySelector('.nm-modal'); if(m){const c=[...m.querySelectorAll('button')].find(b=>/^confirm/i.test(b.textContent.trim())); if(c) c.click();} document.querySelectorAll('.rf-action--ghost, .rf-close, .dt-skip').forEach(b=>b.click());})()`); await sleep(800);
await clickJs(`document.querySelector('.rail--research')`); await sleep(900);
await ev(`(()=>{const i=document.querySelector('.search-panel input, input[placeholder*=exact], .search-input input'); const s=Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set; s.call(i,${JSON.stringify(SEARCH)}); i.dispatchEvent(new Event('input',{bubbles:true})); i.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true}));})()`); await sleep(2500);
await waitFor('.result-title'); await clickJs(`document.querySelector('.result-title')`); await sleep(1200);
await clickJs(byText('[role=dialog] button', /Read publication/)); await sleep(3500);
check('reader-open', await waitFor('.pane__body--reader'));
await clickJs(byText('.seg button, button', /^scroll$/)); await sleep(1500);
await clickJs(`document.querySelector('button[title^="Tools"]')`); await sleep(700);
await clickJs(byText('.tools-sheet__row button, button', /^show( annotations)?$/i)); await sleep(2500);
const refsBefore = await count('.pane__body--reader .section-content .nd-ref');
// the request card's relink button
const relinkBtn = byText('button', /^relink \d+ terms? ▸$/);
const opened = await clickJs(relinkBtn); await sleep(2500);
check('relink-button', opened && (await waitFor('.rl-modal', 10, 300)), 'request card → modal');
const diffs = await count('.rl-modal .rl-diff');
const refSpans = await count('.rl-modal .rl-ref');
const plus = await count('.rl-modal .rl-plus');
check('relink-diff', diffs >= 2 && refSpans >= 2, `${diffs} diff blocks (§1 + §3; §1's heading is protected) · ${refSpans} tinted refs · ${plus} + rows`);
const tagsRow = await ev(`[...document.querySelectorAll('.rl-modal .rl-diff *')].some(e=>/\\["a"|tags\\s*\\+/.test(e.textContent))`);
check('relink-tags-row', tagsRow === true, 'a “tags + [a, …]” row');
const wire = await ev(`document.querySelector('.rl-modal .rl-wire__pre')?.textContent ?? ''`);
check('relink-event-preview', /30041/.test(wire) && /\ba\b/.test(wire), `${wire.length} chars`);
const checks = await count('.rl-modal .rl-check input[type=checkbox], .rl-modal input[type=checkbox]');
check('relink-glossary-checkboxes', checks >= 2, `${checks} per-definition checkboxes`);
await shot('relink-modal');
const armBtn = byText('.rl-modal button', /sign · keep local|sure\? sign/);
await clickJs(armBtn); await sleep(500);
const armed = await ev(`${armBtn}?.textContent.trim() ?? ''`);
check('relink-armed', /sure\? sign \d+ section/.test(armed), `"${armed}"`);
await clickJs(armBtn); await sleep(4500);
check('relink-signed', !(await ev(`!!document.querySelector('.rl-modal')`)), 'modal closed after sign');
await sleep(2500);
const refsAfter = await count('.pane__body--reader .section-content .nd-ref');
check('refs-in-reader', refsAfter >= refsBefore + 2, `${refsBefore} → ${refsAfter} .nd-ref links in the text`);
await shot('relinked');
// hover the first ref → the resolve card shows the definition's title and first sentence
const r0 = await rectOf(`document.querySelector('.pane__body--reader .section-content .nd-ref')`);
if (r0) { await mouse('mouseMoved', r0.x, r0.y, { button: 'none' }); await sleep(1200); }
const pop = await ev(`(document.querySelector('.nd-preview')?.textContent ?? '').replace(/\\s+/g,' ').trim()`);
check('ref-hover-card', pop.length > 20 && /newcomer|consistency condition/i.test(pop), pop.slice(0, 140));
await shot('hover');
ws.close();
const failed = results.filter((r) => !r.ok).length;
console.log(`\n${results.length - failed}/${results.length} passed · screenshots in ${OUT}`);
process.exit(failed ? 1 : 0);
