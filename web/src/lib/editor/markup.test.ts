import { describe, expect, it } from 'vitest';
import {
	codeBlock,
	headingLines,
	linkText,
	parseHeading,
	quoteBlock,
	setHeadingLevel,
	toggleInline,
	toggleList
} from './markup';

describe('headings per style', () => {
	it('reads each style’s own heading syntax', () => {
		expect(parseHeading('## Methods', 'text/markdown')).toEqual({ level: 2, title: 'Methods' });
		expect(parseHeading('** Methods', 'text/org')).toEqual({ level: 2, title: 'Methods' });
		expect(parseHeading('== Methods', 'text/asciidoc')).toEqual({ level: 2, title: 'Methods' });
		expect(parseHeading('== Methods', 'text/markdown')).toBeNull();
		expect(parseHeading('*bold* text', 'text/org')).toBeNull();
		expect(parseHeading('## ', 'text/markdown')).toEqual({ level: 2, title: '' });
		expect(parseHeading('##', 'text/markdown')).toBeNull();
	});

	it('skips heading-looking lines inside fences', () => {
		const md = '# T\n\n## A\n```\n## not a heading\n```\n## B';
		expect(headingLines(md, 'text/markdown').map((h) => h.title)).toEqual(['T', 'A', 'B']);
		const org = '* T\n#+begin_src sh\n** nope\n#+end_src\n** A';
		expect(headingLines(org, 'text/org').map((h) => h.title)).toEqual(['T', 'A']);
		const adoc = '= T\n----\n== nope\n----\n== A';
		expect(headingLines(adoc, 'text/asciidoc').map((h) => h.title)).toEqual(['T', 'A']);
	});

	it('sets, changes and clears a heading level', () => {
		expect(setHeadingLevel('Methods', 'text/markdown', 2)).toBe('## Methods');
		expect(setHeadingLevel('## Methods', 'text/markdown', 3)).toBe('### Methods');
		expect(setHeadingLevel('## Methods', 'text/markdown', 2)).toBe('Methods');
		expect(setHeadingLevel('Methods', 'text/org', 1)).toBe('* Methods');
	});
});

describe('toolbar transforms', () => {
	it('wraps and unwraps inline marks', () => {
		expect(toggleInline('word', '', '', 'text/markdown', 'bold').text).toBe('**word**');
		expect(toggleInline('**word**', '', '', 'text/markdown', 'bold').text).toBe('word');
		expect(toggleInline('word', 'a **', '** b', 'text/markdown', 'bold').unwrapOuter).toBe(true);
		expect(toggleInline('word', '', '', 'text/org', 'italic').text).toBe('/word/');
		expect(toggleInline('word', '', '', 'text/asciidoc', 'italic').text).toBe('_word_');
	});

	it('writes links, lists, quotes and code blocks in the style', () => {
		expect(linkText('text/markdown', 'site', 'https://x.org')).toBe('[site](https://x.org)');
		expect(linkText('text/org', 'site', 'https://x.org')).toBe('[[https://x.org][site]]');
		expect(linkText('text/asciidoc', 'site', 'https://x.org')).toBe('https://x.org[site]');
		expect(toggleList(['a', 'b'], 'text/asciidoc')).toEqual(['* a', '* b']);
		expect(toggleList(['- a', '- b'], 'text/markdown')).toEqual(['a', 'b']);
		expect(quoteBlock('q', 'text/markdown')).toBe('> q');
		expect(quoteBlock('q', 'text/org')).toBe('#+begin_quote\nq\n#+end_quote');
		expect(codeBlock('x', 'text/asciidoc', 'sh')).toBe('[source,sh]\n----\nx\n----');
		expect(codeBlock('x', 'text/markdown')).toBe('```\nx\n```');
	});
});
