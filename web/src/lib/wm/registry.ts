import type { Component } from 'svelte';
import type { Buffer } from './types';
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
import { BUFFER_KIND_META, type BufferKindMeta } from './tours';

// Tour + class lookups live in the leaf `./tours` (no component imports) so a
// call site that only needs a tour key doesn't pull every renderer into the
// graph — which, in the bundled build, surfaced a module-init-order TDZ
// (`Cannot access 'ComposerBuffer' before initialization`) via the cycle
// registry → ComposerBuffer → ComposeView → registry. Re-exported here for
// existing importers; new call sites that only need tours/class should import
// from `./tours` directly.
export { classForKind, tourForKind, toursForKind, toursForClass } from './tours';
export type { BufferTour, BufferKindMeta } from './tours';

export type RendererProps = {
	buffer: Buffer;
};

export type BufferKindEntry = BufferKindMeta & {
	component: Component<RendererProps>;
};

/** kind → renderer component, paired with the pure metadata from `./tours`. */
const COMPONENTS: Record<string, Component<RendererProps>> = {
	chat: ChatBuffer,
	feed: FeedBuffer,
	reader: ReaderBuffer,
	doc: DocBuffer,
	'draft-reader': DraftReaderBuffer,
	composer: ComposerBuffer,
	profile: ProfileBuffer,
	ignored: IgnoredBuffer,
	settings: SettingsBuffer,
	relays: RelaysBuffer,
	'publish-progress': PublishProgressBuffer,
	'profile-edit': ProfileEditBuffer,
	'discussion-view': DiscussionViewBuffer,
	search: SearchBuffer
};

const entries: BufferKindEntry[] = BUFFER_KIND_META.map((m) => ({
	...m,
	component: COMPONENTS[m.kind]
}));

const byKind: Record<string, BufferKindEntry> = Object.fromEntries(
	entries.map((e) => [e.kind, e])
);

export function rendererFor(kind: string): BufferKindEntry | undefined {
	return byKind[kind];
}
