import { describe, expect, it } from 'vitest';
import { resolveLayer } from './resolve';
import { ALL_BASE_IDS, SOURCE_ID } from './registry';
import type { LayerPrefs } from './store.svelte';

// Plain object with the defaults' shape — the store module itself imports
// `$app/environment`, so tests build prefs by hand.
const defaults = (): LayerPrefs => ({
	enabled: [...ALL_BASE_IDS],
	byMimetype: {},
	byPublication: {},
	untagged: null
});

const dec = (
	effectiveFormat: string | null,
	prefs: LayerPrefs,
	extra: Partial<Parameters<typeof resolveLayer>[0]> = {}
) => resolveLayer({ effectiveFormat, pubCoord: null, pick: undefined, ...extra }, prefs);

describe('resolveLayer', () => {
	it('defaults: the m tag picks the base, untagged stays source', () => {
		expect(dec('text/org', defaults())).toEqual({ layerId: 'org-decorated', source: 'format' });
		expect(dec('text/asciidoc', defaults())).toEqual({
			layerId: 'adoc-decorated',
			source: 'format'
		});
		expect(dec('text/markdown', defaults())).toEqual({ layerId: 'md-decorated', source: 'format' });
		expect(dec('text/plain', defaults())).toEqual({ layerId: null, source: 'none' });
		expect(dec(null, defaults())).toEqual({ layerId: null, source: 'none' });
	});

	it('a switched-off base never resolves', () => {
		const p = defaults();
		p.enabled = p.enabled.filter((id) => id !== 'org-decorated');
		expect(dec('text/org', p)).toEqual({ layerId: null, source: 'none' });
	});

	it('untagged fallback is the user\'s answer and is reported as such', () => {
		const p = defaults();
		p.untagged = 'text/asciidoc';
		expect(dec(null, p)).toEqual({ layerId: 'adoc-decorated', source: 'untagged' });
		// A declared format still beats the fallback.
		expect(dec('text/org', p)).toEqual({ layerId: 'org-decorated', source: 'format' });
		// A per-mimetype binding to source applies to the fallback too.
		p.byMimetype['text/asciidoc'] = SOURCE_ID;
		expect(dec(null, p)).toEqual({ layerId: null, source: 'untagged' });
	});

	it('pick and forced source win over everything', () => {
		const p = defaults();
		p.untagged = 'text/org';
		expect(dec('text/markdown', p, { pick: 'adoc-decorated' })).toEqual({
			layerId: 'adoc-decorated',
			source: 'pick'
		});
		expect(dec('text/markdown', p, { pick: null })).toEqual({ layerId: null, source: 'pick' });
		expect(dec('text/markdown', p, { forceSource: true })).toEqual({
			layerId: null,
			source: 'forced'
		});
	});
});
