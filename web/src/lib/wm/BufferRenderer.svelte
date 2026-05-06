<script lang="ts">
	import type { Buffer } from './types';
	import ChatBuffer from './renderers/ChatBuffer.svelte';
	import FeedBuffer from './renderers/FeedBuffer.svelte';
	import SearchBuffer from './renderers/SearchBuffer.svelte';
	import ReaderBuffer from './renderers/ReaderBuffer.svelte';
	import ProfileBuffer from './renderers/ProfileBuffer.svelte';
	import ComposerBuffer from './renderers/ComposerBuffer.svelte';
	import IgnoredBuffer from './renderers/IgnoredBuffer.svelte';
	import KnowledgebaseBuffer from './renderers/KnowledgebaseBuffer.svelte';
	import RefsBuffer from './renderers/RefsBuffer.svelte';
	import SettingsBuffer from './renderers/SettingsBuffer.svelte';
	import DraftReaderBuffer from './renderers/DraftReaderBuffer.svelte';
	import RelaysBuffer from './renderers/RelaysBuffer.svelte';

	let { buffer }: { buffer: Buffer } = $props();
</script>

{#key buffer.id}
	{#if buffer.kind === 'chat'}
		<ChatBuffer {buffer} />
	{:else if buffer.kind === 'feed'}
		<FeedBuffer {buffer} />
	{:else if buffer.kind === 'search'}
		<SearchBuffer {buffer} />
	{:else if buffer.kind === 'reader'}
		<ReaderBuffer {buffer} />
	{:else if buffer.kind === 'draft-reader'}
		<DraftReaderBuffer {buffer} />
	{:else if buffer.kind === 'profile'}
		<ProfileBuffer {buffer} />
	{:else if buffer.kind === 'composer'}
		<ComposerBuffer {buffer} />
	{:else if buffer.kind === 'ignored'}
		<IgnoredBuffer {buffer} />
	{:else if buffer.kind === 'knowledgebase'}
		<KnowledgebaseBuffer {buffer} />
	{:else if buffer.kind === 'refs'}
		<RefsBuffer {buffer} />
	{:else if buffer.kind === 'settings'}
		<SettingsBuffer {buffer} />
	{:else if buffer.kind === 'relays'}
		<RelaysBuffer {buffer} />
	{:else}
		<div class="missing">no renderer registered for kind {buffer.kind}</div>
	{/if}
{/key}

<style>
	.missing {
		padding: var(--s-3);
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--base5);
		font-style: italic;
	}
</style>
