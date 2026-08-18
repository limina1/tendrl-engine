<script lang="ts">
	// Highlight mode's capture surface (worksheet C3): mounted once per
	// reader/doc buffer. While `app.highlightMode` is on, completing a text
	// selection inside a `[data-section-addr]` container opens an
	// Alexandria-style confirm surface — editable selected text, optional
	// annotation (quote highlight), optional NIP-84 context, post/cancel.
	// Off = completely inert.
	//
	// Two capture triggers: `mouseup` (desktop, immediate) and a debounced
	// `selectionchange` — touch selection (long-press + drag handles) never
	// fires mouseup, so without the latter the mobile popover only appeared
	// on the NEXT stray tap. The surface is a viewport-centered card on both
	// shells (no backdrop — the selection stays visible); mobile only sizes
	// it up for touch.
	//
	// Offsets (worksheet C2, option 1): the rendered DOM is not
	// offset-faithful — nostrdown refs render as chips whose text differs
	// from their source span — so endpoints map through the
	// `data-src-start`/`data-src-end` stamps RichContent emits. Content
	// rendered as plain text (no stamps, e.g. DocBuffer's body) falls back
	// to a text walk from the section container, which is exact there. The
	// highlighted text is sliced from the SOURCE content, never from
	// `selection.toString()`, so the offset tag and the event content agree
	// byte-for-byte (the engine rejects mismatches at write time). The text
	// box is editable; an edited highlight no longer slices the source, so
	// it publishes WITHOUT the offset pin (the engine accepts offset-less
	// highlights — only a supplied offset is verified).
	import * as api from '$lib/api';
	import { getAppState } from '$lib/state.svelte';
	import { identityCanSign } from '$lib/identity/signer';
	import { shell } from '$lib/wm/shell.svelte';

	let {
		getContent,
		onposted = undefined
	}: {
		/** Resolve a section address to its raw source content (and optionally
		 *  the exact event id the host rendered, as the offset pin — omit and
		 *  the engine pins its local latest). */
		getContent: (addr: string) => { content: string; eventId?: string } | null;
		onposted?: () => void;
	} = $props();

	const app = getAppState();
	const canSign = $derived(identityCanSign(app.identityStatus));
	const mobile = $derived(shell.mode === 'mobile');

	type Pending = {
		addr: string;
		eventId?: string;
		content: string; // full section content
		start: number;
		end: number;
		text: string; // content.slice(start, end) — the pristine capture
	};
	let pending = $state<Pending | null>(null);
	let text = $state(''); // editable copy of pending.text
	let annotation = $state('');
	let contextOpen = $state(false);
	let contextText = $state('');
	/** Sheet hidden, next selection in the same section becomes the context. */
	let selectingContext = $state(false);
	/** The user has interacted with the surface — from then on it holds its
	 *  capture. Until then a new/adjusted selection replaces it, so mobile
	 *  handle-dragging keeps working after the long-press word selection
	 *  already opened the sheet. */
	let touched = $state(false);
	let posting = $state(false);

	const edited = $derived(pending !== null && text.trim() !== pending.text);
	const canPost = $derived(pending !== null && !posting && text.trim().length >= 3);

	/** Map a selection endpoint to a UTF-16 offset into the section source.
	 *  `edge` decides which way an endpoint inside an atomic chip clamps so
	 *  the chip's replaced span is never half-included. */
	function offsetAt(
		container: HTMLElement,
		node: Node,
		offsetInNode: number,
		edge: 'start' | 'end'
	): number | null {
		// Normalize element-node endpoints (offset = child index) to a
		// concrete child, so the walks below see a text position.
		if (node.nodeType === Node.ELEMENT_NODE) {
			const child = node.childNodes[offsetInNode] ?? node.childNodes[node.childNodes.length - 1];
			if (child) {
				node = child;
				offsetInNode = 0;
			}
		}
		const el =
			node.nodeType === Node.TEXT_NODE ? node.parentElement : (node as HTMLElement | null);
		const seg = el?.closest<HTMLElement>('[data-src-start]');
		if (seg && container.contains(seg)) {
			const srcStart = Number.parseInt(seg.dataset.srcStart ?? '', 10);
			if (!Number.isFinite(srcStart)) return null;
			// Atomic chip/card (differs from its source text): clamp so the
			// selection excludes the chip's replaced span.
			if (seg.dataset.srcEnd !== undefined) {
				const srcEnd = Number.parseInt(seg.dataset.srcEnd, 10);
				return edge === 'start' ? srcEnd : srcStart;
			}
			// Exact-text segment: DOM text == content.slice(srcStart, srcEnd),
			// and DOM CharacterData offsets are UTF-16 code units already.
			let sum = 0;
			const walker = document.createTreeWalker(seg, NodeFilter.SHOW_TEXT);
			let n: Node | null;
			while ((n = walker.nextNode())) {
				if (n === node) return srcStart + sum + offsetInNode;
				sum += (n.textContent ?? '').length;
			}
			return srcStart + sum;
		}
		// No stamps — plain-text rendering (DocBuffer body): every text node
		// is a verbatim slice of the source, so a walk from the container is
		// exact.
		let sum = 0;
		const walker = document.createTreeWalker(container, NodeFilter.SHOW_TEXT);
		let n: Node | null;
		while ((n = walker.nextNode())) {
			if (n === node) return sum + offsetInNode;
			sum += (n.textContent ?? '').length;
		}
		return null;
	}

	/** NIP-84 auto-context — emitted SPARINGLY. Several clients (Amethyst)
	 *  render the `context` tag as the quote body, so a wide context displays
	 *  as if far more than the selection were highlighted. It's only genuinely
	 *  needed when the selected text repeats in the section (so a reader that
	 *  ignores our offset tag can still pick the right occurrence), and even
	 *  then a tight sentence window suffices — never the whole paragraph.
	 *  An explicit user-entered context always wins over this. */
	function contextAround(content: string, start: number, end: number): string | undefined {
		const text = content.slice(start, end).trim().toLowerCase();
		if (!text) return undefined;
		const hay = content.toLowerCase();
		// Unique in the section → content + offset identify it; no context.
		if (hay.indexOf(text) === hay.lastIndexOf(text)) return undefined;

		// Sentence-ish window around the selection, tightly capped.
		const MAX_PAD = 80;
		const isBoundary = (ch: string) => ch === '\n' || ch === '.' || ch === '!' || ch === '?';
		let from = start;
		for (let i = start - 1; i >= Math.max(0, start - MAX_PAD); i--) {
			if (isBoundary(content[i])) break;
			from = i;
		}
		let to = end;
		for (let i = end; i < Math.min(content.length, end + MAX_PAD); i++) {
			to = i + 1;
			if (isBoundary(content[i])) break;
		}
		const ctx = content.slice(from, to).trim();
		// Still useless if it adds nothing beyond the selection itself.
		if (!ctx || ctx.toLowerCase() === text) return undefined;
		return ctx;
	}

	/** Resolve the current DOM selection to a section-sourced slice. */
	function resolveSelection(): {
		addr: string;
		eventId?: string;
		content: string;
		start: number;
		end: number;
		text: string;
	} | null {
		const sel = window.getSelection();
		if (!sel || sel.isCollapsed || sel.rangeCount === 0) return null;
		const range = sel.getRangeAt(0);
		const startEl =
			range.startContainer.nodeType === Node.TEXT_NODE
				? range.startContainer.parentElement
				: (range.startContainer as HTMLElement | null);
		const container = startEl?.closest<HTMLElement>('[data-section-addr]');
		if (!container) return null;
		const addr = container.dataset.sectionAddr;
		if (!addr) return null;
		// Both endpoints must live in the same section — cross-section
		// highlights have no single source string.
		const endEl =
			range.endContainer.nodeType === Node.TEXT_NODE
				? range.endContainer.parentElement
				: (range.endContainer as HTMLElement | null);
		if (endEl?.closest('[data-section-addr]') !== container) return null;

		const info = getContent(addr);
		if (!info) return null;
		let start = offsetAt(container, range.startContainer, range.startOffset, 'start');
		let end = offsetAt(container, range.endContainer, range.endOffset, 'end');
		if (start === null || end === null) return null;
		if (start > end) [start, end] = [end, start];
		// Trim whitespace off the selection edges, offsets adjusted in step
		// so the slice==content invariant (which the engine enforces) holds.
		while (start < end && /\s/.test(info.content[start])) start++;
		while (end > start && /\s/.test(info.content[end - 1])) end--;
		const sliced = info.content.slice(start, end);
		if (sliced.trim().length < 3) return null;

		return {
			addr,
			eventId: info.eventId,
			content: info.content,
			start,
			end,
			text: sliced
		};
	}

	function capture() {
		const hit = resolveSelection();
		if (!hit) return;

		// Context-selection detour: the slice fills the context field instead
		// of starting a new highlight. Same section only — context from a
		// different section isn't "surrounding content".
		if (selectingContext && pending) {
			if (hit.addr !== pending.addr) return;
			contextText = hit.text;
			selectingContext = false;
			window.getSelection()?.removeAllRanges();
			return;
		}

		pending = {
			addr: hit.addr,
			eventId: hit.eventId,
			content: hit.content,
			start: hit.start,
			end: hit.end,
			text: hit.text
		};
		text = hit.text;
		annotation = '';
		contextText = '';
		contextOpen = false;
		touched = false;
	}

	function wantsCapture(): boolean {
		if (!app.highlightMode || !canSign || posting) return false;
		// A capture the user has engaged with holds; an untouched one is
		// replaced by further selection (mobile handle-dragging) — unless
		// we're out collecting context for it.
		if (pending && !selectingContext && touched) return false;
		return true;
	}

	function onMouseUp() {
		if (!wantsCapture()) return;
		// Let the browser finalize the selection first.
		requestAnimationFrame(() => {
			if (wantsCapture()) capture();
		});
	}

	// Touch selection never fires mouseup — settle on selectionchange
	// instead. The debounce spans the drag-handle adjustments so the sheet
	// opens once, when the reader stops moving the handles.
	let selTimer: ReturnType<typeof setTimeout> | undefined;
	function onSelectionChange() {
		if (!wantsCapture()) return;
		clearTimeout(selTimer);
		selTimer = setTimeout(() => {
			if (wantsCapture()) capture();
		}, 500);
	}

	function dismiss() {
		clearTimeout(selTimer);
		pending = null;
		text = '';
		annotation = '';
		contextText = '';
		contextOpen = false;
		selectingContext = false;
		touched = false;
	}

	async function post() {
		if (!pending || !canPost) return;
		const finalText = text.trim();
		// Edited text no longer slices the pinned source — drop the offset
		// (and the offset-derived auto-context) rather than publish a pin the
		// engine would reject.
		const isEdited = finalText !== pending.text;
		posting = true;
		try {
			const resp = await api.publishHighlight({
				target: { address: pending.addr, event_id: pending.eventId },
				content: finalText,
				offset: isEdited ? undefined : [pending.start, pending.end],
				context:
					contextText.trim() ||
					(isEdited ? undefined : contextAround(pending.content, pending.start, pending.end)),
				comment: annotation.trim() || undefined
			});
			const { successful, total } = resp.broadcast;
			app.pushToast(
				total === 0
					? 'Highlight saved locally (no publish relays)'
					: successful === 0
						? `Highlight saved locally — 0/${total} relays accepted`
						: `Highlight published (${successful}/${total} relays)`,
				successful === 0 && total > 0 ? 'error' : 'success'
			);
			window.getSelection()?.removeAllRanges();
			dismiss();
			onposted?.();
		} catch (e) {
			app.pushToast(api.errorMessage(e, 'Highlight failed'), 'error', 5000);
		} finally {
			posting = false;
		}
	}

	function onKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape' && selectingContext) {
			e.preventDefault();
			e.stopPropagation();
			selectingContext = false;
		} else if (e.key === 'Escape' && pending) {
			e.preventDefault();
			e.stopPropagation();
			dismiss();
		} else if ((e.ctrlKey || e.metaKey) && e.key === 'Enter' && pending) {
			e.preventDefault();
			post();
		}
	}

	// Leaving highlight mode drops any half-finished capture.
	$effect(() => {
		if (!app.highlightMode) dismiss();
	});
</script>

<svelte:document onmouseup={onMouseUp} onselectionchange={onSelectionChange} onkeydown={onKeydown} />

{#if pending && selectingContext}
	<div class="hc-ctx-banner" class:hc-ctx-banner--mobile={mobile} role="status">
		<span>select the surrounding text for context</span>
		<button class="hc-btn" onclick={() => (selectingContext = false)}>cancel</button>
	</div>
{:else if pending}
	<div
		class="hc-popover"
		class:hc-popover--mobile={mobile}
		role="dialog"
		aria-label="Publish highlight"
		onfocusin={() => (touched = true)}
	>
		<textarea
			class="hc-text"
			rows={mobile ? 4 : 3}
			bind:value={text}
			disabled={posting}
			aria-label="Highlighted text (editable)"
		></textarea>
		{#if edited}
			<div class="hc-note">edited — will publish without a position pin</div>
		{/if}
		<input
			class="hc-field"
			type="text"
			placeholder="Annotation (optional — makes it a quote highlight)"
			bind:value={annotation}
			disabled={posting}
		/>
		{#if contextOpen}
			<div class="hc-ctx">
				<textarea
					class="hc-field hc-ctx-text"
					rows="2"
					placeholder="Context — surrounding text (paste, or select in the text)"
					bind:value={contextText}
					disabled={posting}
				></textarea>
				<button
					class="hc-btn"
					onclick={() => {
						touched = true;
						selectingContext = true;
					}}
					disabled={posting}
					title="Hide this panel and select the surrounding text in the section"
				>select in text</button>
			</div>
		{/if}
		<div class="hc-foot">
			{#if !contextOpen}
				<button
					class="hc-btn hc-btn--ghost"
					onclick={() => {
						touched = true;
						contextOpen = true;
					}}
					disabled={posting}
					title="Add NIP-84 context — surrounding text that situates the highlight"
				>+ context</button>
			{/if}
			<span class="hc-hint">{text.trim().length} chars{#if !mobile}&nbsp;· Ctrl-Enter{/if}</span>
			<span class="hc-spacer"></span>
			<button class="hc-btn" onclick={dismiss} disabled={posting}>Cancel</button>
			<button class="hc-btn hc-btn--post" onclick={post} disabled={!canPost}>
				{posting ? 'Publishing…' : 'Highlight'}
			</button>
		</div>
	</div>
{/if}

<style>
	/* Centered card on both shells (no backdrop — the selection stays
	   visible behind it). `--kb-inset` (set by the mobile shell) shifts the
	   center up while the soft keyboard is open. */
	.hc-popover {
		position: fixed;
		z-index: 300;
		left: 50%;
		top: calc(50% - var(--kb-inset, 0px) / 2);
		transform: translate(-50%, -50%);
		width: min(360px, 92vw);
		max-height: calc(100dvh - var(--kb-inset, 0px) - 24px);
		overflow-y: auto;
		background: var(--bg);
		border: 1px solid var(--panel-border-strong, var(--panel-border));
		border-radius: var(--r-sm, 3px);
		box-shadow: var(--shadow-lg, 0 8px 30px rgba(0, 0, 0, 0.4));
		padding: 8px;
		display: flex;
		flex-direction: column;
		gap: 6px;
	}
	.hc-text {
		margin: 0;
		padding: 4px 8px;
		border: 1px solid var(--panel-border);
		border-left: 2px solid var(--id-yours);
		border-radius: var(--r-sm);
		background: var(--bg-surface);
		font-family: var(--font-sans);
		font-size: var(--t-xs);
		color: var(--fg-muted);
		max-height: 96px;
		resize: vertical;
	}
	.hc-text:focus {
		outline: none;
		color: var(--fg);
	}
	.hc-note {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 2px);
		color: var(--base5);
	}
	.hc-field {
		font-family: var(--font-sans);
		font-size: var(--t-xs);
		color: var(--fg);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 4px 8px;
	}
	.hc-field:focus,
	.hc-text:focus {
		outline: none;
		border-color: var(--id-yours);
	}
	.hc-ctx {
		display: flex;
		align-items: flex-end;
		gap: 6px;
	}
	.hc-ctx-text {
		flex: 1;
		resize: vertical;
	}
	.hc-foot {
		display: flex;
		align-items: center;
		gap: 8px;
	}
	.hc-hint {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		color: var(--base5);
	}
	.hc-spacer {
		flex: 1;
	}
	.hc-btn {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		color: var(--base6);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 3px 10px;
		cursor: pointer;
	}
	.hc-btn:hover:not(:disabled) {
		color: var(--fg);
		border-color: var(--base5);
	}
	.hc-btn--post {
		color: var(--id-yours);
	}
	.hc-btn--ghost {
		border-style: dashed;
	}
	.hc-btn:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}

	/* The "go select context" affordance while the panel is hidden. */
	.hc-ctx-banner {
		position: fixed;
		z-index: 300;
		top: 12px;
		left: 50%;
		transform: translateX(-50%);
		display: flex;
		align-items: center;
		gap: 10px;
		background: var(--bg);
		border: 1px solid var(--panel-border-strong, var(--panel-border));
		border-radius: var(--r-sm, 3px);
		box-shadow: var(--shadow-lg, 0 8px 30px rgba(0, 0, 0, 0.4));
		padding: 6px 10px;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--fg-muted);
		white-space: nowrap;
	}

	/* Mobile: same centered card, wider with touch-sized type and targets. */
	.hc-popover--mobile {
		width: min(480px, 94vw);
		border-radius: 8px;
		padding: 14px 16px;
		gap: 12px;
	}
	.hc-popover--mobile .hc-text {
		font-size: var(--t-sm);
		max-height: 30dvh;
		padding: 8px 10px;
	}
	.hc-popover--mobile .hc-field {
		font-size: var(--t-sm);
		padding: 10px 12px;
	}
	.hc-popover--mobile .hc-note,
	.hc-popover--mobile .hc-hint {
		font-size: var(--t-xs);
	}
	.hc-popover--mobile .hc-btn {
		font-size: var(--t-sm);
		padding: 10px 16px;
		min-height: 44px;
	}
	.hc-popover--mobile .hc-foot {
		gap: 10px;
	}
	.hc-ctx-banner--mobile {
		font-size: var(--t-sm);
		padding: 10px 14px;
	}
	.hc-ctx-banner--mobile .hc-btn {
		font-size: var(--t-sm);
		padding: 8px 14px;
		min-height: 40px;
	}
</style>
