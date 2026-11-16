import { describe, expect, it } from 'vitest';
import { hiddenMarkerSpans } from './hide-markers';
import { base as md } from './bases/markdown';
import { base as org } from './bases/org';
import { covered } from './bases/test-util';

const hide = (t: string, b: { spans(t: string): import('./registry').StyleSpan[] }) =>
	hiddenMarkerSpans(t, b.spans(t));

describe('hidden markers', () => {
	it('markdown: heading sigil + space, emphasis, labelled link target', () => {
		const t = '## Title\n\nsome **bold** and [label](https://x.org)';
		const h = hide(t, md);
		expect(covered(t, h, 'hide')).toEqual(expect.arrayContaining(['## ', '**', 'https://x.org']));
		expect(covered(t, h, 'hide')).not.toContain('label');
	});

	it('markdown: bullets become •, numbers stay', () => {
		const t = '- one\n- two\n\n1. first';
		const h = hide(t, md);
		expect(covered(t, h, 'bullet')).toEqual(['-', '-']);
		expect(covered(t, h, 'listnum')).toEqual(['1.']);
	});

	it('org: heading stars are hidden, not bullets', () => {
		const t = '* Top\n- item';
		const h = hide(t, org);
		expect(covered(t, h, 'hide')).toContain('* ');
		expect(covered(t, h, 'bullet')).toEqual(['-']);
	});

	it('table pipes stay', () => {
		const t = '| a | b |\n|---|---|\n| 1 | 2 |';
		expect(covered(t, hide(t, md), 'hide').some((s) => s.includes('|'))).toBe(false);
	});
});
