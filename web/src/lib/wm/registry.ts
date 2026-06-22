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

export type BufferKindEntry = {
	kind: string;
	className: ClassName;
	component: Component<RendererProps>;
	defaultLabel?: string;
	/** The on-demand walkthrough this buffer kind offers — an entry key into
	 *  `TIPS` (see discovery.svelte.ts). Single source of truth for "what
	 *  tutorial applies to this surface": the mode-line `W` runs the focused
	 *  buffer's `tour` and colours itself by whether it exists / has been run.
	 *  Surfaces with a richer in-chrome `W` chip (composer's mode-aware routing,
	 *  search's hands-on drill) still point here so the map reads them as
	 *  covered; the chip drives the nuanced behaviour. Omit only when a surface
	 *  genuinely has no tour yet (it then reads grey — author one). */
	tour?: string;
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
	// composer's in-chrome W chip is mode-aware (Full vs Plain chains); the
	// registry points at the Full overview so the mode-line map reads it as
	// covered (not grey) — the dedicated chip still routes by mode.
	{ kind: 'composer', className: 'work', component: ComposerBuffer, defaultLabel: 'composer', tour: 'compose-overview' },
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

/** The distinct walkthroughs available within a window-class — every buffer
 *  kind in that class that declares a `tour`, deduped. Backs the per-window
 *  `W` guide menu (Chat / Research): the menu lists these, each runnable. An
 *  empty list means the window has no tours yet (its `W` reads grey). */
export function toursForClass(className: ClassName): { kind: string; tour: string }[] {
	const seen = new Set<string>();
	const out: { kind: string; tour: string }[] = [];
	for (const e of entries) {
		if (e.className !== className || !e.tour || seen.has(e.tour)) continue;
		seen.add(e.tour);
		out.push({ kind: e.kind, tour: e.tour });
	}
	return out;
}
