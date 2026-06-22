import type { Component } from 'svelte';
import type { Buffer, ClassName } from './types';
import ChatBuffer from './renderers/ChatBuffer.svelte';
import FeedBuffer from './renderers/FeedBuffer.svelte';
import SearchBuffer from './renderers/SearchBuffer.svelte';
import ReaderBuffer from './renderers/ReaderBuffer.svelte';
import DocBuffer from './renderers/DocBuffer.svelte';
import ProfileBuffer from './renderers/ProfileBuffer.svelte';
import ComposerBuffer from './renderers/ComposerBuffer.svelte';
import IgnoredBuffer from './renderers/IgnoredBuffer.svelte';
import SettingsBuffer from './renderers/SettingsBuffer.svelte';
import DraftReaderBuffer from './renderers/DraftReaderBuffer.svelte';
import RelaysBuffer from './renderers/RelaysBuffer.svelte';
import PublishProgressBuffer from './renderers/PublishProgressBuffer.svelte';
import ProfileEditBuffer from './renderers/ProfileEditBuffer.svelte';
import DiscussionViewBuffer from './renderers/DiscussionViewBuffer.svelte';

export type RendererProps = {
	buffer: Buffer;
};

/** One on-demand walkthrough offered by a buffer kind. `key` is its entry into
 *  `TIPS` (see discovery.svelte.ts); the menu title is resolved from that tip.
 *  `mode` ties a tour to a composer view — selecting it from the in-chrome `W`
 *  dropdown switches the editor to that view first (and the row shows a `plain`
 *  / `full` tag). Omit `mode` for view-agnostic tours (they keep the current
 *  view and show no tag). */
export type BufferTour = {
	key: string;
	mode?: 'full' | 'plain';
};

export type BufferKindEntry = {
	kind: string;
	className: ClassName;
	component: Component<RendererProps>;
	defaultLabel?: string;
	/** The buffer kind's *primary* walkthrough — an entry key into `TIPS` (see
	 *  discovery.svelte.ts). Backs the mode-line `W` (runs the focused buffer's
	 *  tour) and its tri-state colour. When the kind offers several tours
	 *  (`tours` below), this is the first/overview one. Omit only when a surface
	 *  genuinely has no tour yet (it then reads grey — author one). */
	tour?: string;
	/** The full set of walkthroughs this buffer kind offers, for surfaces with a
	 *  richer in-chrome `W` *dropdown* (the composer lists all of these; the
	 *  logo `W` aggregates every buffer's, tagged by `defaultLabel`). When
	 *  absent, the single `tour` is the whole list. */
	tours?: BufferTour[];
};

const entries: BufferKindEntry[] = [
	{ kind: 'chat', className: 'chat', component: ChatBuffer, defaultLabel: 'chat' },
	// Work class is the "main content surface" — feed, reader, composer all
	// cycle the center slot. The user moves through read/write/feed as
	// modes of the same window.
	{ kind: 'feed', className: 'work', component: FeedBuffer, defaultLabel: 'feed' },
	{ kind: 'reader', className: 'work', component: ReaderBuffer, defaultLabel: 'reader', tour: 'reader-open' },
	// Slim single-document viewer for 30023 articles / 30818 wiki pages —
	// no pager/outline, just body + comments.
	{ kind: 'doc', className: 'work', component: DocBuffer, defaultLabel: 'doc' },
	{ kind: 'draft-reader', className: 'work', component: DraftReaderBuffer, defaultLabel: 'draft' },
	// The composer offers six discrete tutorials (not one chain). Its in-chrome
	// W is a dropdown listing all of them; selecting a `mode`-tagged tour
	// switches the editor to that view first. `tour` is the overview (Output),
	// so the mode-line map reads the composer as covered (not grey).
	{
		kind: 'composer',
		className: 'work',
		component: ComposerBuffer,
		defaultLabel: 'composer',
		tour: 'compose-output',
		tours: [
			{ key: 'compose-output' },
			{ key: 'compose-views' },
			{ key: 'compose-plain', mode: 'plain' },
			{ key: 'compose-detected', mode: 'plain' },
			{ key: 'compose-sections', mode: 'full' },
			{ key: 'compose-publish' }
		]
	},
	{ kind: 'profile', className: 'work', component: ProfileBuffer, defaultLabel: 'profile' },
	{ kind: 'ignored', className: 'work', component: IgnoredBuffer, defaultLabel: 'ignored' },
	{ kind: 'settings', className: 'work', component: SettingsBuffer, defaultLabel: 'settings', tour: 'sign-in-methods' },
	{ kind: 'relays', className: 'work', component: RelaysBuffer, defaultLabel: 'relays' },
	{ kind: 'publish-progress', className: 'work', component: PublishProgressBuffer, defaultLabel: 'publish' },
	{ kind: 'profile-edit', className: 'work', component: ProfileEditBuffer, defaultLabel: 'profile' },
	{ kind: 'discussion-view', className: 'work', component: DiscussionViewBuffer, defaultLabel: 'discussion' },
	// Research class — one buffer (search) that internally hosts Search /
	// Refs / KB sub-tabs cycled via h/l. The standalone RefsBuffer and
	// KnowledgebaseBuffer were retired once the SearchPanel grew matching
	// tabs (held items + import flow) — one surface, one cycle, one cursor.
	{ kind: 'search', className: 'research', component: SearchBuffer, defaultLabel: 'search', tour: 'search-tour-intro' }
];

const byKind: Record<string, BufferKindEntry> = Object.fromEntries(
	entries.map((e) => [e.kind, e])
);

export function rendererFor(kind: string): BufferKindEntry | undefined {
	return byKind[kind];
}

export function classForKind(kind: string): ClassName | undefined {
	return byKind[kind]?.className;
}

/** The on-demand walkthrough entry key for a buffer kind, if it declares one.
 *  Lets a single affordance (the logo `W`) offer the right tour for whatever
 *  the center/work window is focused on. */
export function tourForKind(kind: string | undefined): string | undefined {
	return kind ? byKind[kind]?.tour : undefined;
}

/** Every walkthrough a buffer kind offers, in order — its `tours` list, or a
 *  one-entry list synthesised from the single `tour`. Backs the composer's
 *  in-chrome `W` dropdown (each row carries its `mode` for the tag + view
 *  switch). Empty when the kind declares no tour. */
export function toursForKind(kind: string | undefined): BufferTour[] {
	const e = kind ? byKind[kind] : undefined;
	if (!e) return [];
	if (e.tours && e.tours.length) return e.tours;
	return e.tour ? [{ key: e.tour }] : [];
}

/** Every walkthrough available within a window-class, flattened across its
 *  buffer kinds and deduped by tour key. Each row carries the owning buffer's
 *  `label` (its `defaultLabel`) so the logo `W` can tag where a tour lives.
 *  Backs the per-window `W` guide menus (logo / Chat / Research); an empty list
 *  means the window has no tours yet (its `W` reads grey). */
export function toursForClass(
	className: ClassName
): { kind: string; label: string; key: string; mode?: 'full' | 'plain' }[] {
	const seen = new Set<string>();
	const out: { kind: string; label: string; key: string; mode?: 'full' | 'plain' }[] = [];
	for (const e of entries) {
		if (e.className !== className) continue;
		for (const t of toursForKind(e.kind)) {
			if (seen.has(t.key)) continue;
			seen.add(t.key);
			out.push({ kind: e.kind, label: e.defaultLabel ?? e.kind, key: t.key, mode: t.mode });
		}
	}
	return out;
}
