// Buffer-kind metadata (kind / class / label / walkthroughs), decoupled from the
// component registry. `registry.ts` pairs this data with the actual Svelte buffer
// components; anything that only needs tour or class info imports from *here* so
// it stays off the registry↔component import cycle (registry → ComposerBuffer →
// ComposeView → …). Importing a component renderer just to read a tour key would
// drag every buffer module into the graph and — in the bundled build — expose a
// module-init-order TDZ (`Cannot access 'ComposerBuffer' before initialization`).

import type { ClassName } from './types';

/** One on-demand walkthrough offered by a buffer kind. `key` is its entry into
 *  `TIPS` (see discovery.svelte.ts); `mode` ties a tour to a composer view. */
export type BufferTour = {
	key: string;
	mode?: 'full' | 'plain';
};

/** A buffer kind's non-component metadata. */
export type BufferKindMeta = {
	kind: string;
	className: ClassName;
	defaultLabel?: string;
	/** The kind's *primary* walkthrough (an entry key into `TIPS`). */
	tour?: string;
	/** The full set of walkthroughs this kind offers (the composer's `W` dropdown
	 *  lists all of these). When absent, the single `tour` is the whole list. */
	tours?: BufferTour[];
};

export const BUFFER_KIND_META: BufferKindMeta[] = [
	{ kind: 'chat', className: 'chat', defaultLabel: 'chat' },
	{ kind: 'feed', className: 'work', defaultLabel: 'feed' },
	{ kind: 'reader', className: 'work', defaultLabel: 'reader', tour: 'reader-open' },
	{ kind: 'doc', className: 'work', defaultLabel: 'doc' },
	{ kind: 'draft-reader', className: 'work', defaultLabel: 'draft' },
	{
		kind: 'composer',
		className: 'work',
		defaultLabel: 'composer',
		tour: 'compose-output',
		tours: [
			{ key: 'compose-output' },
			{ key: 'compose-views' },
			{ key: 'compose-plain', mode: 'plain' },
			{ key: 'compose-detected', mode: 'plain' },
			{ key: 'compose-nostrdown', mode: 'plain' },
			{ key: 'compose-sections', mode: 'full' },
			{ key: 'compose-publish' }
		]
	},
	{ kind: 'profile', className: 'work', defaultLabel: 'profile' },
	{ kind: 'ignored', className: 'work', defaultLabel: 'ignored' },
	{ kind: 'settings', className: 'work', defaultLabel: 'settings', tour: 'sign-in-methods' },
	{ kind: 'relays', className: 'work', defaultLabel: 'relays' },
	{ kind: 'publish-progress', className: 'work', defaultLabel: 'publish' },
	{ kind: 'profile-edit', className: 'work', defaultLabel: 'profile' },
	{ kind: 'discussion-view', className: 'work', defaultLabel: 'discussion' },
	{ kind: 'search', className: 'research', defaultLabel: 'search', tour: 'search-tour-intro' }
];

const byKind: Record<string, BufferKindMeta> = Object.fromEntries(
	BUFFER_KIND_META.map((e) => [e.kind, e])
);

export function classForKind(kind: string): ClassName | undefined {
	return byKind[kind]?.className;
}

/** The on-demand walkthrough entry key for a buffer kind, if it declares one. */
export function tourForKind(kind: string | undefined): string | undefined {
	return kind ? byKind[kind]?.tour : undefined;
}

/** Every walkthrough a buffer kind offers, in order — its `tours` list, or a
 *  one-entry list synthesised from the single `tour`. Empty when it declares none. */
export function toursForKind(kind: string | undefined): BufferTour[] {
	const e = kind ? byKind[kind] : undefined;
	if (!e) return [];
	if (e.tours && e.tours.length) return e.tours;
	return e.tour ? [{ key: e.tour }] : [];
}

/** Every walkthrough available within a window-class, flattened across its buffer
 *  kinds and deduped by tour key; each row carries the owning buffer's label. */
export function toursForClass(
	className: ClassName
): { kind: string; label: string; key: string; mode?: 'full' | 'plain' }[] {
	const seen = new Set<string>();
	const out: { kind: string; label: string; key: string; mode?: 'full' | 'plain' }[] = [];
	for (const e of BUFFER_KIND_META) {
		if (e.className !== className) continue;
		for (const t of toursForKind(e.kind)) {
			if (seen.has(t.key)) continue;
			seen.add(t.key);
			out.push({ kind: e.kind, label: e.defaultLabel ?? e.kind, key: t.key, mode: t.mode });
		}
	}
	return out;
}
