import { describe, expect, it } from 'vitest';
import { base } from './markdown';

describe('markdown base — soft breaks', () => {
	it('newlines inside a paragraph are soft; hard breaks, blank lines, fences and headings are not', () => {
		const t = '# H\nline one\nline two  \nline three\n\npara two\n- a\n  wrapped\n```\nx\ny\n```';
		const s = base.spans(t);
		const at = (needle: string) => t.indexOf(needle) - 1;
		const softs = s.filter((x) => x.cls === 'soft').map((x) => x.start);
		expect(softs).toContain(at('line two'));
		expect(softs).toContain(at('  wrapped'));
		expect(softs).not.toContain(at('line one')); // after the heading
		expect(softs).not.toContain(at('line three')); // after a two-space hard break
		expect(softs).not.toContain(at('para two')); // across a blank line
		expect(softs).not.toContain(at('y\n')); // inside a fence
		expect(s.filter((x) => x.cls === 'soft').every((x) => /^\n[ \t]*$/.test(t.slice(x.start, x.end)))).toBe(true);
	});
});
