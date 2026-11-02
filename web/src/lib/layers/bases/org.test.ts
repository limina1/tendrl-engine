import { describe, expect, it } from 'vitest';
import { base } from './org';
import { classesAt, covered, wellFormed } from './test-util';

const spans = (t: string) => base.spans(t);

describe('org base — Orgdown OD1', () => {
	it('headings: level from the star run, stars dimmed, title inline-scanned', () => {
		const t = '* Top\n** Second *bold* here\n******* deep';
		const s = spans(t);
		expect(covered(t, s, 'h1')).toEqual(['* Top']);
		expect(covered(t, s, 'h2')).toEqual(['** Second *bold* here']);
		expect(covered(t, s, 'h6')).toEqual(['******* deep']);
		expect(covered(t, s, 'mark')).toContain('*');
		expect(covered(t, s, 'mark')).toContain('**');
		expect(covered(t, s, 'strong')).toEqual(['*bold*']);
		expect(wellFormed(t, s)).toBe(true);
	});

	it('headings: TODO keyword, priority cookie and tags', () => {
		const t = '* TODO [#A] Write the thing :work:urgent:';
		const s = spans(t);
		expect(covered(t, s, 'todo')).toEqual(['TODO']);
		expect(covered(t, s, 'mark')).toContain('[#A]');
		expect(covered(t, s, 'tag')).toEqual([':work:urgent:']);
	});

	it('inline emphasis with Org boundary rules', () => {
		const t = 'a *bold* /it/ _ul_ =verb= ~code~ +gone+ not*mid*word (*p*) 2*3*4';
		const s = spans(t);
		expect(covered(t, s, 'strong')).toEqual(['*bold*', '*p*']);
		expect(covered(t, s, 'em')).toEqual(['/it/']);
		expect(covered(t, s, 'underline')).toEqual(['_ul_']);
		expect(covered(t, s, 'code')).toEqual(['=verb=', '~code~']);
		expect(covered(t, s, 'strike')).toEqual(['+gone+']);
	});

	it('verbatim bodies are opaque to other markers', () => {
		const t = 'see =a *b* c= and *d*';
		const s = spans(t);
		expect(covered(t, s, 'strong')).toEqual(['*d*']);
	});

	it('links: [[target][desc]] and [[target]]', () => {
		const t = 'go [[https://x.org][site]] or [[Hayek]] now';
		const s = spans(t);
		expect(covered(t, s, 'link')).toEqual(['[[https://x.org][site]]', '[[Hayek]]']);
		expect(covered(t, s, 'url')).toEqual(['https://x.org']);
		expect(covered(t, s, 'mark')).toEqual(['[[', '][', ']]', '[[', ']]']);
	});

	it('a {{ }} nostrdown token never pairs an emphasis marker across itself', () => {
		const t = 'x *{{ref:a-b|y}} z* and *ok*';
		const s = spans(t);
		// The first `*` sits before an opaque token: the only strong run is *ok*
		// unless the pairing legitimately spans around the token — it does here
		// (`*{{…}} z*`), and that is fine: the atom keeps its extent and only
		// picks up the class. What must NOT happen is a marker INSIDE the token
		// pairing with one outside.
		const strong = covered(t, s, 'strong');
		expect(strong).toContain('*ok*');
		for (const run of strong) expect(run.startsWith('*{{') || run === '*ok*').toBe(true);
		const t2 = 'a {{ref:x|*b}} c* d';
		expect(covered(t2, spans(t2), 'strong')).toEqual([]);
	});

	it('src blocks are fences with a language; quote blocks are quotes', () => {
		const t = '#+BEGIN_SRC rust\nfn *x*() {}\n#+END_SRC\n#+begin_quote\n*q*\n#+end_quote';
		const s = spans(t);
		expect(covered(t, s, 'fence')).toEqual(['#+BEGIN_SRC rust\nfn *x*() {}\n#+END_SRC']);
		expect(covered(t, s, 'lang')).toEqual(['rust']);
		expect(covered(t, s, 'quote')).toEqual(['#+begin_quote\n*q*\n#+end_quote']);
		// No emphasis inside the fence; emphasis still runs inside the quote.
		expect(covered(t, s, 'strong')).toEqual(['*q*']);
	});

	it('an unterminated block runs to the end', () => {
		const t = 'p\n#+BEGIN_EXAMPLE\nraw *x*\nmore';
		const s = spans(t);
		expect(covered(t, s, 'fence')).toEqual(['#+BEGIN_EXAMPLE\nraw *x*\nmore']);
		expect(covered(t, s, 'strong')).toEqual([]);
	});

	it('keywords, comments, rules, fixed width', () => {
		const t = '#+TITLE: T\n# a comment\n-----\n: fixed';
		const s = spans(t);
		expect(covered(t, s, 'kw')).toEqual(['#+TITLE: T']);
		expect(covered(t, s, 'comment')).toEqual(['# a comment']);
		expect(covered(t, s, 'hr')).toEqual(['-----']);
		expect(covered(t, s, 'code')).toEqual([': fixed']);
	});

	it('lists: bullets, checkboxes, description terms', () => {
		const t = '- one\n  + [X] done *now*\n1. third\n- term :: def';
		const s = spans(t);
		expect(covered(t, s, 'mark')).toEqual(expect.arrayContaining(['-', '+', '[X]', '1.', '::']));
		expect(covered(t, s, 'strong')).toEqual(['*now*', 'term']);
	});

	it('tables: rows, rule, header before a rule', () => {
		const t = '| a | b |\n|---+---|\n| 1 | 2 |';
		const s = spans(t);
		expect(covered(t, s, 'table').length).toBe(3);
		expect(covered(t, s, 'th')).toEqual(['| a | b |']);
		expect(classesAt(s, t.indexOf('|---'))).toContain('mark');
	});

	it('offsets survive multi-byte text (UTF-16 basis)', () => {
		const t = '* Été — *gras* 🙂 /ok/';
		const s = spans(t);
		expect(covered(t, s, 'strong')).toEqual(['*gras*']);
		expect(covered(t, s, 'em')).toEqual(['/ok/']);
		expect(wellFormed(t, s)).toBe(true);
	});
});
