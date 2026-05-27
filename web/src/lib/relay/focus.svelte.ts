// One-shot focus signal: when EventViewModal's "Found on" chip is
// clicked, it sets the URL here and opens the relays buffer.
// RelaysBuffer reads + clears this on mount/update so it expands and
// scrolls to the matching row. Cleared after use so a subsequent
// non-focused open of the buffer behaves normally.

export const relayFocus = $state<{ url: string | null }>({ url: null });

export function requestRelayFocus(url: string) {
	relayFocus.url = url;
}

export function consumeRelayFocus(): string | null {
	const url = relayFocus.url;
	relayFocus.url = null;
	return url;
}
