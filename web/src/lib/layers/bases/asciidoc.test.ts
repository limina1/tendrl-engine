import { describe, expect, it } from 'vitest';
import { base } from './asciidoc';
import { classesAt, covered, wellFormed } from './test-util';
import { hiddenMarkerSpans } from '../hide-markers';

const spans = (t: string) => base.spans(t);

describe('asciidoc base', () => {
	it('section titles by = run (and markdown-style #)', () => {
		const t = '= Doc\n== Part *one*\n#### four\n=== three';
		const s = spans(t);
		expect(covered(t, s, 'h1')).toEqual(['= Doc']);
		expect(covered(t, s, 'h2')).toEqual(['== Part *one*']);
		expect(covered(t, s, 'h3')).toEqual(['=== three']);
		expect(covered(t, s, 'h4')).toEqual(['#### four']);
		expect(covered(t, s, 'strong')).toEqual(['*one*']);
		expect(wellFormed(t, s)).toBe(true);
	});

	it('constrained and unconstrained emphasis', () => {
		const t = '*b* **B** _i_ __I__ `m` #h# in*side*word (*p*) a`b`c';
		const s = spans(t);
		expect(covered(t, s, 'strong')).toEqual(['*b*', '**B**', '*p*']);
		expect(covered(t, s, 'em')).toEqual(['_i_', '__I__']);
		expect(covered(t, s, 'code')).toEqual(['`m`']);
		expect(covered(t, s, 'hl')).toEqual(['#h#']);
	});

	it('monospace bodies are opaque', () => {
		const t = '`a *b* c` *d*';
		expect(covered(t, spans(t), 'strong')).toEqual(['*d*']);
	});

	it('macros: urls with text, link:, xref:, image::, <<xref>>, bare urls', () => {
		const t =
			'see https://x.org[site] link:a.html[A] xref:sec[S] image::p.png[alt] <<sec,there>> https://bare.org/p.';
		const s = spans(t);
		expect(covered(t, s, 'link')).toEqual([
			'https://x.org[site]',
			'link:a.html[A]',
			'image::p.png[alt]'
		]);
		expect(covered(t, s, 'xref')).toEqual(['xref:sec[S]', '<<sec,there>>']);
		expect(covered(t, s, 'url')).toEqual(['//x.org', 'a.html', 'p.png', 'https://bare.org/p']);
	});

	it('[[anchor]] is styled as a link (nostrdown decides whether it is a wikilink)', () => {
		const t = 'x [[Hayek]] y';
		const s = spans(t);
		expect(covered(t, s, 'link')).toEqual(['[[Hayek]]']);
		expect(covered(t, s, 'mark')).toEqual(['[[', ']]']);
	});

	it('attribute references, but never a {{ }} token', () => {
		const t = '{nbsp} and {{ref:a|b}} and {{wiki:c}}';
		const s = spans(t);
		expect(covered(t, s, 'kw')).toEqual(['{nbsp}']);
		expect(covered(t, s, 'hl')).toEqual([]);
	});

	it('delimited blocks: listing is a fence with [source,lang]; quote; comment; example', () => {
		const t =
			'[source,python]\n----\nx = *1*\n----\n____\n*q*\n____\n////\n*c*\n////\n====\n*e*\n====';
		const s = spans(t);
		expect(covered(t, s, 'fence')).toEqual(['----\nx = *1*\n----']);
		expect(covered(t, s, 'lang')).toEqual(['python']);
		expect(covered(t, s, 'kw')).toEqual(['[source,python]']);
		expect(covered(t, s, 'quote')).toEqual(['____\n*q*\n____']);
		expect(covered(t, s, 'comment')).toEqual(['////\n*c*\n////']);
		// Emphasis runs inside quote + example, not inside listing/comment.
		expect(covered(t, s, 'strong')).toEqual(['*q*', '*e*']);
		expect(covered(t, s, 'mark')).toEqual(
			expect.arrayContaining(['----', '----', '____', '____', '////', '////', '====', '===='])
		);
	});

	it('[stem] + ++++ is a maths fence', () => {
		const t = '[stem]\n++++\n\\sqrt{4}\n++++';
		const s = spans(t);
		expect(covered(t, s, 'fence')).toEqual(['++++\n\\sqrt{4}\n++++']);
		expect(covered(t, s, 'lang')).toEqual(['stem']);
	});

	it('tables: header row, cell separators', () => {
		const t = '|===\n| A | B\n\n| 1 | 2\n|===';
		const s = spans(t);
		expect(covered(t, s, 'table')).toEqual([t]);
		expect(covered(t, s, 'th')).toEqual(['| A | B']);
		expect(classesAt(s, t.indexOf('| 1'))).toContain('mark');
	});

	it('lists, checklist, description, callout, continuation', () => {
		const t = '* one\n** two\n. first\n- [x] done\nterm:: def\n<1> call\n+\nmore';
		const s = spans(t);
		expect(covered(t, s, 'mark')).toEqual(
			expect.arrayContaining(['*', '**', '.', '-', '[x]', '::', '<1>', '+'])
		);
		expect(covered(t, s, 'strong')).toEqual(['term']);
	});

	it('admonitions, block titles, attribute entries, comments, rule, hard break', () => {
		const t = 'NOTE: careful *x*\n.Figure one\n:toc: left\n// hidden\n[NOTE]\n\'\'\'\nline +';
		const s = spans(t);
		expect(covered(t, s, 'admon')).toEqual(['NOTE:', 'NOTE']);
		expect(covered(t, s, 'strong')).toEqual(['*x*', '.Figure one']);
		expect(covered(t, s, 'kw')).toEqual([':toc: left', '[NOTE]']);
		expect(covered(t, s, 'comment')).toEqual(['// hidden']);
		expect(covered(t, s, 'hr')).toEqual(["'''"]);
		expect(classesAt(s, t.length - 1)).toContain('mark');
	});

	it('soft breaks: paragraph newlines collapse; hard breaks, blank lines, blocks, titles do not', () => {
		const t =
			'== T\nline one\nline two +\nline three\nline four\n\nnext para\n* item\n  wrapped\n----\na\nb\n----\n____\nq one\nq two\n____';
		const s = spans(t);
		const at = (needle: string) => t.indexOf(needle) - 1;
		const softs = s.filter((x) => x.cls === 'soft').map((x) => x.start);
		expect(softs).toEqual([at('line two'), at('line four'), at('  wrapped'), at('q two')]);
		expect(softs).not.toContain(at('line one')); // after a title
		expect(softs).not.toContain(at('line three')); // after ` +`
		expect(softs).not.toContain(at('next para')); // across a blank line
		expect(softs).not.toContain(at('b\n')); // inside a listing
		expect(s.filter((x) => x.cls === 'soft').every((x) => /^\n[ \t]*$/.test(t.slice(x.start, x.end)))).toBe(true);
	});

	it('an unterminated listing runs to the end', () => {
		const t = 'p\n----\nraw *x*\nmore';
		const s = spans(t);
		expect(covered(t, s, 'fence')).toEqual(['----\nraw *x*\nmore']);
		expect(covered(t, s, 'strong')).toEqual([]);
	});

	it('footnotes: plain, named, named reference; body scanned on its own', () => {
		const t = 'A claim.footnote:[See *Hume*, p. 3.] Again.footnote:hume[Treatise.] And.footnote:hume[]';
		const s = spans(t);
		expect(covered(t, s, 'fn')).toEqual([
			'footnote:[See *Hume*, p. 3.]',
			'footnote:hume[Treatise.]',
			'footnote:hume[]'
		]);
		expect(covered(t, s, 'fnbody')).toEqual(['See *Hume*, p. 3.', 'Treatise.']);
		expect(covered(t, s, 'fnname')).toEqual(['hume', 'hume']);
		expect(covered(t, s, 'strong')).toEqual(['*Hume*']);
		expect(covered(t, s, 'mark')).toEqual(
			expect.arrayContaining(['footnote:[', 'footnote:hume[', ']'])
		);
		expect(covered(t, s, 'link')).toEqual([]);
		expect(wellFormed(t, s)).toBe(true);
	});

	it('a footnote body keeps an escaped \\] and a nostrdown ref', () => {
		const t = 'x.footnote:[a [b\\] c {{ref:d|e]}} f] tail *t*';
		const s = spans(t);
		expect(covered(t, s, 'fnbody')).toEqual(['a [b\\] c {{ref:d|e]}} f']);
		expect(covered(t, s, 'strong')).toEqual(['*t*']);
		expect(wellFormed(t, s)).toBe(true);
	});

	it('a footnote inside a nostrdown token is not a footnote', () => {
		const t = '{{quote:naddr1x|footnote:[no]}}';
		expect(covered(t, spans(t), 'fn')).toEqual([]);
	});

	it('xrefs: <<id>>, <<id,text>> (comma in text), xref:id[text], endnote ^n^ link', () => {
		const t = 'see <<ch01-intro>>, <<ch02-x,Smith, Jones and Co>> and xref:ch03[the end] or <<n12,^12^>>';
		const s = spans(t);
		expect(covered(t, s, 'xref')).toEqual([
			'<<ch01-intro>>',
			'<<ch02-x,Smith, Jones and Co>>',
			'xref:ch03[the end]',
			'<<n12,^12^>>'
		]);
		expect(covered(t, s, 'xref-id')).toEqual(['ch01-intro', 'ch02-x', 'ch03', 'n12']);
		expect(covered(t, s, 'xref-text')).toEqual(['Smith, Jones and Co', 'the end', '^12^']);
		expect(covered(t, s, 'sup')).toEqual(['12']);
		// Labelled: the id hides behind the text; bare: only the brackets.
		expect(covered(t, s, 'mark')).toEqual(
			expect.arrayContaining(['<<', '>>', '<<ch02-x,', 'xref:ch03[', ']', '<<n12,', '^'])
		);
		expect(covered(t, s, 'link')).toEqual([]);
		expect(wellFormed(t, s)).toBe(true);
	});

	it('<< in prose (spaces, shift operators) is not an xref', () => {
		const t = 'x << 2 >> y and a<<b c>>';
		expect(covered(t, spans(t), 'xref')).toEqual([]);
	});

	it('anchors: inline anchor:id[] is a hidden sigil; [#id] / [id=…] lines add anchor', () => {
		const t = 'in anchor:ch03-r12[]secret\n[#ch03-h1]\n==== Heading\n[quote#q1,Author]\n[id=fig-2]\n[source,rust]';
		const s = spans(t);
		expect(covered(t, s, 'anchor')).toEqual([
			'anchor:ch03-r12[]',
			'[#ch03-h1]',
			'[quote#q1,Author]',
			'[id=fig-2]'
		]);
		expect(covered(t, s, 'kw')).toEqual(['[#ch03-h1]', '[quote#q1,Author]', '[id=fig-2]', '[source,rust]']);
		expect(classesAt(s, t.indexOf('anchor:'))).toEqual(['anchor', 'mark']);
		expect(covered(t, s, 'h4')).toEqual(['==== Heading']);
	});

	it('superscript and subscript; math-ish prose does not match', () => {
		const t = 'E=mc^2^ and H~2~O, 2^10 is 1024, a ^ b ^ c, x~ y~, `a^b^`, \\^no^';
		const s = spans(t);
		expect(covered(t, s, 'sup')).toEqual(['2']);
		expect(covered(t, s, 'sub')).toEqual(['2']);
		expect(covered(t, s, 'code')).toEqual(['`a^b^`']);
		const sup = t.indexOf('^2^');
		expect(classesAt(s, sup)).toContain('mark');
		expect(classesAt(s, sup + 2)).toContain('mark');
	});

	it('sup/sub never pair across a nostrdown token or a url', () => {
		const t = 'a^{{ref:x}}^ https://e.org/~u~ ok';
		const s = spans(t);
		expect(covered(t, s, 'sup')).toEqual([]);
		expect(covered(t, s, 'sub')).toEqual([]);
	});

	it('roles: [.name]#text#, smallcaps, unconstrained ##, id; plain #x# stays hl', () => {
		const t = '[.smallcaps]#Anno Domini# and [.term.x]##in##side, [#k1]#key#, #mark#';
		const s = spans(t);
		expect(covered(t, s, 'role')).toEqual(['[.smallcaps]#Anno Domini#', '[.term.x]##in##']);
		expect(covered(t, s, 'smallcaps')).toEqual(['[.smallcaps]#Anno Domini#']);
		expect(covered(t, s, 'anchor')).toEqual(['[#k1]#key#']);
		expect(covered(t, s, 'hl')).toEqual(['#mark#']);
		expect(covered(t, s, 'mark')).toEqual(
			expect.arrayContaining(['[.smallcaps]#', '#', '[.term.x]##', '##', '[#k1]#'])
		);
	});

	it('emphasis inside a role body; markers never pair outside it', () => {
		const t = '*a [.sc]#b *c*# d*';
		const s = spans(t);
		expect(covered(t, s, 'strong')).toEqual(['*a [.sc]#b *c*# d*', '*c*']);
	});

	it('sidebar and example blocks span the whole block; [NOTE] + ==== is not an example', () => {
		const t = '****\nside *s*\n****\n\n====\nex\n====\n\n[NOTE]\n====\nnote\n====';
		const s = spans(t);
		expect(covered(t, s, 'sidebar')).toEqual(['****\nside *s*\n****']);
		expect(covered(t, s, 'example')).toEqual(['====\nex\n====']);
		expect(covered(t, s, 'strong')).toEqual(['*s*']);
		expect(wellFormed(t, s)).toBe(true);
	});

	it('nested containers each get their class; unterminated sidebar runs to the end', () => {
		const t = '====\n****\nin\n****\n====\n\n****\nopen';
		const s = spans(t);
		expect(covered(t, s, 'example')).toEqual(['====\n****\nin\n****\n====']);
		expect(covered(t, s, 'sidebar')).toEqual(['****\nin\n****', '****\nopen']);
	});

	it('verse: [verse] block and paragraph keep their line breaks; [sidebar] paragraph', () => {
		const t =
			'[verse, Blake]\n____\nTyger Tyger,\nburning bright\n____\n\n[verse]\nIn the forests\nof the night\n\nprose one\nprose two\n\n[sidebar]\naside\nmore';
		const s = spans(t);
		expect(covered(t, s, 'verse')).toEqual([
			'____\nTyger Tyger,\nburning bright\n____',
			'In the forests\nof the night'
		]);
		expect(covered(t, s, 'quote')).toEqual(['____\nTyger Tyger,\nburning bright\n____']);
		expect(covered(t, s, 'sidebar')).toEqual(['aside\nmore']);
		const softs = s.filter((x) => x.cls === 'soft').map((x) => x.start);
		expect(softs).toEqual([t.indexOf('prose two') - 1, t.indexOf('more') - 1]);
	});

	it('tables: a sidebar delimiter inside a table is cell text, not a block', () => {
		const t = '|===\n|a\n****\n|b\n|===\nafter';
		const s = spans(t);
		expect(covered(t, s, 'table')).toEqual(['|===\n|a\n****\n|b\n|===']);
		expect(covered(t, s, 'sidebar')).toEqual([]);
	});

	it('tables: pipes inside nostrdown, wikilink aliases, passthrough and code are not separators', () => {
		const t = '|===\n|{{ref:a|b}} |[[t|alias]] |+x|y+ |`p|q` |\\| esc\n|===';
		const s = spans(t);
		const pipes = s.filter((x) => x.cls === 'mark' && t.slice(x.start, x.end) === '|');
		expect(pipes.length).toBe(5);
		expect(covered(t, s, 'code')).toEqual(['`p|q`']);
	});

	it('tables: cell specs are marked (cellspec) and kept out of the previous cell', () => {
		const t = '|===\n2+|wide |c\n.2+a|tall |x |y\n|z\n|===';
		const s = spans(t);
		expect(covered(t, s, 'cellspec')).toEqual(['2+', '.2+a']);
		expect(covered(t, s, 'mark')).toEqual(expect.arrayContaining(['2+', '.2+a']));
		expect(covered(t, s, 'tr')).toEqual(['2+|wide |c', '.2+a|tall |x |y', '|z']);
	});

	it('tables: a spec needs whitespace or line start before it (a|b is two cells of text)', () => {
		const t = '|===\n|data|2+|x\n|===';
		expect(covered(t, spans(t), 'cellspec')).toEqual([]);
	});

	it('rendered view hides the new markers and leaves the visible text', () => {
		const t = 'a.footnote:[n] <<x,see ^1^>> H~2~O [.smallcaps]#AD# anchor:k[]b';
		const hidden = hiddenMarkerSpans(t, spans(t)).filter((x) => x.cls === 'hide');
		let shown = '';
		for (let i = 0; i < t.length; i++) if (!hidden.some((h) => h.start <= i && i < h.end)) shown += t[i];
		expect(shown).toBe('a.n see 1 H2O AD b');
	});
});
