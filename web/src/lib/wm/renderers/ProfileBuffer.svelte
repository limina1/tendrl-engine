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
		const kicker = title ?? addr.d_tag ?? '[Untitled]';
		// Long-form articles (30023), wiki pages (30818), and NIP specs
		// (30817) are single documents — route them to the slim DocBuffer,
		// which drops the reader's pager/outline chrome.
		if (addr.kind === 30023 || addr.kind === 30818 || addr.kind === 30817) {
			store.openBuffer({
				className: 'work',
				buffer: {
					id: `doc:${addr.kind}:${addr.pubkey}:${addr.d_tag}`,
					kind: 'doc',
					label: addr.kind === 30023 ? 'article' : addr.kind === 30817 ? 'spec' : 'wiki',
					kicker
				}
			});
			return;
		}
		// Other addressables (e.g. standalone 30041 sections) reuse the
		// reader buffer route — its parseBufferId regex matches any kind,
		// and the publication-load path falls back to a single-section view.
		store.openBuffer({
			className: 'work',
			buffer: {
				id: `reader:${addr.kind}:${addr.pubkey}:${addr.d_tag}`,
				kind: 'reader',
				label: addr.kind === 30041 ? 'section' : 'reader',
				kicker
			}
		});
	}

	function openComment(event: { id: string; content: string; kind?: number }) {
		// A NIP-22 comment or NIP-84 highlight isn't a standalone reader
		// destination — route it to the DiscussionViewBuffer, which resolves
		// the thread / highlighted target it belongs to. Mirrors
		// +page.svelte's `onDiscussion` handler.
		const kicker = event.content.trim().slice(0, 32) || event.id.slice(0, 8) + '…';
		store.openBuffer({
			className: 'work',
			buffer: {
				id: `discussion:${event.id}`,
				kind: 'discussion-view',
				label: event.kind === 9802 ? 'highlight' : 'comment',
				kicker
			}
		});
	}
</script>

<div class="profile-wrap">
	{#if pubkey}
		<ProfileView
			{pubkey}
			bufferId={buffer.id}
			onopenpub={openPub}
			onopenaddr={openAddr}
			oncomment={openComment}
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
