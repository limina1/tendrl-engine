#!/usr/bin/env node
// Step 2 GUI check: definition cards + per-event glossary promotion.
// Assumes a scratch engine seeded by vocab-request-seed.sh AND
// vocab-definitions.sh (two definitions on §1, glossary currently EMPTY —
// the script ends with the definition removed), chromium on :9333.
// Usage: node scripts/verify/vocab-definitions.mjs --root <30040 coord> --search 'id:<index id>' [--out DIR]
import fs from 'node:fs';
import path from 'node:path';
const argv = process.argv.slice(2);
const opt = (n, d) => { const i = argv.indexOf(`--${n}`); return i >= 0 && i + 1 < argv.length ? argv[i + 1] : d; };
const PORT = Number(opt('port', process.env.PORT ?? '3131')); const CDP = Number(opt('cdp', '9333'));
const APP = `http://localhost:${PORT}/`; const ROOT = opt('root', process.env.ROOT ?? ''); const SEARCH = opt('search', process.env.SEARCH ?? 'k:30040');
const OUT = opt('out', '/tmp/vocab-def-shots'); fs.mkdirSync(OUT, { recursive: true });
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
// Tools → show annotations (local)
await clickJs(`document.querySelector('button[title^="Tools"]')`); await sleep(700);
const showBtn = await clickJs(byText('.tools-sheet__row button, button', /^show( annotations)?$/i)); await sleep(2500);
const loadNow = async () => { try { return await (await fetch(`http://localhost:${PORT}/api/v1/insertions/load`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ root: ROOT, policy: 'local_only' }) })).json(); } catch { return null; } };
const before = ((await loadNow())?.insertions ?? []).filter((i) => i.role === 'definition');
const promotedBefore = before.filter((i) => i.promoted).length;
const defCards = await count('.icard .badge.def, .badge.def, [class*=badge][class*=def]');
// promoted definitions leave the margin (they render under the glossary sub-index), so cards = unpromoted
check('definition-cards', defCards >= 1 && defCards === before.length - promotedBefore, `${defCards} def badges for ${before.length} definitions (${promotedBefore} already promoted; show clicked: ${showBtn})`);
const answers = await count('[class*=answers]');
check('answers-chips', answers >= 1, `${answers} “answers” chips`);
await shot('def-cards');
// promote → glossary on the first definition card: preview then armed sign
const promoteBtn = byText('button', /promote → glossary/);
const opened = await clickJs(promoteBtn); await sleep(1800);
const previewText = await ev(`(document.querySelector('.icard__promote')?.textContent ?? '').replace(/\\s+/g,' ')`);
check('promote-preview', opened && /glossary/i.test(previewText) && /index|slot|unchanged/i.test(previewText), previewText.slice(0, 160));
const wire = await ev(`document.querySelector('.icard__wire-pre, [class*=wire]')?.textContent ?? ''`);
check('promote-event-preview', /"t"|glossary/i.test(wire), `${wire.length} chars`);
await shot('promote-preview');
const armBtn = byText('button', /sign · promote|sign · remove|sure\?/);
await clickJs(armBtn); await sleep(500);
const armed = await ev(`${armBtn}?.textContent.trim() ?? ''`);
await clickJs(armBtn); await sleep(3500);
const toast = await ev(`[...document.querySelectorAll('.toast, [class*=toast]')].map(t=>t.textContent).join(' | ')`);
check('promote-signed', /glossary/i.test(toast) || (await count('.badge.def')) < defCards, `armed label "${armed}" · toast "${toast.slice(0, 120)}"`);
await sleep(2000);
const glossaryRow = await ev(`(()=>{const t=[...document.querySelectorAll('.pane__body--reader .cv-index__title')].find(h=>/glossary/i.test(h.textContent)); if(!t) return null; const row=t.closest('.cv-index'); return row ? row.textContent.replace(/\\s+/g,' ').trim() : t.textContent})()`);
check('glossary-in-reader', !!glossaryRow, glossaryRow ? `nested index row “${glossaryRow}” (collapsed by default)` : 'no .cv-index row titled Glossary');
await shot('after-promote');
const promoted = ((await loadNow())?.insertions ?? []).filter((i) => i.role === 'definition' && i.promoted);
check('engine-says-promoted', promoted.length === promotedBefore + 1, `${promotedBefore} → ${promoted.length} promoted: ${promoted.map((p) => p.term).join(', ')}`);
ws.close();
const failed = results.filter((r) => !r.ok).length;
console.log(`\n${results.length - failed}/${results.length} passed · screenshots in ${OUT}`);
process.exit(failed ? 1 : 0);
