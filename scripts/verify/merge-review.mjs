#!/usr/bin/env node
// Step 5c GUI check: as the OWNER — the 🔔 pill shows 1 ask → notifications
// buffer → "review the diff ▸" → the review pane (header, ask, index block,
// diff hunks, event preview, footer) → armed merge → ✓ merged → notifications
// `done`. Assumes an OPEN 818 against the owner's publication with the owner
// logged in (e.g. the requester half of merge-review.sh without its merge —
// or run merge-requests.sh's steps up to the owner re-login), chromium on :9333.
// Usage: node scripts/verify/merge-review.mjs [--port 3131] [--out DIR]
import fs from 'node:fs';
import path from 'node:path';
const argv = process.argv.slice(2);
const opt = (n, d) => { const i = argv.indexOf(`--${n}`); return i >= 0 && i + 1 < argv.length ? argv[i + 1] : d; };
const PORT = Number(opt('port', process.env.PORT ?? '3131')); const CDP = Number(opt('cdp', '9333'));
const APP = `http://localhost:${PORT}/`; const OUT = opt('out', '/tmp/review-shots'); fs.mkdirSync(OUT, { recursive: true });
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
await send('Page.navigate', { url: APP }); await sleep(4500);
await ev(`(()=>{const m=document.querySelector('.nm-modal'); if(m){const c=[...m.querySelectorAll('button')].find(b=>/^confirm/i.test(b.textContent.trim())); if(c) c.click();} document.querySelectorAll('.rf-action--ghost, .rf-close, .dt-skip').forEach(b=>b.click());})()`); await sleep(1500);
const pill = await ev(`document.querySelector('.pill--notif')?.textContent.replace(/\\s+/g,' ').trim() ?? null`);
check('bell-one-ask', !!pill && /1 ask\b/.test(pill), `"${pill}"`);
await clickJs(`document.querySelector('.pill--notif')`); await sleep(2000);
const reviewBtn = byText('button', /review the diff ▸/);
const opened = await clickJs(reviewBtn); await sleep(3000);
check('review-opens', opened && (await waitFor('.mr[data-mr]', 15, 300)), 'notifications → review the diff ▸ → .mr');
const status = await ev(`document.querySelector('.mr')?.dataset.status ?? null`);
const role = await ev(`document.querySelector('.mr')?.dataset.role ?? null`);
const base = await ev(`document.querySelector('.mr__base--ok, .mr__base--warn')?.textContent.replace(/\\s+/g,' ').trim() ?? ''`);
check('review-header', status === 'open' && role === 'owner' && /applies cleanly|moved on/i.test(base), `status=${status} role=${role} base="${base}"`);
const ask = await ev(`document.querySelector('.mr__ask')?.textContent.replace(/\\s+/g,' ').trim() ?? ''`);
check('review-ask', /Relink \d+ terms/.test(ask), `"${ask}"`);
const structRows = await ev(`[...document.querySelectorAll('.mr__block[data-block=index] .mr__struct')].map(e=>e.textContent.replace(/\\s+/g,' ').trim())`);
check('review-index-block', Array.isArray(structRows) && structRows.some((r) => /Glossary/.test(r)) && structRows.some((r) => /replaced/.test(r)), (structRows ?? []).join(' | ').slice(0, 200));
const added = await count('.mr .diff__ln--added');
const refRuns = await count('.mr .diff__ref');
check('review-diff-hunks', added >= 2 && refRuns >= 2, `${added} added lines · ${refRuns} tinted refs`);
const wire = await ev(`document.querySelector('.mr__wire-pre')?.textContent ?? ''`);
check('review-event-preview', /30041/.test(wire) && /30040/.test(wire), `${wire.length} chars`);
const agentSlot = await ev(`!!document.querySelector('.mr-agent[data-slot=assistant-review]')`);
check('review-agent-slot', agentSlot === true, 'placeholder for step 6');
await shot('review');
const mergeBtn = byText('.mr__foot button', /merge \+|sure\? sign/);
await clickJs(mergeBtn); await sleep(500);
const armed = await ev(`${mergeBtn}?.textContent.trim() ?? ''`);
check('merge-armed', /sure\? sign \d+ next version/.test(armed), `"${armed}"`);
await clickJs(mergeBtn); await sleep(5000);
const after = await ev(`document.querySelector('.mr')?.dataset.status ?? null`);
const badge = await ev(`document.querySelector('.mr__badge--ok')?.textContent.replace(/\\s+/g,' ').trim() ?? ''`);
const buttonsGone = (await count('.mr__foot .mr__btn--merge')) === 0;
check('merged', after === 'merged' && /merged/i.test(badge) && buttonsGone, `status=${after} badge="${badge}" buttons gone=${buttonsGone}`);
await shot('merged');
let n = null; try { n = await (await fetch(`http://localhost:${PORT}/api/v1/notifications?status=open`)).json(); } catch {}
check('notifications-done', (n?.acts ?? []).length === 0 && (n?.done ?? []).some((r) => r.status === 'merged'), `acts ${(n?.acts ?? []).length} · done ${(n?.done ?? []).length}`);
ws.close();
const failed = results.filter((r) => !r.ok).length;
console.log(`\n${results.length - failed}/${results.length} passed · screenshots in ${OUT}`);
process.exit(failed ? 1 : 0);
