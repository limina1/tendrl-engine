<script lang="ts">
	// The narrate modal's review step: every section of the publication with
	// the engine's transcript flags (src/transcript.rs). Skip flags start
	// struck through, review flags start unresolved; a click on a span opens
	// skip / keep / replace. "edit text" shows the final transcript (resolved
	// by the engine) as a textarea — what is typed there is what gets
	// narrated and stored as the recording's transcript. Decisions live in
	// the parent (NarrateModal) as bindable records; the engine applies them.
	import * as api from '$lib/api';
	import {
		KIND_LABEL,
		countsFor,
		decisionOf,
		editsFor,
		flagKey,
		replacementOf,
		segmentsOf,
		type Decision
	} from '$lib/narration-review';

	let {
		nodes,
		picked = $bindable(),
		decisions = $bindable(),
		reps = $bindable(),
		handText = $bindable(),
		disabled = false
	}: {
		nodes: api.NarrationPlanNode[];
		picked: Record<string, boolean>;
		decisions: Record<string, Decision>;
		reps: Record<string, string>;
		handText: Record<string, string>;
		disabled?: boolean;
	} = $props();

	const sections = $derived(nodes.filter((n) => !n.is_index && n.spoken != null));

	/** First ticked section with something to look at, else the first one. */
	function firstInteresting(): string {
		const on = sections.filter((s) => picked[s.coord] !== false);
		const n =
			on.find((s) => countsFor(s, decisions, handText).pending > 0) ??
			on.find((s) => (s.review?.flags.length ?? 0) > 0) ??
			on[0] ??
			sections[0];
		return n?.coord ?? '';
	}
	// svelte-ignore state_referenced_locally
	let selected = $state(firstInteresting());
	let focus = $state(0);
	let editing = $state(false);
	let editText = $state('');
	let editLoading = $state(false);
	let recheck = $state<{ flags: number; text: string } | null>(null);
	let transcriptEl: HTMLDivElement | undefined = $state();
	let repInput: HTMLInputElement | undefined = $state();

	const cur = $derived(sections.find((s) => s.coord === selected) ?? sections[0] ?? null);
	const flags = $derived(cur?.review?.flags ?? []);
	const segs = $derived(cur?.spoken != null ? segmentsOf(cur.spoken, flags) : []);
	const curCounts = $derived(cur ? countsFor(cur, decisions, handText) : { skips: 0, pending: 0 });
	const isHand = $derived(!!cur && handText[cur.coord] !== undefined);
	const focusFlag = $derived(flags.length ? flags[Math.min(focus, flags.length - 1)] : null);
	const focusIdx = $derived(Math.min(focus, Math.max(flags.length - 1, 0)));
	const focusDecision = $derived(cur && focusFlag ? decisionOf(focusFlag, cur.coord, focusIdx, decisions) : 'pending');
	const totalPending = $derived(sections.reduce((a, s) => a + (picked[s.coord] !== false ? countsFor(s, decisions, handText).pending : 0), 0));

	function selectSection(coord: string) {
		selected = coord;
		focus = 0;
		editing = false;
		recheck = null;
	}

	function decide(i: number, d: Exclude<Decision, 'pending'>) {
		if (!cur || disabled) return;
		decisions[flagKey(cur.coord, i)] = d;
	}

	function setRep(i: number, value: string) {
		if (!cur) return;
		reps[flagKey(cur.coord, i)] = value;
	}

	function keepAll() {
		if (!cur || disabled) return;
		flags.forEach((f, i) => {
			if (decisionOf(f, cur.coord, i, decisions) === 'pending') decisions[flagKey(cur.coord, i)] = 'keep';
		});
	}

	function focusFlagAt(i: number) {
		focus = i;
		queueMicrotask(() => transcriptEl?.querySelector(`[data-flag="${i}"]`)?.scrollIntoView({ block: 'nearest' }));
	}

	async function openEdit() {
		if (!cur || cur.spoken == null) return;
		editing = true;
		recheck = null;
		if (handText[cur.coord] !== undefined) {
			editText = handText[cur.coord];
			void runRecheck(cur.coord, editText);
			return;
		}
		editLoading = true;
		const coord = cur.coord;
		try {
			const r = await api.reviewNarrationText({ text: cur.spoken, title: cur.title, edits: editsFor(cur, decisions, reps) });
			if (selected !== coord) return;
			editText = r.text;
			recheck = { flags: r.flags.length, text: r.text };
		} catch (e) {
			editText = '';
			recheck = { flags: -1, text: e instanceof Error ? e.message : String(e) };
		} finally {
			editLoading = false;
		}
	}

	// Re-check a hand edit through the engine, debounced; a stale response
	// (the text moved on) is dropped.
	let recheckTimer: ReturnType<typeof setTimeout> | null = null;
	function runRecheck(coord: string, text: string) {
		if (recheckTimer) clearTimeout(recheckTimer);
		recheckTimer = setTimeout(async () => {
			try {
				const node = sections.find((s) => s.coord === coord);
				const r = await api.reviewNarrationText({ text, title: node?.title ?? '' });
				if (selected === coord && editText === text) recheck = { flags: r.flags.length, text: r.text };
			} catch {
				/* the re-check is advisory */
			}
		}, 450);
	}

	function onEditInput(e: Event) {
		if (!cur) return;
		editText = (e.target as HTMLTextAreaElement).value;
		handText[cur.coord] = editText;
		runRecheck(cur.coord, editText);
	}

	function discardEdit() {
		if (!cur) return;
		delete handText[cur.coord];
		editing = false;
		recheck = null;
	}

	function onKey(e: KeyboardEvent) {
		const t = e.target as HTMLElement;
		if (t.closest('input, textarea, select') || e.metaKey || e.ctrlKey || e.altKey || editing || !flags.length) return;
		if (e.key === 'j' || e.key === 'n') focusFlagAt(Math.min(focusIdx + 1, flags.length - 1));
		else if (e.key === 'k' || e.key === 'p') focusFlagAt(Math.max(focusIdx - 1, 0));
		else if (e.key === 's') decide(focusIdx, 'skip');
		else if (e.key === 'a') decide(focusIdx, 'keep');
		else if (e.key === 'r') {
			decide(focusIdx, 'replace');
			queueMicrotask(() => repInput?.focus());
		} else return;
		e.preventDefault();
		e.stopPropagation();
	}

	function segClass(i: number): string {
		if (!cur) return '';
		const d = decisionOf(flags[i], cur.coord, i, decisions);
		return `tr-span tr-span--${d}${i === focusIdx ? ' tr-span--focus' : ''}`;
	}
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div class="tr" onkeydown={onKey} role="group" aria-label="Review transcript" tabindex="-1">
	<section class="tr-list" aria-label="Sections">
		<div class="tr-list__head">
			<span>sections</span>
			<span class="tr-sp"></span>
			<span class:tr-warn={totalPending > 0}>{totalPending} to check</span>
		</div>
		<ul data-tour="narrate-review-sections">
			{#each nodes as n (n.coord)}
				{#if n.is_index}
					<li class="tr-row tr-row--index" style="--d:{n.depth}">{n.title}</li>
				{:else if n.spoken != null}
					{@const c = countsFor(n, decisions, handText)}
					{@const on = picked[n.coord] !== false}
					<li class="tr-row" class:tr-row--sel={cur?.coord === n.coord} style="--d:{n.depth}">
						<input type="checkbox" checked={on} {disabled} aria-label="narrate {n.title}" onchange={(e) => (picked[n.coord] = (e.target as HTMLInputElement).checked)} />
						<button class="tr-row__title" class:tr-row__title--off={!on} aria-current={cur?.coord === n.coord} onclick={() => selectSection(n.coord)}>{n.title}</button>
						{#if n.stale}<span class="tr-chip tr-chip--skip" title="this section changed since it was reviewed">changed</span>{/if}
						{#if handText[n.coord] !== undefined}<span class="tr-chip" title="hand-edited transcript">edited</span>{/if}
						{#if n.review?.section?.action === 'skip'}<span class="tr-chip tr-chip--skip" title={n.review.section.reason}>skip §</span>{/if}
						{#if c.skips > 0}<span class="tr-chip" title="spans skipped">{c.skips} skip</span>{/if}
						{#if c.pending > 0}<span class="tr-chip tr-chip--check" title="spans to check">{c.pending} check</span>{/if}
					</li>
				{/if}
			{/each}
		</ul>
	</section>

	<section class="tr-main" aria-label="Transcript">
		{#if cur}
			<div class="tr-main__head">
				<span class="tr-main__title">{cur.title}</span>
				<span class="tr-dim">{(cur.spoken ?? '').length.toLocaleString()} chars · {curCounts.skips} skipped · {curCounts.pending} to check</span>
				<span class="tr-sp"></span>
				{#if curCounts.pending > 0 && !editing && !isHand}
					<button class="tr-btn" onclick={keepAll} {disabled}>keep all checked</button>
				{/if}
				<button class="tr-btn" class:tr-btn--on={editing} aria-pressed={editing} onclick={() => (editing ? (editing = false) : openEdit())} disabled={disabled || cur.spoken == null}>
					{editing ? 'back to flags' : 'edit text'}
				</button>
			</div>

			{#if cur.review?.section?.action === 'skip'}
				<div class="tr-banner">
					<span class="tr-banner__lead">suggested: skip this section</span>
					<span class="tr-dim tr-grow">{cur.review.section.reason}</span>
					<button class="tr-btn" onclick={() => (picked[cur.coord] = picked[cur.coord] === false)} {disabled}>
						{picked[cur.coord] === false ? 'narrate anyway' : 'skip section'}
					</button>
				</div>
			{/if}
			{#if isHand && !editing}
				<div class="tr-banner tr-banner--note">
					<span class="tr-grow">hand-edited — the edited text is what gets narrated; the decisions below no longer apply</span>
					<button class="tr-btn" onclick={() => openEdit()} {disabled}>open edit</button>
					<button class="tr-btn" onclick={discardEdit} {disabled}>discard edit</button>
				</div>
			{/if}

			{#if editing}
				<div class="tr-edit">
					<label for="tr-edit-text" class="tr-dim">final transcript — what the voice reads (skips and replacements applied by the engine; your edits are re-checked)</label>
					<textarea id="tr-edit-text" data-entry value={editText} oninput={onEditInput} disabled={disabled || editLoading} spellcheck="true"></textarea>
					<span class="tr-dim">
						{#if editLoading}resolving…{:else if recheck && recheck.flags < 0}{recheck.text}{:else if recheck}re-checked: {recheck.flags === 0 ? 'no flags' : `${recheck.flags} flag${recheck.flags === 1 ? '' : 's'} in this text`}{/if}
						{#if isHand} · <button class="tr-link" onclick={discardEdit} {disabled}>discard edit</button>{/if}
					</span>
				</div>
			{:else}
				<div class="tr-text" class:tr-text--off={picked[cur.coord] === false || isHand} bind:this={transcriptEl}>
					{#if !segs.length}<span class="tr-dim">This section has no spoken text.</span>{/if}
					{#each segs as s, si (si)}
						{#if s.flag === undefined}{s.text}{:else}<button
								class={segClass(s.flag)}
								data-flag={s.flag}
								title="{KIND_LABEL[flags[s.flag].kind]} — {flags[s.flag].reason}"
								onclick={() => (focus = s.flag ?? 0)}
								><span class="tr-span__orig">{s.text}</span>{#if decisionOf(flags[s.flag], cur.coord, s.flag, decisions) === 'replace'}<span class="tr-span__rep">{replacementOf(flags[s.flag], cur.coord, s.flag, reps)}</span>{/if}</button
							>{/if}
					{/each}
				</div>

				{#if focusFlag && !isHand}
					<div class="tr-card">
						<div class="tr-card__row">
							<span class="tr-chip">{KIND_LABEL[focusFlag.kind]}</span>
							<span class="tr-dim tr-grow">{focusFlag.reason}</span>
							<span class="tr-dim">{focusIdx + 1} of {flags.length} · {({ pending: 'unresolved', skip: 'skipped', keep: 'kept', replace: 'replaced' })[focusDecision]}</span>
						</div>
						<div class="tr-card__row tr-card__row--wrap">
							<span class="tr-card__excerpt">{focusFlag.excerpt}</span>
							<span class="tr-grow"></span>
							<button class="tr-btn" class:tr-btn--on={focusDecision === 'skip'} onclick={() => decide(focusIdx, 'skip')} {disabled}>skip</button>
							<button class="tr-btn" class:tr-btn--on={focusDecision === 'keep'} onclick={() => decide(focusIdx, 'keep')} {disabled}>keep as is</button>
							<label for="tr-rep" class="tr-dim">read as</label>
							<input
								id="tr-rep"
								class="tr-input"
								data-entry
								bind:this={repInput}
								value={replacementOf(focusFlag, cur.coord, focusIdx, reps)}
								oninput={(e) => setRep(focusIdx, (e.target as HTMLInputElement).value)}
								onkeydown={(e) => {
									if (e.key === 'Enter') decide(focusIdx, 'replace');
								}}
								placeholder="spoken form"
								{disabled}
							/>
							<button class="tr-btn" class:tr-btn--on={focusDecision === 'replace'} onclick={() => decide(focusIdx, 'replace')} {disabled}>replace</button>
						</div>
						<div class="tr-dim tr-keys">j / k next · previous flag · s skip · a keep · r replace</div>
					</div>
				{/if}
			{/if}
		{:else}
			<div class="tr-dim tr-empty">No section text to review.</div>
		{/if}
	</section>
</div>

<style>
	.tr {
		display: grid;
		grid-template-columns: minmax(220px, 340px) minmax(0, 1fr);
		min-height: 0;
		flex: 1 1 auto;
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm, 3px);
		outline: none;
	}
	.tr-list {
		border-right: 1px solid var(--panel-border);
		display: flex;
		flex-direction: column;
		min-height: 0;
	}
	.tr-list__head,
	.tr-main__head {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 6px 10px;
		border-bottom: 1px solid var(--panel-border);
		color: var(--base6);
	}
	.tr-list ul {
		list-style: none;
		margin: 0;
		padding: 4px 0;
		overflow-y: auto;
		flex: 1 1 auto;
		min-height: 0;
	}
	.tr-row {
		display: flex;
		align-items: center;
		gap: 6px;
		padding: 3px 8px 3px calc(8px + var(--d, 0) * 12px);
		border-left: 2px solid transparent;
	}
	.tr-row--sel {
		background: var(--base2);
		border-left-color: var(--blue);
	}
	.tr-row--index {
		color: var(--base6);
		padding-top: 8px;
		font-size: calc(var(--t-xs) - 1px);
	}
	.tr-row input {
		margin: 0;
		flex-shrink: 0;
		accent-color: var(--blue);
	}
	.tr-row__title {
		flex: 1 1 auto;
		min-width: 0;
		text-align: left;
		font: inherit;
		color: var(--fg);
		background: none;
		border: 0;
		padding: 2px 0;
		cursor: pointer;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.tr-row__title--off {
		color: var(--base6);
		text-decoration: line-through;
	}
	.tr-chip {
		flex-shrink: 0;
		font-size: calc(var(--t-xs) - 1px);
		padding: 0 5px;
		border-radius: 8px;
		border: 1px solid var(--panel-border-strong);
		color: var(--base6);
		white-space: nowrap;
	}
	.tr-chip--skip {
		border-color: color-mix(in srgb, var(--red) 55%, transparent);
		color: var(--red);
	}
	.tr-chip--check {
		border-color: color-mix(in srgb, var(--yellow) 60%, transparent);
		color: var(--yellow);
	}
	.tr-warn {
		color: var(--yellow);
	}
	.tr-main {
		display: flex;
		flex-direction: column;
		min-height: 0;
		min-width: 0;
	}
	.tr-main__title {
		color: var(--fg);
		font-size: var(--t-sm);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.tr-dim {
		color: var(--base6);
	}
	.tr-grow {
		flex: 1 1 auto;
		min-width: 0;
	}
	.tr-sp {
		flex: 1 1 auto;
	}
	.tr-banner {
		display: flex;
		align-items: center;
		gap: 8px;
		margin: 8px 12px 0;
		padding: 6px 8px;
		border: 1px solid color-mix(in srgb, var(--red) 45%, transparent);
		background: color-mix(in srgb, var(--red) 7%, transparent);
		border-radius: var(--r-sm, 3px);
	}
	.tr-banner--note {
		border-color: var(--panel-border-strong);
		background: var(--panel-bg-soft);
	}
	.tr-banner__lead {
		color: var(--red);
		white-space: nowrap;
	}
	.tr-text {
		flex: 1 1 auto;
		min-height: 0;
		overflow-y: auto;
		padding: 12px 16px;
		font-family: var(--font-serif);
		font-size: var(--t-md);
		line-height: 1.7;
		white-space: pre-wrap;
		color: var(--fg);
	}
	.tr-text--off {
		color: var(--base6);
	}
	.tr-span {
		font: inherit;
		color: inherit;
		background: none;
		border: 0;
		padding: 0 1px;
		margin: 0;
		cursor: pointer;
		display: inline;
		white-space: pre-wrap;
		text-align: left;
		border-radius: 2px;
	}
	.tr-span--pending {
		background: color-mix(in srgb, var(--yellow) 18%, transparent);
		border-bottom: 1px solid var(--yellow);
	}
	.tr-span--keep {
		border-bottom: 1px dotted var(--base6);
	}
	.tr-span--skip {
		background: color-mix(in srgb, var(--red) 10%, transparent);
	}
	.tr-span--skip .tr-span__orig,
	.tr-span--replace .tr-span__orig {
		text-decoration: line-through;
		color: var(--base6);
	}
	.tr-span__rep {
		color: var(--cyan);
		margin-left: 4px;
	}
	.tr-span--focus {
		outline: 1px solid var(--blue);
		outline-offset: 1px;
	}
	.tr-card {
		border-top: 1px solid var(--panel-border);
		background: var(--panel-bg-soft);
		padding: 8px 12px;
		display: flex;
		flex-direction: column;
		gap: 6px;
	}
	.tr-card__row {
		display: flex;
		align-items: baseline;
		gap: 8px;
	}
	.tr-card__row--wrap {
		align-items: center;
		flex-wrap: wrap;
	}
	.tr-card__excerpt {
		font-family: var(--font-serif);
		font-size: var(--t-md);
		padding: 1px 6px;
		background: var(--bg-alt);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm, 3px);
		max-width: 360px;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.tr-keys {
		font-size: calc(var(--t-xs) - 1px);
	}
	.tr-btn {
		font: inherit;
		color: var(--fg);
		background: var(--panel-bg-soft);
		border: 1px solid var(--panel-border-strong);
		border-radius: var(--r-sm, 3px);
		padding: 3px 8px;
		cursor: pointer;
		white-space: nowrap;
	}
	.tr-btn--on {
		color: var(--bg);
		background: var(--blue);
		border-color: var(--blue);
	}
	.tr-btn:disabled {
		opacity: 0.5;
		cursor: default;
	}
	.tr-link {
		font: inherit;
		color: var(--blue);
		background: none;
		border: 0;
		padding: 0;
		cursor: pointer;
		text-decoration: underline;
	}
	.tr-input {
		width: 160px;
		font: inherit;
		color: var(--fg);
		background: var(--bg-alt);
		border: 1px solid var(--panel-border-strong);
		border-radius: var(--r-sm, 3px);
		padding: 3px 8px;
	}
	.tr-edit {
		flex: 1 1 auto;
		min-height: 0;
		display: flex;
		flex-direction: column;
		gap: 6px;
		padding: 10px 12px;
	}
	.tr-edit textarea {
		flex: 1 1 auto;
		min-height: 200px;
		resize: none;
		box-sizing: border-box;
		width: 100%;
		background: var(--bg-alt);
		color: var(--fg);
		border: 1px solid var(--panel-border-strong);
		border-radius: var(--r-sm, 3px);
		padding: 10px 12px;
		font-family: var(--font-serif);
		font-size: var(--t-md);
		line-height: 1.6;
	}
	.tr-empty {
		padding: 24px 16px;
	}
	@media (max-width: 720px) {
		.tr {
			grid-template-columns: minmax(0, 1fr);
			grid-template-rows: minmax(120px, 30%) minmax(0, 1fr);
		}
		.tr-list {
			border-right: 0;
			border-bottom: 1px solid var(--panel-border);
		}
	}
</style>
