#!/usr/bin/env node
// Step 5 GUI check: the 🔔 pill, the notifications buffer (asks · activity ·
// mine), Settings › Notifications. Assumes a scratch engine seeded by
// vocab-request-seed.sh and scripts/verify/notifications.sh (which enables
// 9802 count + 1111 list and ingests one highlight + one comment on my
// publication), chromium on :9333.
// Usage: node scripts/verify/notifications.mjs [--port 3131] [--out DIR]
import fs from 'node:fs';
import path from 'node:path';
const argv = process.argv.slice(2);
const opt = (n, d) => { const i = argv.indexOf(`--${n}`); return i >= 0 && i + 1 < argv.length ? argv[i + 1] : d; };
const PORT = Number(opt('port', process.env.PORT ?? '3131')); const CDP = Number(opt('cdp', '9333'));
const APP = `http://localhost:${PORT}/`; const OUT = opt('out', '/tmp/notif-shots'); fs.mkdirSync(OUT, { recursive: true });
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
// 1. the bell pill on the modeline: "🔔 0 asks · 1 new" (9802 counts, 1111 only lists)
const pill = await ev(`document.querySelector('.pill--notif')?.textContent.replace(/\\s+/g,' ').trim() ?? null`);
check('bell-pill', !!pill && /🔔/.test(pill) && /1 new/.test(pill), `"${pill}"`);
await shot('modeline');
// 2. click → the notifications buffer with its three groups
await clickJs(`document.querySelector('.pill--notif')`); await sleep(2000);
const bufOpen = await waitFor('.nb-head, .nb-title', 10, 300);
const groups = await ev(`[...document.querySelectorAll('.nb-head, .nb-title, [class*=grp], h2, h3')].map(e=>e.textContent.replace(/\\s+/g,' ').trim()).filter(t=>t.length<60)`);
const bodyText = await ev(`document.body.textContent.replace(/\\s+/g,' ')`);
check('notifications-buffer', bufOpen && /asks/i.test(bodyText) && /activity/i.test(bodyText), Array.isArray(groups) ? groups.slice(0, 6).join(' · ') : String(groups));
const rows = await ev(`[...document.querySelectorAll('.pane__body--notifications .badge, [class*=badge]')].map(e=>e.textContent.trim()).filter(Boolean).slice(0,12)`);
const highlightRow = /highlighted/i.test(bodyText);
const commentRow = /commented on/i.test(bodyText);
check('activity-rows', highlightRow === true && commentRow === true, `highlight row ${highlightRow} · comment row ${commentRow} · badges ${JSON.stringify(rows).slice(0, 120)}`);
const settingsLink = await ev(`!!${byText('button, a', /settings › notifications/)}`);
check('settings-link', settingsLink === true, 'activity header → settings › notifications ▸');
await shot('notifications-buffer');
// 3. settings › notifications: fixed rows checked + disabled, optional rows editable
await clickJs(byText('button, a', /settings › notifications/)); await sleep(2000);
const fixedRows = await count('.notif-row--fixed');
const fixedChecked = await ev(`[...document.querySelectorAll('.notif-row--fixed input[type=checkbox]')].every(i=>i.checked && i.disabled)`);
const optRows = await count('.notif-row:not(.notif-row--fixed):not(.notif-row--head)');
const on9802 = await ev(`(()=>{const r=[...document.querySelectorAll('.notif-row')].find(r=>/9802/.test(r.textContent)); const i=r?.querySelector('input[type=checkbox]'); const s=r?.querySelector('select'); return i && s ? {on:i.checked, how:s.value} : null})()`);
check('settings-rows', fixedRows === 3 && fixedChecked === true && optRows >= 5, `${fixedRows} fixed (checked+disabled: ${fixedChecked}) · ${optRows} optional`);
check('settings-reflects-prefs', !!on9802 && on9802.on === true && on9802.how === 'count', JSON.stringify(on9802));
await shot('settings');
// 4. toggle 9735 on → PUT → the API agrees
await ev(`(()=>{const r=[...document.querySelectorAll('.notif-row')].find(r=>/9735/.test(r.textContent)); const i=r?.querySelector('input[type=checkbox]'); if(i&&!i.checked){ i.click(); } })()`); await sleep(1200);
let prefs = null; try { prefs = await (await fetch(`http://localhost:${PORT}/api/v1/notifications/prefs`)).json(); } catch {}
const r9735 = (prefs?.rules ?? []).find((r) => r.key === '9735');
check('settings-put', !!r9735 && r9735.on === true, r9735 ? `9735 on=${r9735.on} how=${r9735.how}` : 'no 9735 rule');
ws.close();
const failed = results.filter((r) => !r.ok).length;
console.log(`\n${results.length - failed}/${results.length} passed · screenshots in ${OUT}`);
process.exit(failed ? 1 : 0);
