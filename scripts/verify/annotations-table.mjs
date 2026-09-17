#!/usr/bin/env node
// Annotations table — CDP verification harness (Track E of
// docs/annotations-table-contract.org). Self-contained: Node >= 22 (native
// WebSocket + fetch), no dependencies.
//
// Assumes:
//   - a scratch engine on $PORT (default 3131) seeded per the
//     "Annotations table — verify" recipe in docs/commands.org
//     (3 sections, 5 notes by 2 keys, 1 signed request), network mode auto;
//   - chromium --headless=new --remote-debugging-port=9333 already running
//     (any page; the harness navigates it).
//
// Usage:
//   node scripts/verify/annotations-table.mjs --root 30040:<pk>:<d> \
//        [--search 'id:<index event id>'] [--expect-cards 6] [--port 3131] \
//        [--cdp 9333] [--out /tmp/annot-shots] [--keep-mobile]
//
// Every check prints `PASS name — detail` or `FAIL name — detail`; the exit
// code is 1 when any check failed. Selectors are the contract's DOM
// contract verbatim — change them there first.

import fs from 'node:fs';
import path from 'node:path';

// ---------------------------------------------------------------- args

const argv = process.argv.slice(2);
const opt = (name, dflt) => {
	const i = argv.indexOf(`--${name}`);
	return i >= 0 && i + 1 < argv.length ? argv[i + 1] : dflt;
};
const flag = (name) => argv.includes(`--${name}`);

const PORT = Number(opt('port', process.env.PORT ?? '3131'));
const CDP = Number(opt('cdp', process.env.CDP_PORT ?? '9333'));
const APP = `http://localhost:${PORT}/`;
const ROOT = opt('root', process.env.ROOT ?? '');
const SEARCH = opt('search', process.env.SEARCH ?? (ROOT ? `k:30040 by:${ROOT.split(':')[1]}` : 'k:30040'));
const EXPECT_CARDS = Number(opt('expect-cards', process.env.EXPECT_CARDS ?? '0')) || 0;
const OUT = opt('out', process.env.OUT ?? '/tmp/annotations-table-shots');
const KEEP_MOBILE = flag('keep-mobile');

fs.mkdirSync(OUT, { recursive: true });

// ---------------------------------------------------------------- driver

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let page = null;
for (let i = 0; i < 40 && !page; i++) {
	try {
		const tabs = await (await fetch(`http://localhost:${CDP}/json`)).json();
		page = tabs.find((t) => t.type === 'page');
	} catch {}
	if (!page) await sleep(250);
}
if (!page) {
	console.error(`FAIL cdp — no page target on :${CDP} (start chromium --headless=new --remote-debugging-port=${CDP})`);
	process.exit(2);
}

const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
let seq = 0;
const pending = new Map();
ws.onmessage = (m) => {
	const d = JSON.parse(m.data);
	if (d.id && pending.has(d.id)) { pending.get(d.id)(d); pending.delete(d.id); }
};
const send = (method, params = {}) => new Promise((r) => {
	const id = ++seq;
	pending.set(id, r);
	ws.send(JSON.stringify({ id, method, params }));
});
const ev = async (expr) => {
	const r = await send('Runtime.evaluate', { expression: expr, awaitPromise: true, returnByValue: true });
	if (r.result?.exceptionDetails) return { __err: r.result.exceptionDetails.text ?? 'exception' };
	return r.result?.result?.value ?? null;
};
const rectOf = async (js) => ev(`(()=>{const e=(${js}); if(!e) return null; e.scrollIntoView({block:'center', inline:'center'}); const r=e.getBoundingClientRect(); if(!r.width||!r.height) return null; return {x:r.x+r.width/2,y:r.y+r.height/2,w:r.width,h:r.height}})()`);
const mouse = (type, x, y, extra = {}) => send('Input.dispatchMouseEvent', { type, x, y, button: 'left', clickCount: 1, ...extra });
const clickAt = async (r) => { if (!r) return false; await mouse('mouseMoved', r.x, r.y, { button: 'none' }); await mouse('mousePressed', r.x, r.y); await mouse('mouseReleased', r.x, r.y); return true; };
const clickJs = async (js) => clickAt(await rectOf(js));
const byText = (sel, re) => `[...document.querySelectorAll(${JSON.stringify(sel)})].find(b=>${re}.test(b.textContent.replace(/\\s+/g,' ').trim()))`;
const count = (sel) => ev(`document.querySelectorAll(${JSON.stringify(sel)}).length`);
const exists = async (sel) => (await count(sel)) > 0;
const waitFor = async (sel, tries = 30, every = 300) => { for (let i = 0; i < tries; i++) { if (await exists(sel)) return true; await sleep(every); } return false; };
let shotN = 0;
const shot = async (name) => {
	const r = await send('Page.captureScreenshot', { format: 'png' });
	if (!r.result?.data) return;
	const file = path.join(OUT, `${String(++shotN).padStart(2, '0')}-${name}.png`);
	fs.writeFileSync(file, Buffer.from(r.result.data, 'base64'));
};

// ---------------------------------------------------------------- checks

const results = [];
const check = (name, ok, detail = '') => {
	results.push({ name, ok });
	console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? ' — ' + detail : ''}`);
	return ok;
};
const skip = (name, why) => { results.push({ name, ok: false }); console.log(`FAIL ${name} — skipped: ${why}`); return false; };

await send('Runtime.enable');
await send('Page.enable');
await send('Emulation.setDeviceMetricsOverride', { width: 1280, height: 900, deviceScaleFactor: 1, mobile: false });
await send('Page.navigate', { url: APP });
await sleep(3500);

// -- 1. first-run modals: network-mode choice (turn the walkthrough off,
//    confirm), any fetch-confirm sheet (cancel), discovery tips (skip).
const modal = await ev(`(()=>{const m=document.querySelector('.nm-modal'); if(!m) return 'none'; const w=[...m.querySelectorAll('button')].find(b=>/walkthrough/i.test(b.textContent)); if(w && /✓/.test(w.textContent)) w.click(); const c=[...m.querySelectorAll('button')].find(b=>/^confirm/i.test(b.textContent.trim())); if(c){c.click(); return 'confirmed'} return 'no confirm button'})()`);
await sleep(800);
await ev(`(()=>{document.querySelectorAll('.rf-action--ghost, .rf-close').forEach(b=>b.click()); for (const b of document.querySelectorAll('button')) { if (/skip the rest|skip/i.test(b.textContent) && b.closest('[class*=tip], .dt-skip')) b.click(); } document.querySelectorAll('.dt-skip').forEach(b=>b.click());})()`);
await sleep(500);
check('app-open', (await ev(`!!document.querySelector('.rail--research') || !!document.querySelector('.mb-bar, .mobile-bar')`)), `network-mode modal: ${modal}`);
await shot('app');

// -- 2. open the seeded publication in the reader: search → result → event
//    menu → "Read publication"; then the scroll (continuous) view so every
//    section is on the page for the hover / scroll checks.
await clickJs(`document.querySelector('.rail--research')`);
await sleep(900);
const typed = await ev(`(()=>{const i=document.querySelector('.search-panel input, input[placeholder*=exact], .search-input input'); if(!i) return false; const s=Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set; s.call(i,${JSON.stringify(SEARCH)}); i.dispatchEvent(new Event('input',{bubbles:true})); i.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true})); return true})()`);
await sleep(2500);
const gotResult = await waitFor('.result-title', 20, 300);
check('search-result', typed && gotResult, `query ${JSON.stringify(SEARCH)}`);
await clickJs(`document.querySelector('.result-title')`);
await sleep(1200);
const readBtn = await clickJs(byText('[role=dialog] button', /Read publication/));
await sleep(3500);
const readerOpen = await waitFor('.pane__body--reader', 20, 300);
check('reader-open', readBtn && readerOpen, readBtn ? '' : 'no "Read publication" in the event menu');
await clickJs(byText('.seg button, button', /^scroll$/));
await sleep(2000);
const sectionsOnPage = await count('.pane__body--reader .section, .pane__body--reader .continuous-section');
check('reader-sections', sectionsOnPage >= 3, `${sectionsOnPage} sections on the page`);
await shot('reader');

// -- 3. Tools → table N → a split with the table buffer.
const toolsOpened = await clickJs(`document.querySelector('button[title^="Tools"]')`);
await sleep(700);
const tableBtn = await ev(`document.querySelector('.tools-sheet__row button.ins-table')?.textContent.replace(/\\s+/g,' ').trim() ?? null`);
if (!tableBtn) {
	skip('table-button', toolsOpened ? 'no .tools-sheet__row button.ins-table (Track B not merged?)' : 'Tools sheet did not open');
} else {
	check('table-button', /table/.test(tableBtn), `label "${tableBtn}"`);
	await clickJs(`document.querySelector('.tools-sheet__row button.ins-table')`);
	await sleep(1500);
}
// a split = the reader body and the table body both on screen at once
const readerStill = await exists('.pane__body--reader');
const tablePane = await exists('.pane__body--annotations .abuf');
const abufRoot = await ev(`document.querySelector('.abuf')?.getAttribute('data-root') ?? null`);
const rootOk = !!abufRoot && (!ROOT || abufRoot === ROOT);
check('table-split', readerStill && tablePane && rootOk, `reader pane ${readerStill}, table pane ${tablePane}; .abuf[data-root]=${abufRoot ?? 'none'}${ROOT && abufRoot && abufRoot !== ROOT ? ' (expected ' + ROOT + ')' : ''}`);
await waitFor('.acard', 30, 400);
await shot('table-open');

// -- 4. every layout: lane + card counts.
const modes = (await ev(`[...document.querySelectorAll('.lay button[data-mode]')].map(b=>b.dataset.mode)`)) ?? [];
if (!Array.isArray(modes) || modes.length === 0) {
	skip('layouts', 'no .lay button[data-mode] (Track C not merged?)');
} else {
	check('layout-modes', modes.length === 5 && ['section', 'author', 'timeline', 'free', 'collective'].every((m) => modes.includes(m)), modes.join(' · '));
	for (const m of modes) {
		await clickJs(`document.querySelector('.lay button[data-mode="${m}"]')`);
		await sleep(700);
		const lanes = await count('.abuf .lane');
		const cards = await count('.abuf .acard');
		const ok = EXPECT_CARDS ? cards === EXPECT_CARDS : cards > 0;
		check(`layout-${m}`, ok && (m === 'timeline' || m === 'free' ? true : lanes > 0), `${lanes} lanes, ${cards} cards${EXPECT_CARDS ? ' (expected ' + EXPECT_CARDS + ')' : ''}`);
		await shot(`layout-${m}`);
	}
	await clickJs(`document.querySelector('.lay button[data-mode="section"]')`);
	await sleep(500);
}

// -- 5. an author bar filters and rewrites the spell.
const barPk = await ev(`document.querySelector('.hist .bar[data-pk]')?.dataset.pk ?? null`);
if (!barPk) {
	skip('author-bar', 'no .hist .bar[data-pk]');
} else {
	const cardsBefore = await count('.abuf .acard');
	await clickJs(`document.querySelector('.hist .bar[data-pk]')`);
	await sleep(900);
	const spell = await ev(`document.querySelector('.spell__input')?.value ?? ''`);
	const on = await exists('.hist .bar.on');
	const cardsAfter = await count('.abuf .acard');
	check('author-bar', spell.includes('by:') && on, `spell "${spell}"; ${cardsBefore} → ${cardsAfter} cards`);
	await shot('author-bar');
	await clickJs(`document.querySelector('.hist .bar.on')`); // toggle back off
	await sleep(700);
	const spellOff = await ev(`document.querySelector('.spell__input')?.value ?? ''`);
	check('author-bar-off', !spellOff.includes('by:'), `spell "${spellOff}"`);
}

// -- 6. free layout: drag the first card by (80, 60) → pinned.
const hasFree = modes.includes?.('free');
if (!hasFree) {
	skip('drag-pin', 'no free layout');
} else {
	await clickJs(`document.querySelector('.lay button[data-mode="free"]')`);
	await sleep(700);
	// Press on the title row: the header's left edge is the → §n chip, which the
	// drag action ignores by contract (button, .acard__anchor, .acard__open).
	const r = await rectOf(`document.querySelector('.abuf .acard .acard__t')`);
	if (!r) skip('drag-pin', 'no .acard in free layout');
	else {
		const before = await ev(`(()=>{const c=document.querySelector('.abuf .acard'); return {id:c.dataset.id, left:c.offsetLeft, top:c.offsetTop}})()`);
		const sx = r.x, sy = r.y;
		await mouse('mouseMoved', sx, sy, { button: 'none' });
		await mouse('mousePressed', sx, sy);
		for (let i = 1; i <= 8; i++) await mouse('mouseMoved', sx + (80 * i) / 8, sy + (60 * i) / 8, { buttons: 1 });
		await mouse('mouseReleased', sx + 80, sy + 60);
		await sleep(700);
		const after = await ev(`(()=>{const c=document.querySelector('.abuf .acard[data-id=${JSON.stringify(before?.id ?? '')}]') || document.querySelector('.abuf .acard'); return {pinned:c.classList.contains('pinned'), left:c.offsetLeft, top:c.offsetTop, anyPinned:!!document.querySelector('.abuf .acard.pinned')}})()`);
		check('drag-pin', after.anyPinned, `pinned=${after.pinned}; (${before.left},${before.top}) → (${after.left},${after.top})`);
		await shot('free-pinned');
	}
	await clickJs(`document.querySelector('.lay button[data-mode="section"]')`);
	await sleep(500);
}

// -- 7. hover a card → its section lights in the reader. A front-matter card
// anchors to the root, which no section matches — pick one with a §n chip.
{
	const r = await rectOf(`[...document.querySelectorAll('.abuf .acard')].find((c) => /§\\d/.test(c.querySelector('.acard__anchor')?.textContent ?? ''))`);
	if (!r) skip('hover-anchor', 'no .acard');
	else {
		await mouse('mouseMoved', r.x, r.y, { button: 'none' });
		await sleep(600);
		const hot = await count('.pane__body--reader .section.anchor-hot');
		const tip = await ev(`document.querySelector('.pane__body--reader .anchor-tip')?.textContent.trim() ?? null`);
		check('hover-anchor', hot > 0, `${hot} .section.anchor-hot; tip ${tip ? JSON.stringify(tip) : 'none'}`);
		await shot('hover');
		await mouse('mouseMoved', 5, 5, { button: 'none' });
		await sleep(300);
	}
}

// -- 8. the §n chip scrolls the reader: top the reader, click the anchor
//    with the largest §n, compare the reader scroll container's scrollTop.
{
	const scroller = `(()=>{const body=document.querySelector('.pane__body--reader'); if(!body) return null; return [body, ...body.querySelectorAll('*')].find(e=>e.scrollHeight>e.clientHeight+4 && /auto|scroll/.test(getComputedStyle(e).overflowY))})()`;
	await ev(`(()=>{const s=${scroller}; if(s) s.scrollTop=0; return !!s})()`);
	await sleep(300);
	const before = await ev(`(()=>{const s=${scroller}; return s ? s.scrollTop : null})()`);
	const chip = `(()=>{const chips=[...document.querySelectorAll('.abuf .acard__anchor')]; if(!chips.length) return null; chips.sort((a,b)=>(parseInt(b.textContent.replace(/\\D/g,''))||0)-(parseInt(a.textContent.replace(/\\D/g,''))||0)); return chips[0]})()`;
	const label = await ev(`${chip}?.textContent.trim() ?? null`);
	if (!label) skip('anchor-scroll', 'no .acard__anchor');
	else {
		await clickJs(chip);
		await sleep(1200);
		const after = await ev(`(()=>{const s=${scroller}; return s ? s.scrollTop : null})()`);
		check('anchor-scroll', before != null && after != null && after !== before, `chip "${label}": scrollTop ${before} → ${after}`);
		await shot('anchor-scroll');
	}
}

// -- 9. mobile 390×844: the list view (.vseg segments, .alist .grp rows).
await send('Emulation.setDeviceMetricsOverride', { width: 390, height: 844, deviceScaleFactor: 2, mobile: true });
await sleep(1500);
let vseg = await exists('.vseg');
if (!vseg) {
	// the mobile shell shows one work buffer at a time — find the table in the ☰ drawer
	await clickJs(byText('button', /^☰$|menu/i));
	await sleep(700);
	await clickJs(byText('button, [role=button], li', /annotations/i));
	await sleep(1200);
	vseg = await exists('.vseg');
}
const grp = await count('.alist .grp');
const mobileRows = await count('.alist .acard.m');
const vsegModes = (await ev(`[...document.querySelectorAll('.vseg button[data-mode]')].map(b=>b.dataset.mode)`)) ?? [];
check('mobile-list', vseg && grp > 0, `.vseg=${vseg} (${Array.isArray(vsegModes) ? vsegModes.join('·') : '?'}); ${grp} .alist .grp, ${mobileRows} rows`);
if (Array.isArray(vsegModes) && vsegModes.length) check('mobile-no-free', !vsegModes.includes('free'), vsegModes.join('·'));
await shot('mobile');
if (!KEEP_MOBILE) await send('Emulation.setDeviceMetricsOverride', { width: 1280, height: 900, deviceScaleFactor: 1, mobile: false });

// ---------------------------------------------------------------- summary

const failed = results.filter((r) => !r.ok);
console.log(`\n${results.length - failed.length}/${results.length} checks passed; screenshots in ${OUT}`);
ws.close();
process.exit(failed.length ? 1 : 0);
