<script lang="ts">
	// The refresh/fetch glyph as an SVG that spins about its own centre.
	// A text glyph (⟳ / ↻) sits off-centre (font metrics), and spinning the
	// button rotated the whole box — spin this instead, inside a button
	// that centres it (inline-flex).
	let { spinning = false, size = 13 }: { spinning?: boolean; size?: number } = $props();
</script>

<svg
	class="refresh-icon"
	class:refresh-icon--spin={spinning}
	viewBox="0 0 16 16"
	width={size}
	height={size}
	aria-hidden="true"
>
	<path
		d="M13.2 8a5.2 5.2 0 1 1-1.52-3.68"
		fill="none"
		stroke="currentColor"
		stroke-width="1.6"
		stroke-linecap="round"
	/>
	<path d="M13.6 2.2v3.6H10" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />
</svg>

<style>
	.refresh-icon {
		display: block;
		flex-shrink: 0;
		transform-box: fill-box;
		transform-origin: 50% 50%;
	}
	.refresh-icon--spin {
		animation: refresh-icon-spin 0.9s linear infinite;
	}
	@keyframes refresh-icon-spin {
		to {
			transform: rotate(360deg);
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.refresh-icon--spin {
			animation-duration: 2.4s;
		}
	}
</style>
