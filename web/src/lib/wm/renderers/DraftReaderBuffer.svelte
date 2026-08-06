<script lang="ts">
	import { getAppState } from '$lib/state.svelte';
	import DraftReader from './DraftReader.svelte';
	import type { Buffer } from '../types';

	let { buffer: _buffer }: { buffer: Buffer } = $props();

	const app = getAppState();

	function removeById(id: string) {
		const item = app.compose.sections.find((s) => s.id === id);
		if (item) app.handleDeleteFromCompose([item]);
	}

	function unlockAllImported() {
		for (const s of app.compose.sections) {
			if (s.source_addr && s.readonly) app.handleToggleReadonly(s.id);
		}
	}

	function lockAllUnlocked() {
		for (const s of app.compose.sections) {
			if (s.source_addr && !s.readonly && s.content === s.original_content) {
				app.handleToggleReadonly(s.id);
			}
		}
	}
</script>

<DraftReader
	compose={app.compose}
	publicationDTag={app.composeDTag}
	ontogglereadonly={app.handleToggleReadonly}
	onremove={removeById}
	onunlockall={unlockAllImported}
	onlockall={lockAllUnlocked}
/>
