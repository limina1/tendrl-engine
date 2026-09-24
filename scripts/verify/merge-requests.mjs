#!/usr/bin/env node
// Step 4 GUI check: as the REQUESTER, the request card's "relink N terms ▸" on
// someone else's publication → the modal's fork + merge request path
// (derivative block, the 818 preview, the destination set with reason chips,
// the armed "sign + send merge request") → the 818 exists. Assumes:
//   seed as the owner → login a requester → vocab-definitions.sh (the requester's
//   request + the assistant's definitions) → the owner's kind 10002 ingested
//   (so the destination set is deliverable); chromium on :9333.
// Usage: node scripts/verify/merge-requests.mjs --search 'id:<index id>' [--port 3131] [--out DIR]
import fs from 'node:fs';
import path from 'node:path';
const argv = process.argv.slice(2);
const opt = (n, d) => { const i = argv.indexOf(`--${n}`); return i >= 0 && i + 1 < argv.length ? argv[i + 1] : d; };
const PORT = Number(opt('port', process.env.PORT ?? '3131')); const CDP = Number(opt('cdp', '9333'));
const APP = `http://localhost:${PORT}/`; const SEARCH = opt('search', process.env.SEARCH ?? 'k:30040'); const OUT = opt('out', '/tmp/mr-shots'); fs.mkdirSync(OUT, { recursive: true });
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
const opened = await clickJs(byText('button', /^relink \d+ terms? ▸$/)); await sleep(3000);
check('relink-button', opened && (await waitFor('.rl-modal', 10, 300)), 'request card → modal');
const title = await ev(`document.querySelector('.rl-modal .rl-title')?.textContent.replace(/\\s+/g,' ').trim() ?? ''`);
check('foreign-title', /fork \+ merge request/i.test(title), `"${title}"`);
const deriv = await count('.rl-modal .rl-derivative');
const derivRows = await ev(`(document.querySelector('.rl-modal .rl-derivative')?.textContent ?? '').replace(/\\s+/g,' ').trim().slice(0, 200)`);
check('derivative-block', deriv >= 1 && /E → |index′|section′/i.test(derivRows), derivRows);
const mr = await ev(`document.querySelector('.rl-modal .rl-mr[data-kind="818"]')?.textContent ?? ''`);
check('mr-preview', /818/.test(mr) && /source/.test(mr) && /filled with/.test(mr), `${mr.length} chars`);
const destRows = await count('.rl-modal .rl-dest__row');
const reasons = await ev(`[...document.querySelectorAll('.rl-modal .rl-why')].map(e=>e.className.replace(/.*rl-why--(\\w+).*/,'$1'))`);
const deliverable = await ev(`document.querySelector('.rl-modal .rl-dest')?.dataset.deliverable ?? null`);
check('destination-set', destRows >= 1 && Array.isArray(reasons) && reasons.includes('inbox'), `${destRows} rows · reasons ${JSON.stringify(reasons)} · deliverable=${deliverable}`);
await shot('foreign-modal');
const sendBtn = byText('.rl-modal button', /sign \+ send merge request|sure\? sign derivative/);
const disabled = await ev(`${sendBtn}?.disabled ?? null`);
check('send-enabled', disabled === false, `button disabled=${disabled}`);
await clickJs(sendBtn); await sleep(500);
const armed = await ev(`${sendBtn}?.textContent.trim() ?? ''`);
check('send-armed', /sure\? sign derivative \+ send 818 to \d+ relay/.test(armed), `"${armed}"`);
await clickJs(sendBtn); await sleep(9000); // signs + broadcasts to the test relay (theforest) — a relay round-trip
const closed = !(await ev(`!!document.querySelector('.rl-modal')`));
const toast = await ev(`[...document.querySelectorAll('.toast, [class*=toast]')].map(t=>t.textContent).join(' | ')`);
check('sent', closed, `modal closed · toast "${toast.slice(0, 140)}"`);
await shot('after-send');
let list = null; try { list = await (await fetch(`http://localhost:${PORT}/api/v1/merge-requests?role=requester&status=open`)).json(); } catch {}
const mine = list?.requests ?? [];
check('818-listed', mine.length >= 1 && mine[0].status === 'open', `${mine.length} open merge request(s) by me`);
ws.close();
const failed = results.filter((r) => !r.ok).length;
console.log(`\n${results.length - failed}/${results.length} passed · screenshots in ${OUT}`);
process.exit(failed ? 1 : 0);
