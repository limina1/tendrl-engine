// The invariant every layer rests on: `applyStyles` only splits text runs
// and attaches classes — the concatenated segment text is still the source,
// nostrdown atoms keep their extent, highlights keep their parts.
import { describe, expect, it } from 'vitest';
import { buildSegments, type ContentSegment, type ResolvedRef } from '$lib/nostr/nostrdown';
import { applyStyles } from './style-segments';
import { base as org } from './bases/org';
import { base as adoc } from './bases/asciidoc';

function flatText(segs: ContentSegment[], content: string): string {
	let out = '';
	for (const s of segs) {
		if (s.type === 'text') out += s.text;
		else if (s.type === 'highlight') out += flatText(s.parts, content);
		else out += content.slice(s.srcStart, s.srcEnd); // atom: its source slice
	}
	return out;
}

function atoms(segs: ContentSegment[]): ContentSegment[] {
	const out: ContentSegment[] = [];
	for (const s of segs) {
		if (s.type === 'highlight') out.push(...atoms(s.parts));
		else if (s.type !== 'text') out.push(s);
	}
	return out;
}

const ref = (content: string, token: string, kind: ResolvedRef['kind'] = 'ref'): ResolvedRef => {
	const start = content.indexOf(token);
	return { kind, start, end: start + token.length, target: 't', label: 'L', found: true };
};

describe('applyStyles invariants', () => {
	it('org: a {{ref}} inside *bold* and a [[topic]] in a heading stay atoms', () => {
		const content = '* Heading with [[Hayek]]\nSome *bold {{ref:a-b|x}} text* and _u_.';
		const refs = [ref(content, '[[Hayek]]', 'wiki'), ref(content, '{{ref:a-b|x}}')];
		const hl = [{ start: 2, end: 30, id: 'h'.repeat(64), pubkey: 'p'.repeat(64) }];
		const before = buildSegments(content, hl, refs);
		const after = applyStyles(before, org.spans(content));
		expect(flatText(after, content)).toBe(content);
		const a = atoms(after);
		expect(a.map((s) => content.slice(s.srcStart, s.srcEnd))).toEqual([
			'[[Hayek]]',
			'{{ref:a-b|x}}'
		]);
		// The wikilink chip sizes with the heading; the ref chip with the bold.
		expect(a[0].styles).toEqual(expect.arrayContaining(['h1', 'link']));
		expect(a[1].styles).toEqual(expect.arrayContaining(['strong']));
		// Every piece carries exact source offsets.
		const walk = (segs: ContentSegment[]) => {
			for (const s of segs) {
				if (s.type === 'text') expect(content.slice(s.srcStart, s.srcEnd)).toBe(s.text);
				if (s.type === 'highlight') walk(s.parts);
			}
		};
		walk(after);
	});

	it('asciidoc: [[anchor]] under a highlight is still the nostrdown atom', () => {
		const content = '== Title\nSee [[Hayek]] and *bold* `code {{wiki:x}}`.';
		const refs = [ref(content, '[[Hayek]]', 'wiki'), ref(content, '{{wiki:x}}', 'wiki')];
		const hl = [{ start: 9, end: 22, id: 'h'.repeat(64), pubkey: 'p'.repeat(64) }];
		const after = applyStyles(buildSegments(content, hl, refs), adoc.spans(content));
		expect(flatText(after, content)).toBe(content);
		expect(atoms(after).map((s) => content.slice(s.srcStart, s.srcEnd))).toEqual([
			'[[Hayek]]',
			'{{wiki:x}}'
		]);
		expect(atoms(after)[1].styles).toContain('code');
	});

	it('no spans → the input array is returned untouched', () => {
		const segs = buildSegments('plain', [], []);
		expect(applyStyles(segs, [])).toBe(segs);
	});

	it('overlapping spans split text at every boundary and never drop text', () => {
		const content = 'abcdefghij';
		const segs = buildSegments(content, [], []);
		const after = applyStyles(segs, [
			{ start: 0, end: 6, cls: 'a' },
			{ start: 3, end: 9, cls: 'b' },
			{ start: 4, end: 5, cls: 'c' }
		]);
		expect(flatText(after, content)).toBe(content);
		const at = (i: number) =>
			(after.find((s) => s.srcStart <= i && i < s.srcEnd)?.styles ?? []).slice().sort();
		expect(at(1)).toEqual(['a']);
		expect(at(3)).toEqual(['a', 'b']);
		expect(at(4)).toEqual(['a', 'b', 'c']);
		expect(at(7)).toEqual(['b']);
		expect(at(9)).toEqual([]);
	});
});
