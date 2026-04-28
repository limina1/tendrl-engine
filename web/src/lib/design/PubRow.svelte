<script lang="ts">
	import Icon from '../Icon.svelte';
	import IdBar, { type IdKind } from './IdBar.svelte';

	export type PubRowData = {
		title: string;
		author: string;
		date: string;
		sections?: number;
		status?: 'local' | 'remote' | 'draft' | 'imported' | 'forked' | 'diverged';
		kind: IdKind;
		tags?: string[];
	};

	type Props = {
		p: PubRowData;
		selected?: boolean;
		onmore?: (e: MouseEvent) => void;
	};

	let { p, selected = false, onmore }: Props = $props();
</script>

<div class="row {selected ? 'row--selected' : ''}">
	<IdBar kind={p.kind} />
	<div class="row__body">
		<div class="row__head">
			<span class="row__title">{p.title}</span>
		</div>
		<div class="row__meta">
			<span class="row__author">{p.author}</span>
			<span class="row__sep">·</span>
			<span>{p.date}</span>
			{#if p.sections}
				<span class="row__sep">·</span>
				<span>{p.sections} §</span>
			{/if}
			{#if p.tags}
				{#each p.tags as t (t)}
					<span class="pill pill--ghost row__tag">#{t}</span>
				{/each}
			{/if}
		</div>
	</div>
	<div class="row__tail">
		{#if p.status}
			<span class="pill pill--{p.status}">{p.status}</span>
		{/if}
		<button
			class="btn btn--ghost btn--icon"
			onclick={(e) => {
				e.stopPropagation();
				onmore?.(e);
			}}
			aria-label="More"
		>
			<Icon name="more" size={12} />
		</button>
	</div>
</div>

<style>
	.row__head {
		display: flex;
		align-items: baseline;
		gap: 8px;
	}
	.row__author { color: var(--base6); }
	.row__sep { color: var(--base4); }
	.row__tag { font-size: 10px; }
</style>
