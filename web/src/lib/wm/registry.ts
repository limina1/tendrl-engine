import type { Component } from 'svelte';
import type { Buffer, ClassName } from './types';
import ChatBuffer from './renderers/ChatBuffer.svelte';
import FeedBuffer from './renderers/FeedBuffer.svelte';
import SearchBuffer from './renderers/SearchBuffer.svelte';
import ReaderBuffer from './renderers/ReaderBuffer.svelte';
import ProfileBuffer from './renderers/ProfileBuffer.svelte';
import ComposerBuffer from './renderers/ComposerBuffer.svelte';
import IgnoredBuffer from './renderers/IgnoredBuffer.svelte';
import KnowledgebaseBuffer from './renderers/KnowledgebaseBuffer.svelte';
import RefsBuffer from './renderers/RefsBuffer.svelte';

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
	{ kind: 'feed', className: 'research', component: FeedBuffer, defaultLabel: 'feed' },
	{ kind: 'search', className: 'research', component: SearchBuffer, defaultLabel: 'search' },
	{ kind: 'reader', className: 'work', component: ReaderBuffer, defaultLabel: 'reader' },
	{ kind: 'profile', className: 'work', component: ProfileBuffer, defaultLabel: 'profile' },
	{ kind: 'composer', className: 'work', component: ComposerBuffer, defaultLabel: 'composer' },
	{ kind: 'ignored', className: 'work', component: IgnoredBuffer, defaultLabel: 'ignored' },
	{ kind: 'knowledgebase', className: 'research', component: KnowledgebaseBuffer, defaultLabel: 'kb' },
	{ kind: 'refs', className: 'research', component: RefsBuffer, defaultLabel: 'refs' }
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
