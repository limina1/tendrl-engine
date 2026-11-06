import { describe, expect, it } from 'vitest';
import { base } from './asciidoc';
import { classesAt, covered, wellFormed } from './test-util';

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
			'xref:sec[S]',
			'image::p.png[alt]',
			'<<sec,there>>'
		]);
		expect(covered(t, s, 'url')).toEqual(['//x.org', 'a.html', 'sec', 'p.png', 'https://bare.org/p']);
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
});
