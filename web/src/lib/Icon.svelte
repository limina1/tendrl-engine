<script lang="ts" module>
	export type IconName =
		| 'chevron'
		| 'search'
		| 'chat'
		| 'doc'
		| 'compose'
		| 'settings'
		| 'x'
		| 'plus'
		| 'refresh'
		| 'filter'
		| 'hash'
		| 'link'
		| 'more'
		| 'eye'
		| 'eye-off'
		| 'send'
		| 'check'
		| 'lock'
		| 'fork'
		| 'trash'
		| 'tag'
		| 'sparkle'
		| 'drag'
		| 'folder'
		| 'user'
		| 'arrow-left'
		| 'arrow-right'
		| 'quote';

	const ICONS: Record<IconName, string> = {
		chevron: '<path d="M9 6l6 6-6 6" />',
		search: '<circle cx="11" cy="11" r="7" /><path d="m20 20-3.5-3.5" />',
		chat: '<path d="M21 12a8 8 0 0 1-11.5 7.2L4 21l1.8-5.5A8 8 0 1 1 21 12z" />',
		doc: '<path d="M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM14 3v6h6" />',
		compose:
			'<path d="M12 20h9" /><path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4z" />',
		settings:
			'<circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />',
		x: '<path d="M6 6l12 12M18 6L6 18" />',
		plus: '<path d="M12 5v14M5 12h14" />',
		refresh:
			'<path d="M3 12a9 9 0 0 1 15.5-6.3L21 8" /><path d="M21 3v5h-5" /><path d="M21 12a9 9 0 0 1-15.5 6.3L3 16" /><path d="M3 21v-5h5" />',
		filter: '<path d="M22 3H2l8 9.46V19l4 2v-8.54z" />',
		hash: '<path d="M4 9h16M4 15h16M10 3 8 21M16 3l-2 18" />',
		link:
			'<path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.72" /><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.72-1.72" />',
		more:
			'<circle cx="12" cy="5" r="1" fill="currentColor" /><circle cx="12" cy="12" r="1" fill="currentColor" /><circle cx="12" cy="19" r="1" fill="currentColor" />',
		eye:
			'<path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z" /><circle cx="12" cy="12" r="3" />',
		'eye-off':
			'<path d="M17.94 17.94A10.07 10.07 0 0 1 12 20C5 20 1 12 1 12a18.5 18.5 0 0 1 5.06-5.94M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19" /><path d="M14.12 14.12a3 3 0 1 1-4.24-4.24" /><path d="M1 1l22 22" />',
		send: '<path d="M22 2L11 13M22 2l-7 20-4-9-9-4z" />',
		check: '<path d="M20 6 9 17l-5-5" />',
		lock:
			'<rect x="3" y="11" width="18" height="11" rx="2" /><path d="M7 11V7a5 5 0 0 1 10 0v4" />',
		fork:
			'<circle cx="6" cy="3" r="2" /><circle cx="6" cy="21" r="2" /><circle cx="18" cy="6" r="2" /><path d="M6 5v8a4 4 0 0 0 4 4h4M6 13a4 4 0 0 1 4-4h4M18 8v0a4 4 0 0 1-4 4" />',
		trash:
			'<path d="M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2m3 0v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6" />',
		tag:
			'<path d="M20.59 13.41 13.42 20.58a2 2 0 0 1-2.83 0L2 12V2h10l8.59 8.59a2 2 0 0 1 0 2.82z" /><circle cx="7" cy="7" r="1" fill="currentColor" />',
		sparkle:
			'<path d="M12 3v4M12 17v4M3 12h4M17 12h4M5.6 5.6l2.8 2.8M15.6 15.6l2.8 2.8M5.6 18.4l2.8-2.8M15.6 8.4l2.8-2.8" />',
		drag:
			'<circle cx="9" cy="6" r="1" fill="currentColor" /><circle cx="9" cy="12" r="1" fill="currentColor" /><circle cx="9" cy="18" r="1" fill="currentColor" /><circle cx="15" cy="6" r="1" fill="currentColor" /><circle cx="15" cy="12" r="1" fill="currentColor" /><circle cx="15" cy="18" r="1" fill="currentColor" />',
		folder: '<path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />',
		user:
			'<path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2" /><circle cx="12" cy="7" r="4" />',
		'arrow-left': '<path d="M19 12H5M12 19l-7-7 7-7" />',
		'arrow-right': '<path d="M5 12h14M12 5l7 7-7 7" />',
		quote:
			'<path d="M3 21c0-7 4-11 8-11M3 21V13a4 4 0 0 1 4-4M13 21c0-7 4-11 8-11M13 21V13a4 4 0 0 1 4-4" />'
	};

	export { ICONS };
</script>

<script lang="ts">
	type Direction = 'right' | 'left' | 'up' | 'down';

	type Props = {
		name: IconName;
		size?: number;
		strokeWidth?: number;
		dir?: Direction;
	};

	let { name, size = 14, strokeWidth = 1.5, dir = 'right' }: Props = $props();

	const rotation: Record<Direction, number> = { right: 0, left: 180, up: -90, down: 90 };
	const rot = $derived(name === 'chevron' ? rotation[dir] : 0);
	const inner = $derived(ICONS[name] ?? '');
</script>

<svg
	width={size}
	height={size}
	viewBox="0 0 24 24"
	fill="none"
	stroke="currentColor"
	stroke-width={strokeWidth}
	stroke-linecap="round"
	stroke-linejoin="round"
	style={rot ? `transform: rotate(${rot}deg)` : undefined}
>
	{@html inner}
</svg>
