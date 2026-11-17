#!/usr/bin/env node
// Feed kinds + long-form reader GUI check (feat/longform-tools-kinds): engine on --port
// with a synced general feed (articles, publications, wiki); chromium on :9333.
// Usage: node scripts/verify/feed-kinds.mjs [--port 3131] [--out DIR]
import fs from 'node:fs';
import path from 'node:path';
const argv = process.argv.slice(2);
const opt = (n, d) => { const i = argv.indexOf(`--${n}`); return i >= 0 && i + 1 < argv.length ? argv[i + 1] : d; };
const PORT = Number(opt('port', process.env.PORT ?? '3131')); const CDP = Number(opt('cdp', '9333'));
const APP = `http://localhost:${PORT}/`; const SEARCH = opt('search', process.env.SEARCH ?? 'k:30040'); const OUT = opt('out', '/tmp/feed-kinds-shots'); fs.mkdirSync(OUT, { recursive: true });
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
// close a first-run network-mode chooser / confirm modal if one is up
await ev(`(()=>{[...document.querySelectorAll('[role=dialog] button')].filter(b=>/^(auto|skip|close|×|cancel)/i.test(b.textContent.trim())).slice(0,1).forEach(b=>b.click());})()`); await sleep(800);
await sleep(2500);
// the kinds button → the checkable list (all / none / invert)
check('kinds-button', await waitFor('.kinds-btn'));
await clickJs(`document.querySelector('.kinds-btn')`); await sleep(500);
check('kind-panel', await waitFor('.kind-panel .kind-row'));
const rowsTxt = async () => ev(`[...document.querySelectorAll('.kind-list .kind-row')].map(r=>(r.querySelector('input').checked?'+':'-')+r.querySelector('.kind-name').textContent.trim()).join('|')`);
const r0 = await rowsTxt();
check('list-default', r0 === '+publications|-standalone sections|+articles|+wiki pages|-specs', r0);
await waitFor('.feed-list .row', 40, 300);
const metas = async () => ev(`[...document.querySelectorAll('.feed-list .row .meta')].map(m=>m.textContent.trim().toLowerCase().replace(/^\\d+ sections$/, 'publication'))`);
await shot('panel-default');
// none → empty feed, no relay sync
await clickJs(byText('.kind-tool', /^none$/)); await sleep(2200);
check('none-empty', (await count('.feed-list .row')) === 0 && /No kinds checked/.test(await ev(`document.querySelector('.empty')?.textContent ?? ''`)), await rowsTxt());
// invert → every kind checked
await clickJs(byText('.kind-tool', /^invert$/)); await sleep(2500);
const r1 = await rowsTxt();
check('invert-all', !r1.includes('-'), r1);
// uncheck articles by its row → publications / sections / wiki / specs only
await clickJs(`[...document.querySelectorAll('.kind-list .kind-row')].find(r=>/articles/.test(r.textContent)).querySelector('input')`); await sleep(2500);
const m2 = await metas();
const kindsSeen = [...new Set(m2)];
check('articles-off', m2.length > 0 && !m2.includes('article'), `${m2.length} rows: ${kindsSeen.join(',')}`);
await shot('no-articles');
// invert → articles only
await clickJs(byText('.kind-tool', /^invert$/)); await sleep(2500);
const m3 = await metas();
check('invert-articles-only', m3.length > 0 && m3.every((t) => t === 'article'), `${m3.length} rows: ${[...new Set(m3)].join(',')}`);
const btn = await ev(`document.querySelector('.kinds-btn').textContent.trim()`);
check('button-summary', /^kinds 1\/5/.test(btn), btn);
// back to defaults minus articles for the wiki step
await clickJs(byText('.kind-tool', /^invert$/)); await sleep(2500);
// open a wiki row from the feed: the reader, with the tools button
const wikiRow = `[...document.querySelectorAll('.feed-list .row')].find(r=>r.querySelector('.meta')?.textContent.trim().toLowerCase()==='wiki')`;
if (await ev(`!!(${wikiRow})`)) {
  await clickJs(`(${wikiRow}).querySelector('.title')`); await sleep(3000);
  check('wiki-in-reader', await waitFor('.pane__body--reader'));
  await clickJs(`document.querySelector('button[title^="Tools"]')`); await sleep(800);
  const tools = await ev(`document.querySelector('.tools-sheet')?.innerText.replace(/\\s+/g,' ').slice(0,400) ?? ''`);
  check('tools-sheet', /highlight/i.test(tools), tools);
  await shot('wiki-tools');
} else check('wiki-row', false, 'no wiki row in feed');
// openCoord path (nostrdown refs, comment targets): a 30818 coordinate → reader buffer id
const coord = await ev(`(async()=>{const r=await fetch('/api/v1/publications?limit=5&policy=local_only&general=true'); const d=await r.json(); const w=d.publications.find(p=>p.addr.kind===30818); return w?\`30818:\${w.addr.pubkey}:\${w.addr.d_tag}\`:null})()`);
check('have-wiki-coord', typeof coord === 'string', String(coord));
// restore the defaults
await ev(`fetch('/api/v1/feed/kinds',{method:'PUT',headers:{'content-type':'application/json'},body:JSON.stringify({only:[30040,30023,30818]})})`);
ws.close();
const failed = results.filter((r) => !r.ok).length;
console.log(`\n${results.length - failed}/${results.length} passed · screenshots in ${OUT}`);
process.exit(failed ? 1 : 0);
