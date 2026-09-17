<script lang="ts">
	// The event's outward links: every `r` tag that is a web URL, rendered as
	// a real anchor. The engine emits `r` for each URL in the text on every
	// publish path (nostrdown::reference_tags) and agent-written notes cite
	// the web this way, so the tag row is the citation list — no prose
	// parsing here. Same component under a section, a note card, a request
	// and the event modal.
	let {
		tags = [],
		links = null,
		compact = false,
		label = 'links'
	}: {
		/** The event's tags; `r` values starting with http(s) are shown. */
		tags?: string[][];
		/** Pre-derived URL list (the tree stream ships `links` per leaf). */
		links?: string[] | null;
		compact?: boolean;
		label?: string;
	} = $props();

	const urls = $derived.by(() => {
		const src = links ?? tags.filter((t) => t[0] === 'r' && t[1]).map((t) => t[1]);
		const out: string[] = [];
		for (const u of src) {
			if (!/^https?:\/\//i.test(u) || out.includes(u)) continue;
			out.push(u);
		}
		return out;
	});

	/** host + a short path, so the row scans; the full URL is the title. */
	function short(u: string): string {
		try {
			const x = new URL(u);
			const path = x.pathname.replace(/\/$/, '');
			const tail = path.length > 28 ? '…' + path.slice(-27) : path;
			return x.host.replace(/^www\./, '') + tail;
		} catch {
			return u;
		}
	}
</script>

{#if urls.length > 0}
	<nav class="elinks" class:elinks--compact={compact} aria-label="Outward links">
		{#if !compact}<span class="elinks__k">{label}</span>{/if}
		{#each urls as u (u)}
			<a class="elinks__a" href={u} target="_blank" rel="noopener noreferrer" title={u}>{short(u)}<span class="elinks__out" aria-hidden="true">↗</span></a>
		{/each}
	</nav>
{/if}

<style>
	.elinks {
		display: flex;
		flex-wrap: wrap;
		align-items: baseline;
		gap: 4px 8px;
		margin-top: var(--s-1);
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		line-height: 1.5;
	}
	.elinks--compact { margin-top: 4px; }
	.elinks__k {
		font-size: var(--t-3xs);
		font-weight: 600;
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--base5);
		margin-right: 2px;
	}
	.elinks__a {
		color: var(--id-remote, var(--cyan));
		text-decoration: none;
		border-bottom: 1px dotted currentColor;
		max-width: 100%;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		min-height: 22px;
		display: inline-flex;
		align-items: center;
		gap: 3px;
	}
	.elinks__a:hover, .elinks__a:focus-visible { color: var(--fg); border-bottom-style: solid; }
	.elinks__out { font-size: var(--t-3xs); opacity: 0.8; }
</style>
