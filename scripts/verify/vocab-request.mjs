#!/usr/bin/env node
// Vocabulary request — CDP verification harness (plan Phase 5c step 1).
// Self-contained: Node >= 22 (native WebSocket + fetch), no dependencies.
// Same driver as scripts/verify/annotations-table.mjs.
//
// Assumes:
//   - a scratch engine on $PORT (default 3131) seeded by
//     scripts/verify/vocab-request-seed.sh (a 3-section publication by a
//     logged-in user; network mode auto);
//   - chromium --headless=new --remote-debugging-port=9333 already running.
//
// Usage:
//   node scripts/verify/vocab-request.mjs --root 30040:<pk>:<d> --search 'id:<index id>' \
//        [--port 3131] [--cdp 9333] [--out /tmp/vocab-shots]
//
// Checks: app-open · search-result · reader-open · reader-sections ·
// annotate-mode · capture-card · modal-open · modal-vocabulary · words-rendered ·
// mark-phrase · generated-line · preview-terms · unmark-click · sign-local ·
// request-listed. Exit 1 on any FAIL.

import fs from 'node:fs';
import path from 'node:path';

const argv = process.argv.slice(2);
const opt = (name, dflt) => { const i = argv.indexOf(`--${name}`); return i >= 0 && i + 1 < argv.length ? argv[i + 1] : dflt; };
const PORT = Number(opt('port', process.env.PORT ?? '3131'));
const CDP = Number(opt('cdp', process.env.CDP_PORT ?? '9333'));
const APP = `http://localhost:${PORT}/`;
const API = `http://localhost:${PORT}/api/v1`;
const ROOT = opt('root', process.env.ROOT ?? '');
const SEARCH = opt('search', process.env.SEARCH ?? (ROOT ? `k:30040 by:${ROOT.split(':')[1]}` : 'k:30040'));
const OUT = opt('out', process.env.OUT ?? '/tmp/vocab-shots');
fs.mkdirSync(OUT, { recursive: true });

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let page = null;
for (let i = 0; i < 40 && !page; i++) { try { const tabs = await (await fetch(`http://localhost:${CDP}/json`)).json(); page = tabs.find((t) => t.type === 'page'); } catch {} if (!page) await sleep(250); }
if (!page) { console.error(`FAIL cdp — no page target on :${CDP}`); process.exit(2); }
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
let seq = 0; const pending = new Map();
ws.onmessage = (m) => { const d = JSON.parse(m.data); if (d.id && pending.has(d.id)) { pending.get(d.id)(d); pending.delete(d.id); } };
const send = (method, params = {}) => new Promise((r) => { const id = ++seq; pending.set(id, r); ws.send(JSON.stringify({ id, method, params })); });
const ev = async (expr) => { const r = await send('Runtime.evaluate', { expression: expr, awaitPromise: true, returnByValue: true }); if (r.result?.exceptionDetails) return { __err: r.result.exceptionDetails.text ?? 'exception' }; return r.result?.result?.value ?? null; };
const rectOf = async (js) => ev(`(()=>{const e=(${js}); if(!e) return null; e.scrollIntoView({block:'center', inline:'center'}); const r=e.getBoundingClientRect(); if(!r.width||!r.height) return null; return {x:r.x+r.width/2,y:r.y+r.height/2,w:r.width,h:r.height}})()`);
const mouse = (type, x, y, extra = {}) => send('Input.dispatchMouseEvent', { type, x, y, button: 'left', clickCount: 1, ...extra });
const clickAt = async (r) => { if (!r) return false; await mouse('mouseMoved', r.x, r.y, { button: 'none' }); await mouse('mousePressed', r.x, r.y); await mouse('mouseReleased', r.x, r.y); return true; };
const clickJs = async (js) => clickAt(await rectOf(js));
const byText = (sel, re) => `[...document.querySelectorAll(${JSON.stringify(sel)})].find(b=>${re}.test(b.textContent.replace(/\\s+/g,' ').trim()))`;
const count = (sel) => ev(`document.querySelectorAll(${JSON.stringify(sel)}).length`);
const exists = async (sel) => (await count(sel)) > 0;
const waitFor = async (sel, tries = 30, every = 300) => { for (let i = 0; i < tries; i++) { if (await exists(sel)) return true; await sleep(every); } return false; };
let shotN = 0;
const shot = async (name) => { const r = await send('Page.captureScreenshot', { format: 'png' }); if (!r.result?.data) return; fs.writeFileSync(path.join(OUT, `${String(++shotN).padStart(2, '0')}-${name}.png`), Buffer.from(r.result.data, 'base64')); };
const results = [];
const check = (name, ok, detail = '') => { results.push({ name, ok }); console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? ' — ' + detail : ''}`); return ok; };

await send('Runtime.enable'); await send('Page.enable');
await send('Emulation.setDeviceMetricsOverride', { width: 1280, height: 900, deviceScaleFactor: 1, mobile: false });
await send('Page.navigate', { url: APP });
await sleep(3500);

// 1. first-run modals
const modal = await ev(`(()=>{const m=document.querySelector('.nm-modal'); if(!m) return 'none'; const w=[...m.querySelectorAll('button')].find(b=>/walkthrough/i.test(b.textContent)); if(w && /✓/.test(w.textContent)) w.click(); const c=[...m.querySelectorAll('button')].find(b=>/^confirm/i.test(b.textContent.trim())); if(c){c.click(); return 'confirmed'} return 'no confirm button'})()`);
await sleep(800);
await ev(`(()=>{document.querySelectorAll('.rf-action--ghost, .rf-close').forEach(b=>b.click()); document.querySelectorAll('.dt-skip').forEach(b=>b.click());})()`);
await sleep(500);
check('app-open', await ev(`!!document.querySelector('.rail--research')`), `network-mode modal: ${modal}`);

// 2. reader
await clickJs(`document.querySelector('.rail--research')`); await sleep(900);
const typed = await ev(`(()=>{const i=document.querySelector('.search-panel input, input[placeholder*=exact], .search-input input'); if(!i) return false; const s=Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set; s.call(i,${JSON.stringify(SEARCH)}); i.dispatchEvent(new Event('input',{bubbles:true})); i.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true})); return true})()`);
await sleep(2500);
check('search-result', typed && (await waitFor('.result-title', 20, 300)), `query ${JSON.stringify(SEARCH)}`);
await clickJs(`document.querySelector('.result-title')`); await sleep(1200);
const readBtn = await clickJs(byText('[role=dialog] button', /Read publication/)); await sleep(3500);
check('reader-open', readBtn && (await waitFor('.pane__body--reader', 20, 300)));
await clickJs(byText('.seg button, button', /^scroll$/)); await sleep(2000);
const sections = await count('.pane__body--reader .section, .pane__body--reader .continuous-section');
check('reader-sections', sections >= 3, `${sections} sections`);

// 3. annotate mode (Tools sheet → the pill)
await clickJs(`document.querySelector('button[title^="Tools"]')`); await sleep(700);
const pillOn = await clickJs(byText('.hl-mode-pill', /annotate mode/));
await sleep(500);
check('annotate-mode', pillOn && (await ev(`!!document.querySelector('.hl-mode-pill--on')`)));

// 4. select a word in §1 (a Range + the document mouseup the capture listens to) → capture card → annotate →
const selected = await ev(`(()=>{const p=document.querySelector('.pane__body--reader .section-content'); if(!p) return 'no paragraph'; const walker=document.createTreeWalker(p, NodeFilter.SHOW_TEXT); let n; while((n=walker.nextNode())){ const m=/\\b(newcomer|caution|theory)\\b/i.exec(n.data); if(m){ const r=document.createRange(); r.setStart(n, m.index); r.setEnd(n, m.index+m[0].length); const s=getSelection(); s.removeAllRanges(); s.addRange(r); document.dispatchEvent(new MouseEvent('mouseup',{bubbles:true})); return m[0]; } } return 'no target word'})()`);
await sleep(900);
const card = await waitFor('.hc-btn--post', 10, 300);
check('capture-card', card, `selected "${selected}"`);
await shot('capture');
await clickJs(`document.querySelector('.hc-btn--post')`); await sleep(1500);
check('modal-open', await waitFor('.rq-seg__opt', 10, 300));
// vocabulary output
await clickJs(byText('.rq-seg__opt', /^vocabulary$/)); await sleep(1800);
check('modal-vocabulary', await ev(`document.querySelector('.rq-seg__opt.on')?.textContent.trim()==='vocabulary'`));
const words = await count('.rq-word');
check('words-rendered', words > 20, `${words} words${selected && /^[a-z]/i.test(selected) ? `; pre-marked ${await count('.rq-word.on')}` : ''}`);
await shot('modal-vocabulary');

// 5. drag over two adjacent words → one phrase marked; generated line and preview carry it
const w0 = await rectOf(`document.querySelectorAll('.rq-word')[12]`);
const w1 = await rectOf(`document.querySelectorAll('.rq-word')[13]`);
if (w0 && w1) { await mouse('mouseMoved', w0.x, w0.y, { button: 'none' }); await mouse('mousePressed', w0.x, w0.y); await mouse('mouseMoved', w1.x, w1.y, { buttons: 1 }); await mouse('mouseReleased', w1.x, w1.y); }
await sleep(700);
const marked = await count('.rq-word.on');
const phrase = await ev(`[...document.querySelectorAll('.rq-word')].slice(12,14).map(e=>e.textContent.replace(/^[^\\p{L}\\p{N}]+|[^\\p{L}\\p{N}]+$/gu,'')).join(' ')`);
check('mark-phrase', marked >= 2, `${marked} marked words; phrase "${phrase}"`);
const gen = await ev(`document.querySelector('.rq-gen')?.textContent.replace(/\\s+/g,' ').trim() ?? ''`);
check('generated-line', gen.startsWith('define these words in the context of these events:') && gen.includes(phrase), `"${gen}"`);
await sleep(900); // preview debounce + round-trip
const pre = await ev(`document.querySelector('.rq-preview__pre')?.textContent ?? ''`);
check('preview-terms', pre.includes('"term"') && pre.includes(phrase) && pre.includes('annotation-request') && pre.includes('"vocabulary"'), pre ? `preview ${pre.length} chars` : 'no preview');
await shot('marked');

// 6. click a marked word → that mark goes
await clickAt(w0); await sleep(500);
const afterClick = await count('.rq-word.on');
check('unmark-click', afterClick < marked, `${marked} → ${afterClick}`);
// re-mark for the sign step
if (w0 && w1) { await mouse('mouseMoved', w0.x, w0.y, { button: 'none' }); await mouse('mousePressed', w0.x, w0.y); await mouse('mouseMoved', w1.x, w1.y, { buttons: 1 }); await mouse('mouseReleased', w1.x, w1.y); }
await sleep(600);

// 7. sign · keep local → the record lists with terms + the generated task
const signed = await clickJs(byText('button', /sign · keep local|save request/)); await sleep(2500);
check('sign-local', signed && !(await exists('.rq-seg__opt')), 'modal closed');
await shot('after-sign');
let listed = null;
try { listed = await (await fetch(`${API}/annotations/requests?root=${encodeURIComponent(ROOT)}&status=all`)).json(); } catch (e) { listed = { error: String(e) }; }
const reqs = listed?.requests ?? [];
const vocab = reqs.find((r) => r.output === 'vocabulary');
check('request-listed', !!vocab && Array.isArray(vocab.terms) && vocab.terms.length >= 1 && /^define these words in the context of these events: /.test(vocab.task ?? ''), vocab ? `terms ${JSON.stringify(vocab.terms)} · task "${vocab.task}" · ${vocab.coordinate ? 'signed 30041' : 'record'}` : `no vocabulary request among ${reqs.length}`);

ws.close();
const failed = results.filter((r) => !r.ok).length;
console.log(`\n${results.length - failed}/${results.length} passed · screenshots in ${OUT}`);
process.exit(failed ? 1 : 0);
