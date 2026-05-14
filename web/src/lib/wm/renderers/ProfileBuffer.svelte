<script lang="ts">
	import ProfileView from '$lib/components/ProfileView.svelte';
	import { getActiveStore } from '../buffer-store.svelte';
	import type { Buffer } from '../types';

	let { buffer }: { buffer: Buffer } = $props();

	const store = getActiveStore();

	const pubkey = $derived(parsePubkey(buffer.id));

	function parsePubkey(id: string): string | null {
		const m = id.match(/^profile:([0-9a-f]{64})$/);
		return m ? m[1] : null;
	}

	function openPub(pub: { addr: { kind: number; pubkey: string; d_tag: string }; title: string | null }) {
		const id = `reader:${pub.addr.kind}:${pub.addr.pubkey}:${pub.addr.d_tag}`;
		store.openBuffer({
			className: 'work',
			buffer: { id, kind: 'reader', label: 'reader', kicker: pub.title ?? '[Untitled]' }
		});
	}

	function openAddr(addr: { kind: number; pubkey: string; d_tag: string }, title: string | null) {
		// Non-30040 addressables (long-form articles, wikis) reuse the same
		// reader buffer route — the reader's parseBufferId regex matches
		// any kind, and the publication-load path will fall back to a
		// single-section view when the event isn't an NKBIP-01 index.
		const id = `reader:${addr.kind}:${addr.pubkey}:${addr.d_tag}`;
		const label = addr.kind === 30023 ? 'article' : addr.kind === 30818 ? 'wiki' : 'reader';
		store.openBuffer({
			className: 'work',
			buffer: { id, kind: 'reader', label, kicker: title ?? addr.d_tag ?? '[Untitled]' }
		});
	}
</script>

<div class="profile-wrap">
	{#if pubkey}
		<ProfileView
			{pubkey}
			onopenpub={openPub}
			onopenaddr={openAddr}
			onback={() => store.killFocused()}
		/>
	{:else}
		<div class="empty"><p>Buffer id does not encode a profile pubkey</p></div>
	{/if}
</div>

<style>
	.profile-wrap { display: flex; flex-direction: column; height: 100%; min-height: 0; }
	.empty { flex: 1; display: flex; align-items: center; justify-content: center; color: var(--base5); font-size: var(--t-sm); }
</style>
