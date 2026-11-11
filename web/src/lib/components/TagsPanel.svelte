<script lang="ts">
	// The composer's tags panel: each tag is a name and a list of values,
	// emitted as `[name, values…]`. Tags the engine sets itself (d, title, m,
	// a, …) are listed read-only under the editable rows so the author sees
	// the whole tag set the event will carry.
	import type { TagEntry } from '$lib/types';

	let {
		tags,
		onupdate,
		auto = [],
		disabled = false,
		empty = 'No tags yet.'
	}: {
		tags: TagEntry[];
		onupdate: (tags: TagEntry[]) => void;
		/** Engine-set tags, shown read-only (`d · title · m text/markdown`). */
		auto?: string[];
		disabled?: boolean;
		empty?: string;
	} = $props();

	const valuesOf = (t: TagEntry) => t.values ?? [t.value];

	function commit(i: number, name: string, values: string[]) {
		const next = tags.slice();
		next[i] = { name, value: values[0] ?? '', values };
		onupdate(next);
	}
	function setName(i: number, name: string) {
		commit(i, name.trim(), valuesOf(tags[i]));
	}
	function setValue(i: number, j: number, v: string) {
		const vs = valuesOf(tags[i]).slice();
		vs[j] = v;
		commit(i, tags[i].name, vs);
	}
	function addValue(i: number) {
		commit(i, tags[i].name, [...valuesOf(tags[i]), '']);
	}
	function removeValue(i: number, j: number) {
		const vs = valuesOf(tags[i]).filter((_, k) => k !== j);
		if (vs.length === 0) onupdate(tags.filter((_, k) => k !== i));
		else commit(i, tags[i].name, vs);
	}
	function addTag() {
		onupdate([...tags, { name: '', value: '', values: [''] }]);
	}
</script>

<div class="tags-panel">
	{#each tags as tag, i (i)}
		<div class="tp-row">
			<input
				class="tp-name"
				aria-label="Tag name"
				placeholder="name"
				value={tag.name}
				{disabled}
				onchange={(e) => setName(i, e.currentTarget.value)}
			/>
			{#each valuesOf(tag) as v, j (j)}
				<span class="tp-val">
					<input
						aria-label="Value {j + 1} of {tag.name || 'tag'}"
						placeholder="value"
						value={v}
						{disabled}
						oninput={(e) => setValue(i, j, e.currentTarget.value)}
					/>
					<button
						class="tp-x"
						aria-label="Remove value {j + 1}"
						title={valuesOf(tag).length === 1 ? 'Remove this tag' : 'Remove this value'}
						{disabled}
						onclick={() => removeValue(i, j)}>×</button
					>
				</span>
			{/each}
			<button class="tp-more" title="Add another value to this tag" {disabled} onclick={() => addValue(i)}
				>+ value</button
			>
		</div>
	{:else}
		<div class="tp-empty">{empty}</div>
	{/each}
	<div class="tp-foot">
		<button class="tp-add" {disabled} onclick={addTag}>+ Tag</button>
	</div>
	{#if auto.length > 0}
		<div class="tp-auto" title="Set by the engine when it signs — not editable here">
			{#each auto as a (a)}<span>{a}</span>{/each}
		</div>
	{/if}
</div>

<style>
	.tags-panel {
		display: flex;
		flex-direction: column;
		gap: 2px;
		font-size: var(--t-sm);
	}
	.tp-row {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 4px;
		padding: 4px 0;
		border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
	}
	input {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--fg);
		background: var(--bg-alt);
		border: 1px solid var(--border);
		border-radius: var(--r-sm);
		padding: 3px 6px;
		min-height: 26px;
		min-width: 0;
	}
	.tp-name {
		width: 72px;
		color: var(--yellow, var(--fg));
	}
	.tp-val {
		display: inline-flex;
		align-items: stretch;
		min-width: 0;
		flex: 1 1 110px;
	}
	.tp-val input {
		flex: 1;
		border-radius: var(--r-sm) 0 0 var(--r-sm);
	}
	.tp-x {
		background: var(--bg-alt);
		border: 1px solid var(--border);
		border-left: 0;
		border-radius: 0 var(--r-sm) var(--r-sm) 0;
		color: var(--fg-muted);
		cursor: pointer;
		min-width: 24px;
	}
	.tp-more,
	.tp-add {
		background: none;
		border: 1px dashed var(--border);
		border-radius: var(--r-sm);
		color: var(--fg-muted);
		cursor: pointer;
		font-size: var(--t-xs);
		padding: 2px 8px;
		min-height: 26px;
	}
	.tp-more:hover,
	.tp-add:hover,
	.tp-x:hover {
		color: var(--fg);
		border-color: var(--id-yours);
	}
	.tp-foot {
		padding-top: 6px;
	}
	.tp-empty {
		color: var(--fg-muted);
		font-size: var(--t-xs);
		padding: 4px 0;
	}
	.tp-auto {
		display: flex;
		flex-wrap: wrap;
		gap: 4px 10px;
		padding-top: 6px;
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		color: var(--fg-muted);
	}
</style>
