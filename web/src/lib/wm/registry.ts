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
};

const entries: BufferKindEntry[] = [
	{ kind: 'chat', className: 'chat', component: ChatBuffer, defaultLabel: 'chat' },
	// Work class is the "main content surface" — feed, reader, composer all
	// cycle the center slot. The user moves through read/write/feed as
	// modes of the same window.
	{ kind: 'feed', className: 'work', component: FeedBuffer, defaultLabel: 'feed' },
	{ kind: 'reader', className: 'work', component: ReaderBuffer, defaultLabel: 'reader' },
	// Slim single-document viewer for 30023 articles / 30818 wiki pages —
	// no pager/outline, just body + comments.
	{ kind: 'doc', className: 'work', component: DocBuffer, defaultLabel: 'doc' },
	{ kind: 'draft-reader', className: 'work', component: DraftReaderBuffer, defaultLabel: 'draft' },
	{ kind: 'composer', className: 'work', component: ComposerBuffer, defaultLabel: 'composer' },
	{ kind: 'profile', className: 'work', component: ProfileBuffer, defaultLabel: 'profile' },
	{ kind: 'ignored', className: 'work', component: IgnoredBuffer, defaultLabel: 'ignored' },
	{ kind: 'settings', className: 'work', component: SettingsBuffer, defaultLabel: 'settings' },
	{ kind: 'relays', className: 'work', component: RelaysBuffer, defaultLabel: 'relays' },
	{ kind: 'publish-progress', className: 'work', component: PublishProgressBuffer, defaultLabel: 'publish' },
	{ kind: 'profile-edit', className: 'work', component: ProfileEditBuffer, defaultLabel: 'profile' },
	{ kind: 'discussion-view', className: 'work', component: DiscussionViewBuffer, defaultLabel: 'discussion' },
	// Research class — one buffer (search) that internally hosts Search /
	// Refs / KB sub-tabs cycled via h/l. The standalone RefsBuffer and
	// KnowledgebaseBuffer were retired once the SearchPanel grew matching
	// tabs (held items + import flow) — one surface, one cycle, one cursor.
	{ kind: 'search', className: 'research', component: SearchBuffer, defaultLabel: 'search' }
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
