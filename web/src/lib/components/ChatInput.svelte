<script lang="ts">
	import { untrack } from 'svelte';
	import { entryDrafts } from '$lib/wm/entry-drafts';

	let {
		onsend,
		disabled = false,
		draftKey = 'chat'
	}: {
		onsend: (content: string) => void;
		disabled?: boolean;
		/** Where the unsent message is parked across a buffer switch. */
		draftKey?: string;
	} = $props();

	// svelte-ignore state_referenced_locally
	let value = $state(entryDrafts.get(draftKey));
	let textarea: HTMLTextAreaElement | undefined = $state();
	$effect(() => {
		return () => entryDrafts.set(draftKey, value);
	});
	// A restored multi-line draft needs the box sized to it once mounted.
	$effect(() => {
		if (textarea) untrack(autoGrow);
	});

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Enter' && !e.shiftKey) {
			e.preventDefault();
			submit();
		}
	}

	function submit() {
		const trimmed = value.trim();
		if (!trimmed || disabled) return;
		onsend(trimmed);
		value = '';
		entryDrafts.clear(draftKey);
		if (textarea) textarea.style.height = 'auto';
	}

	function autoGrow() {
		if (!textarea) return;
		textarea.style.height = 'auto';
		textarea.style.height = Math.min(textarea.scrollHeight, 200) + 'px';
	}
</script>

<div class="input-bar">
	<textarea
		bind:this={textarea}
		bind:value
		onkeydown={handleKeydown}
		oninput={autoGrow}
		placeholder="Send a message..."
		rows="1"
		{disabled}
	></textarea>
	<button class="primary" onclick={submit} disabled={disabled || !value.trim()}>Send</button>
</div>

<style>
	.input-bar {
		display: flex;
		gap: 8px;
		padding: 12px 16px;
		border-top: 1px solid var(--border);
		background: var(--bg-surface);
	}

	textarea {
		flex: 1;
		min-height: 40px;
		max-height: 200px;
		line-height: 1.4;
	}

	button {
		align-self: flex-end;
	}
</style>
