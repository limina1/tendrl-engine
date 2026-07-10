<script lang="ts">
	// Highlight mode's capture surface (worksheet C3): mounted once per
	// reader/doc buffer. While `app.highlightMode` is on, completing a text
	// selection inside a `[data-section-addr]` container opens an
	// Alexandria-style confirm popover — selected-text preview, optional
	// annotation (quote highlight), post/cancel. Off = completely inert.
	//
	// Offsets (worksheet C2, option 1): the rendered DOM is not
	// offset-faithful — nostrdown refs render as chips whose text differs
	// from their source span — so endpoints map through the
	// `data-src-start`/`data-src-end` stamps RichContent emits. Content
	// rendered as plain text (no stamps, e.g. DocBuffer's body) falls back
	// to a text walk from the section container, which is exact there. The
	// highlighted text is sliced from the SOURCE content, never from
	// `selection.toString()`, so the offset tag and the event content agree
	// byte-for-byte (the engine rejects mismatches at write time).
	import * as api from '$lib/api';
	import { getAppState } from '$lib/state.svelte';
	import { identityCanSign } from '$lib/identity/signer';

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

	type Pending = {
		addr: string;
		eventId?: string;
		content: string; // full section content
		start: number;
		end: number;
		text: string; // content.slice(start, end)
		x: number;
		y: number;
	};
	let pending = $state<Pending | null>(null);
	let annotation = $state('');
	let posting = $state(false);

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

	/** NIP-84 context — emitted SPARINGLY. Several clients (Amethyst) render
	 *  the `context` tag as the quote body, so a wide context displays as if
	 *  far more than the selection were highlighted. It's only genuinely
	 *  needed when the selected text repeats in the section (so a reader that
	 *  ignores our offset tag can still pick the right occurrence), and even
	 *  then a tight sentence window suffices — never the whole paragraph. */
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

	function onMouseUp() {
		if (!app.highlightMode || pending || !canSign) return;
		// Let the browser finalize the selection first.
		requestAnimationFrame(() => {
			const sel = window.getSelection();
			if (!sel || sel.isCollapsed || sel.rangeCount === 0) return;
			const range = sel.getRangeAt(0);
			const startEl =
				range.startContainer.nodeType === Node.TEXT_NODE
					? range.startContainer.parentElement
					: (range.startContainer as HTMLElement | null);
			const container = startEl?.closest<HTMLElement>('[data-section-addr]');
			if (!container) return;
			const addr = container.dataset.sectionAddr;
			if (!addr) return;
			// Both endpoints must live in the same section — cross-section
			// highlights have no single source string.
			const endEl =
				range.endContainer.nodeType === Node.TEXT_NODE
					? range.endContainer.parentElement
					: (range.endContainer as HTMLElement | null);
			if (endEl?.closest('[data-section-addr]') !== container) return;

			const info = getContent(addr);
			if (!info) return;
			let start = offsetAt(container, range.startContainer, range.startOffset, 'start');
			let end = offsetAt(container, range.endContainer, range.endOffset, 'end');
			if (start === null || end === null) return;
			if (start > end) [start, end] = [end, start];
			// Trim whitespace off the selection edges, offsets adjusted in step
			// so the slice==content invariant (which the engine enforces) holds.
			while (start < end && /\s/.test(info.content[start])) start++;
			while (end > start && /\s/.test(info.content[end - 1])) end--;
			const text = info.content.slice(start, end);
			if (text.trim().length < 3) return;

			const rect = range.getBoundingClientRect();
			pending = {
				addr,
				eventId: info.eventId,
				content: info.content,
				start,
				end,
				text,
				x: Math.max(8, Math.min(rect.left + rect.width / 2 - 160, window.innerWidth - 336)),
				y: Math.min(rect.bottom + 8, window.innerHeight - 180)
			};
			annotation = '';
		});
	}

	function dismiss() {
		pending = null;
		annotation = '';
	}

	async function post() {
		if (!pending || posting) return;
		posting = true;
		try {
			const resp = await api.publishHighlight({
				target: { address: pending.addr, event_id: pending.eventId },
				content: pending.text,
				offset: [pending.start, pending.end],
				context: contextAround(pending.content, pending.start, pending.end),
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
		if (e.key === 'Escape' && pending) {
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

	function short(s: string, n: number): string {
		return s.length > n ? `${s.slice(0, n)}…` : s;
	}
</script>

<svelte:document onmouseup={onMouseUp} onkeydown={onKeydown} />

{#if pending}
	<div class="hc-popover" style="left: {pending.x}px; top: {pending.y}px" role="dialog" aria-label="Publish highlight">
		<blockquote class="hc-preview">{short(pending.text, 180)}</blockquote>
		<!-- svelte-ignore a11y_autofocus -->
		<input
			class="hc-annotation"
			type="text"
			placeholder="Annotation (optional — makes it a quote highlight)"
			bind:value={annotation}
			disabled={posting}
		/>
		<div class="hc-foot">
			<span class="hc-hint">{pending.end - pending.start} chars · Ctrl-Enter</span>
			<span class="hc-spacer"></span>
			<button class="hc-btn" onclick={dismiss} disabled={posting}>Cancel</button>
			<button class="hc-btn hc-btn--post" onclick={post} disabled={posting}>
				{posting ? 'Publishing…' : 'Highlight'}
			</button>
		</div>
	</div>
{/if}

<style>
	.hc-popover {
		position: fixed;
		z-index: 300;
		width: min(320px, 90vw);
		background: var(--bg);
		border: 1px solid var(--panel-border-strong, var(--panel-border));
		border-radius: var(--r-sm, 3px);
		box-shadow: var(--shadow-lg, 0 8px 30px rgba(0, 0, 0, 0.4));
		padding: 8px;
		display: flex;
		flex-direction: column;
		gap: 6px;
	}
	.hc-preview {
		margin: 0;
		padding: 4px 8px;
		border-left: 2px solid var(--id-yours);
		font-size: var(--t-xs);
		color: var(--fg-muted);
		max-height: 72px;
		overflow: hidden;
	}
	.hc-annotation {
		font-family: var(--font-sans);
		font-size: var(--t-xs);
		color: var(--fg);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 4px 8px;
	}
	.hc-annotation:focus {
		outline: none;
		border-color: var(--id-yours);
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
	.hc-btn:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}
</style>
