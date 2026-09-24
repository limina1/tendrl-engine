#!/usr/bin/env node
// Annotations ledger — CDP verification harness (Track F of
// docs/annotations-ledger-contract.org). Self-contained: Node >= 22 (native
// WebSocket + fetch), no dependencies.
//
// Assumes:
//   - a scratch engine on $PORT (default 3131) seeded by
//     scripts/verify/annotations-ledger-seed.sh (the "Annotations ledger —
//     verify" recipe in docs/commands.org), network mode auto, the OWNER
//     logged in with the engine signing source;
//   - chromium --headless=new --remote-debugging-port=9333 already running
//     (any page; the harness navigates it).
//
// Usage:
//   node scripts/verify/annotations-ledger.mjs --root 30040:<pk>:<d> \
//        [--search 'id:<index event id>'] [--port 3131] [--cdp 9333] \
//        [--out /tmp/ledger-shots] [--keep-mobile]
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
const OUT = opt('out', process.env.OUT ?? '/tmp/annotations-ledger-shots');
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
const waitGone = async (sel, tries = 30, every = 300) => { for (let i = 0; i < tries; i++) { if (!(await exists(sel))) return true; await sleep(every); } return false; };
/** Set a form control's value the way Svelte's bind sees it (the
 *  prototype setter + an input event); `sel` may resolve to an input,
 *  a textarea or a select. */
const typeInto = (js, value) => ev(`(()=>{const e=(${js}); if(!e) return false; const proto=e.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:e.tagName==='SELECT'?HTMLSelectElement.prototype:HTMLInputElement.prototype; const s=Object.getOwnPropertyDescriptor(proto,'value').set; s.call(e,${JSON.stringify(value)}); e.dispatchEvent(new Event('input',{bubbles:true})); e.dispatchEvent(new Event('change',{bubbles:true})); return true})()`);
const text = (js) => ev(`(${js})?.textContent.replace(/\\s+/g,' ').trim() ?? null`);
let shotN = 0;
const shot = async (name) => {
	const r = await send('Page.captureScreenshot', { format: 'png' });
	if (!r.result?.data) return;
	const file = path.join(OUT, `${String(++shotN).padStart(2, '0')}-${name}.png`);
	fs.writeFileSync(file, Buffer.from(r.result.data, 'base64'));
};
/** Expand a ledger row by id (click its .row unless already .x); returns
 *  whether an .xb follows it. */
const expandRow = async (rowJs) => {
	const isOpen = await ev(`(${rowJs})?.classList.contains('x') ?? null`);
	if (isOpen === null) return false;
	if (!isOpen) { await clickJs(rowJs); await sleep(600); }
	return ev(`(()=>{const r=(${rowJs}); if(!r) return false; const n=r.nextElementSibling; return !!(n && n.classList.contains('xb')) || !!document.querySelector('.detail .xb')})()`);
};
/** The .xb that belongs to a row: inline (next sibling) or the detail pane. */
const xbOf = (rowJs) => `(()=>{const r=(${rowJs}); if(!r) return null; const n=r.nextElementSibling; return (n && n.classList.contains('xb')) ? n : document.querySelector('.detail .xb')})()`;

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

// -- 1. first-run modals: network-mode choice (walkthrough off, confirm),
//    any fetch-confirm sheet (cancel), discovery tips (skip).
const modal = await ev(`(()=>{const m=document.querySelector('.nm-modal'); if(!m) return 'none'; const w=[...m.querySelectorAll('button')].find(b=>/walkthrough/i.test(b.textContent)); if(w && /✓/.test(w.textContent)) w.click(); const c=[...m.querySelectorAll('button')].find(b=>/^confirm/i.test(b.textContent.trim())); if(c){c.click(); return 'confirmed'} return 'no confirm button'})()`);
await sleep(800);
await ev(`(()=>{document.querySelectorAll('.rf-action--ghost, .rf-close').forEach(b=>b.click()); document.querySelectorAll('.dt-skip').forEach(b=>b.click());})()`);
await sleep(500);
check('app-open', (await ev(`!!document.querySelector('.rail--research') || !!document.querySelector('.mb-bar, .mobile-bar')`)), `network-mode modal: ${modal}`);
await shot('app');

// -- 2. the seeded publication in the reader: search → result → event menu
//    → "Read publication"; the scroll view so every section is on the page.
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
await shot('reader');

// -- 3. Tools → table N → the split: reader | ledger.
const toolsOpened = await clickJs(`document.querySelector('button[title^="Tools"]')`);
await sleep(700);
const tableBtn = await text(`document.querySelector('.tools-sheet__row button.ins-table')`);
if (!tableBtn) {
	skip('table-button', toolsOpened ? 'no .tools-sheet__row button.ins-table' : 'Tools sheet did not open');
} else {
	check('table-button', /table/.test(tableBtn), `label "${tableBtn}"`);
	await clickJs(`document.querySelector('.tools-sheet__row button.ins-table')`);
	await sleep(1500);
}
const readerStill = await exists('.pane__body--reader');
const ledgerPane = await exists('.pane__body--annotations .abuf');
const abufRoot = await ev(`document.querySelector('.abuf')?.getAttribute('data-root') ?? null`);
const rootOk = !!abufRoot && (!ROOT || abufRoot === ROOT);
check('table-split', readerStill && ledgerPane && rootOk, `reader pane ${readerStill}, ledger pane ${ledgerPane}; .abuf[data-root]=${abufRoot ?? 'none'}${ROOT && abufRoot && abufRoot !== ROOT ? ' (expected ' + ROOT + ')' : ''}`);
await waitFor('.abuf .row', 30, 400);
await shot('ledger-open');

// -- 4. groups per grouping: .gh count for section / author / kind / time.
const groupsAvail = (await ev(`[...document.querySelectorAll('.seg--group button[data-group]')].map(b=>b.dataset.group)`)) ?? [];
if (!Array.isArray(groupsAvail) || groupsAvail.length === 0) {
	skip('groups', 'no .seg--group button[data-group]');
} else {
	check('group-modes', ['section', 'author', 'kind', 'time'].every((g) => groupsAvail.includes(g)), groupsAvail.join(' · '));
	for (const g of ['section', 'author', 'kind', 'time']) {
		if (!groupsAvail.includes(g)) continue;
		await clickJs(`document.querySelector('.seg--group button[data-group="${g}"]')`);
		await sleep(600);
		const gh = await count('.abuf .gh');
		const rows = await count('.abuf .row');
		const labels = await ev(`[...document.querySelectorAll('.abuf .gh')].map(e=>e.textContent.replace(/\\s+/g,' ').trim()).slice(0,6)`);
		check(`group-${g}`, gh > 0 && rows > 0, `${gh} groups, ${rows} rows · ${(labels ?? []).join(' | ')}`);
		await shot(`group-${g}`);
	}
	await clickJs(`document.querySelector('.seg--group button[data-group="section"]')`);
	await sleep(500);
}
const totalRows = await count('.abuf .row');
const childRows = await count('.abuf .row.child');
check('answers-nest', childRows > 0, `${childRows} .row.child of ${totalRows} rows (section grouping)`);

// -- 5. a who-bar filters the rows and rewrites the spell; a second click clears it.
const barPk = await ev(`document.querySelector('.facet--who .bar[data-pk]')?.dataset.pk ?? null`);
if (!barPk) {
	skip('facet-by', 'no .facet--who .bar[data-pk]');
} else {
	const rowsBefore = await count('.abuf .row');
	await clickJs(`document.querySelector('.facet--who .bar[data-pk]')`);
	await sleep(900);
	const spell = await ev(`document.querySelector('.spell__input')?.value ?? ''`);
	const on = await exists('.facet--who .bar.on');
	const rowsAfter = await count('.abuf .row');
	check('facet-by', spell.includes('by:') && on && rowsAfter <= rowsBefore, `spell "${spell}"; ${rowsBefore} → ${rowsAfter} rows`);
	await shot('facet-by');
	await clickJs(`document.querySelector('.facet--who .bar.on')`);
	await sleep(700);
	const spellOff = await ev(`document.querySelector('.spell__input')?.value ?? ''`);
	const rowsOff = await count('.abuf .row');
	check('facet-off', !spellOff.includes('by:') && rowsOff === rowsBefore, `spell "${spellOff}"; ${rowsOff} rows`);
}

// -- 6. a row opens in place → .xb with the thread; sign a reply → the
//    thread grows and the row's "N ↩" counts it. The row: the owner's §1
//    note, which the seed gave two kind-1111 replies.
const noteRow = `[...document.querySelectorAll('.abuf .row[data-kind="note"]')].find(r=>r.querySelector('.row__sub--rep')) || document.querySelector('.abuf .row[data-kind="note"]')`;
{
	const opened = await expandRow(noteRow);
	check('row-expand', opened, opened ? `.xb under ${await text(`(${noteRow})?.querySelector('.row__t')`)}` : 'no .xb after clicking a .row[data-kind=note]');
	const thread = await waitFor('.xb .thread', 20, 300);
	const repliesBefore = await count('.xb .thread .reply');
	check('thread-visible', thread, `${repliesBefore} .reply rows in .xb .thread`);
	await shot('row-thread');
	const typedReply = await typeInto(`document.querySelector('.xb .replybox textarea')`, 'A reply from the harness — the annotation itself is untouched.');
	await clickJs(`document.querySelector('.xb .replybox__sign')`);
	await sleep(2500);
	const repliesAfter = await count('.xb .thread .reply');
	const rep = await text(`(${noteRow})?.querySelector('.row__sub--rep')`);
	check('reply-sign', typedReply && repliesAfter > repliesBefore, `${repliesBefore} → ${repliesAfter} .reply; row "${rep ?? '—'}"`);
	await shot('reply-signed');
}

// -- 7. answer the open ask by hand: .act--answer → the annotate window in
//    answer mode (fixed title) → type → sign → the window closes and the
//    note nests under the ask.
const askRow = `[...document.querySelectorAll('.abuf .row[data-kind="request"]')].find(r=>/\\bask\\b/i.test(r.querySelector('.badge')?.textContent ?? '') && r.querySelector('.flag.wait'))`;
{
	const askId = await ev(`(${askRow})?.dataset.id ?? null`);
	if (!askId) skip('answer-ask', 'no open ask row (.row[data-kind=request] with .flag.wait and an "ask" badge)');
	else {
		await expandRow(askRow);
		const btn = await clickJs(`${xbOf(askRow)}?.querySelector('.act--answer')`);
		await sleep(900);
		const fixed = await waitFor('.rq-modal .rq-term__fixed', 15, 300);
		check('answer-ask-window', btn && fixed, `.rq-term__fixed ${fixed}`);
		await typeInto(`document.querySelector('.rq-modal .rq-def')`, 'Feyerabend, Against Method (1975), ch. 3 — paraphrasing a condition he attributes to the logical empiricists.');
		await sleep(300);
		await shot('answer-ask');
		await clickJs(`document.querySelector('.rq-modal .rq-btn--sign')`);
		const closed = await waitGone('.rq-modal', 30, 300);
		await sleep(1500);
		const child = await ev(`(()=>{const r=(${askRow}) || document.querySelector('.abuf .row[data-id=${JSON.stringify(askId)}]'); if(!r) return null; let n=r.nextElementSibling; while(n && n.classList.contains('xb')) n=n.nextElementSibling; return !!(n && n.classList.contains('child'))})()`);
		const stillWaiting = await ev(`!!document.querySelector('.abuf .row[data-id=${JSON.stringify(askId)}] .flag.wait')`);
		check('answer-ask', closed && child === true && !stillWaiting, `window closed ${closed}; .row.child under the ask ${child}; waiting flag gone ${!stillWaiting}`);
		await shot('answer-ask-done');
	}
}

// -- 8. define a term by hand on the ANSWERED vocabulary request: the term
//    picker, "+ another term…", type, sign → closes → a new def row.
const vocabRow = `[...document.querySelectorAll('.abuf .row[data-kind="request"]')].find(r=>/vocabulary/i.test(r.querySelector('.badge')?.textContent ?? ''))`;
{
	const vid = await ev(`(${vocabRow})?.dataset.id ?? null`);
	if (!vid) skip('define-term', 'no vocabulary request row');
	else {
		const defsBefore = await count('.abuf .row[data-kind="def"]');
		await expandRow(vocabRow);
		const label = await text(`${xbOf(vocabRow)}?.querySelector('.act--answer')`);
		const btn = await clickJs(`${xbOf(vocabRow)}?.querySelector('.act--answer')`);
		await sleep(900);
		const pick = await waitFor('.rq-modal select.rq-term__pick', 15, 300);
		check('define-term-window', btn && pick, `button "${label ?? '—'}"; .rq-term__pick ${pick}`);
		// choose "+ another term…" (the last option) → the free input
		const picked = await ev(`(()=>{const s=document.querySelector('.rq-modal select.rq-term__pick'); if(!s) return false; const o=[...s.options].find(o=>/another term/i.test(o.textContent)); if(!o) return false; s.value=o.value; s.dispatchEvent(new Event('change',{bubbles:true})); return true})()`);
		await sleep(500);
		const newInput = await waitFor('.rq-modal .rq-term__new', 10, 200);
		await typeInto(`document.querySelector('.rq-modal .rq-term__new')`, 'binding');
		await typeInto(`document.querySelector('.rq-modal .rq-def')`, 'Binding, as used here: a condition the method of the day treats as not open to revision.');
		await sleep(300);
		await shot('define-term');
		await clickJs(`document.querySelector('.rq-modal .rq-btn--sign')`);
		const closed = await waitGone('.rq-modal', 30, 300);
		await sleep(1500);
		const defsAfter = await count('.abuf .row[data-kind="def"]');
		check('define-term', picked && newInput && closed && defsAfter > defsBefore, `"+ another term…" ${picked}; input ${newInput}; closed ${closed}; def rows ${defsBefore} → ${defsAfter}`);
		await shot('define-term-done');
	}
}

// -- 9. a def row's relink opens the diff modal (the existing RelinkModal).
{
	const defRow = `document.querySelector('.abuf .row[data-kind="def"]')`;
	const did = await ev(`(${defRow})?.dataset.id ?? null`);
	if (!did) skip('relink-opens', 'no def row');
	else {
		await expandRow(defRow);
		const btn = await clickJs(`${xbOf(defRow)}?.querySelector('.act--relink')`);
		await sleep(1200);
		const rl = await waitFor('.rl-modal', 20, 300);
		check('relink-opens', btn && rl, `.rl-modal ${rl}`);
		await shot('relink');
		await send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Escape', code: 'Escape' });
		await send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Escape', code: 'Escape' });
		await sleep(500);
		if (await exists('.rl-modal')) await clickJs(byText('.rl-modal button', /^cancel$|^close$|^×$/i));
		await sleep(400);
		check('relink-closes', !(await exists('.rl-modal')), '');
	}
}

// -- 10. the ⑂ row: review the diff → the review takes the reader's leaf;
//     "← reader" restores it.
{
	const mrRow = `document.querySelector('.abuf .row[data-kind="mr"]')`;
	const mid = await ev(`(${mrRow})?.dataset.id ?? null`);
	if (!mid) skip('review-in-reader-leaf', 'no ⑂ row (.row[data-kind=mr]) — did the seed produce an 818?');
	else {
		await expandRow(mrRow);
		const btn = await clickJs(`${xbOf(mrRow)}?.querySelector('.act--review')`);
		await sleep(2500);
		const mr = await waitFor('.mr[data-mr]', 20, 300);
		const readerGone = !(await exists('.pane__body--reader'));
		const ledgerStill = await exists('.abuf');
		check('review-in-reader-leaf', btn && mr && readerGone && ledgerStill, `.mr ${mr}; reader pane gone ${readerGone}; ledger still there ${ledgerStill}`);
		const parts = await ev(`['.rv__status','.rv__ask','.rv__sum','.rv__defs','.rv__sec','.rvfoot'].map(s=>s+':'+document.querySelectorAll(s).length).join(' ')`);
		check('review-parts', (await exists('.rv__sum')) && (await exists('.rvfoot')), parts ?? '');
		await shot('review');
		const back = await clickJs(`document.querySelector('.pane-head__back')`);
		await sleep(1500);
		const readerBack = (await exists('.pane__body--reader')) || (await exists('.rd-content'));
		check('review-back', back && readerBack, `← reader clicked ${back}; reader back ${readerBack}`);
	}
}

// -- 11. the reader strip: stubs per section; a stub opens its card; the
//     card's "≡ row" reveals the ledger row.
{
	const stubs = await count('.pane__body--reader .strip .stub');
	const perSection = await ev(`[...document.querySelectorAll('.pane__body--reader .section, .pane__body--reader .continuous-section')].map(s=>s.querySelectorAll('.strip .stub').length)`);
	check('reader-strip', stubs > 0 && Array.isArray(perSection) && perSection.some((n) => n >= 1), `${stubs} .stub; per section ${(perSection ?? []).join('/')}`);
	// a note stub (its card is an .icard with the "≡ row" button)
	const noteStub = `[...document.querySelectorAll('.pane__body--reader .strip .stub')].find(s=>s.querySelector('.stub__g.note')) || document.querySelector('.pane__body--reader .strip .stub')`;
	const sid = await ev(`(${noteStub})?.dataset.id ?? null`);
	await clickJs(noteStub);
	await sleep(900);
	const cardOpen = await ev(`(()=>{const s=(${noteStub}); if(!s) return false; const strip=s.closest('.strip'); let n=strip?.nextElementSibling; return !!(n && (n.matches('.icard, .rcard') || n.querySelector('.icard, .rcard'))) || !!s.closest('.section, .continuous-section')?.querySelector('.icard, .rcard')})()`);
	check('stub-opens-card', cardOpen && (await exists('.pane__body--reader .stub.on')), `stub ${sid ?? '—'} → card ${cardOpen}`);
	await shot('reader-stub');
	const rowBtn = await clickJs(`document.querySelector('.pane__body--reader .icard__row, .pane__body--reader .rcard__row')`);
	await sleep(1200);
	const revealed = await ev(`(()=>{const x=document.querySelector('.abuf .row.x'); return x ? x.dataset.id : null})()`);
	check('row-reveal', rowBtn && !!revealed && (!sid || revealed === sid), `≡ row → .row.x[data-id=${revealed ?? 'none'}]${sid && revealed && revealed !== sid ? ' (expected ' + sid + ')' : ''}`);
}

// -- 12. hover a row → its section lights in the reader.
{
	const r = await rectOf(`[...document.querySelectorAll('.abuf .row')].find(r=>/§\\d/.test(r.querySelector('.row__anc')?.textContent ?? ''))`);
	if (!r) skip('hover-anchor', 'no .row with a §n cell');
	else {
		await mouse('mouseMoved', r.x, r.y, { button: 'none' });
		await sleep(600);
		const hot = await count('.pane__body--reader .section.anchor-hot');
		const tip = await text(`document.querySelector('.pane__body--reader .anchor-tip')`);
		check('hover-anchor', hot > 0, `${hot} .section.anchor-hot; tip ${tip ? JSON.stringify(tip) : 'none'}`);
		await shot('hover');
		await mouse('mouseMoved', 5, 5, { button: 'none' });
		await sleep(300);
	}
}

// -- 13. the § cell scrolls the reader: top the reader, click the largest
//     §n, compare the reader scroll container's scrollTop.
{
	const scroller = `(()=>{const body=document.querySelector('.pane__body--reader'); if(!body) return null; return [body, ...body.querySelectorAll('*')].find(e=>e.scrollHeight>e.clientHeight+4 && /auto|scroll/.test(getComputedStyle(e).overflowY))})()`;
	await ev(`(()=>{const s=${scroller}; if(s) s.scrollTop=0; return !!s})()`);
	await sleep(300);
	const before = await ev(`(()=>{const s=${scroller}; return s ? s.scrollTop : null})()`);
	const anc = `(()=>{const cells=[...document.querySelectorAll('.abuf .row__anc')]; if(!cells.length) return null; cells.sort((a,b)=>(parseInt(b.textContent.replace(/\\D/g,''))||0)-(parseInt(a.textContent.replace(/\\D/g,''))||0)); return cells[0]})()`;
	const label = await text(anc);
	if (!label) skip('anchor-scroll', 'no .row__anc');
	else {
		await clickJs(anc);
		await sleep(1200);
		const after = await ev(`(()=>{const s=${scroller}; return s ? s.scrollTop : null})()`);
		check('anchor-scroll', before != null && after != null && after !== before, `cell "${label}": scrollTop ${before} → ${after}`);
		await shot('anchor-scroll');
	}
}

// -- 14. mobile 390×844: the same rows in .abuf.phone, no board segment,
//     facets scrolling sideways.
await send('Emulation.setDeviceMetricsOverride', { width: 390, height: 844, deviceScaleFactor: 2, mobile: true });
await sleep(1500);
let phone = await exists('.abuf.phone');
if (!phone) {
	// the mobile shell shows one work buffer at a time — find the ledger in the ☰ drawer
	await clickJs(byText('button', /^☰$|menu/i));
	await sleep(700);
	await clickJs(byText('button, [role=button], li', /annotations/i));
	await sleep(1200);
	phone = await exists('.abuf.phone');
}
const phoneRows = await count('.abuf.phone .row');
const noBoard = !(await exists('.abuf.phone .seg--view'));
const facetsScroll = await ev(`(()=>{const f=document.querySelector('.abuf.phone .facets'); if(!f) return null; const cs=getComputedStyle(f); return {overflow:cs.overflowX, wider:f.scrollWidth>f.clientWidth}})()`);
check('mobile', phone && phoneRows > 0 && noBoard, `.abuf.phone ${phone}; ${phoneRows} rows; no .seg--view ${noBoard}`);
check('mobile-facets-scroll', !!facetsScroll && (/auto|scroll/.test(facetsScroll.overflow) || facetsScroll.wider), facetsScroll ? `overflow-x ${facetsScroll.overflow}, wider ${facetsScroll.wider}` : 'no .facets');
await shot('mobile');
if (!KEEP_MOBILE) await send('Emulation.setDeviceMetricsOverride', { width: 1280, height: 900, deviceScaleFactor: 1, mobile: false });

// ---------------------------------------------------------------- summary

const failed = results.filter((r) => !r.ok);
console.log(`\n${results.length - failed.length}/${results.length} checks passed; screenshots in ${OUT}`);
ws.close();
process.exit(failed.length ? 1 : 0);
