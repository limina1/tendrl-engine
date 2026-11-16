// Highlight display settings — a per-device view preference (localStorage,
// never the engine): one alpha for every highlight mark, so overlapping
// highlights blend legibly (docs/highlighters-contract.org).

import { browser } from '$app/environment';

const STORAGE_KEY = 'tendrl.highlights';
export const ALPHA_MIN = 10;
export const ALPHA_MAX = 80;
export const ALPHA_DEFAULT = 38;

function load(): number {
	if (!browser) return ALPHA_DEFAULT;
	try {
		const raw = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '{}');
		const a = Number(raw.alpha);
		return Number.isFinite(a) ? Math.min(ALPHA_MAX, Math.max(ALPHA_MIN, a)) : ALPHA_DEFAULT;
	} catch {
		return ALPHA_DEFAULT;
	}
}

class HighlightDisplay {
	/** Percent, ALPHA_MIN..ALPHA_MAX. */
	alpha = $state(load());

	setAlpha(pct: number) {
		this.alpha = Math.min(ALPHA_MAX, Math.max(ALPHA_MIN, Math.round(pct)));
		if (!browser) return;
		try {
			localStorage.setItem(STORAGE_KEY, JSON.stringify({ alpha: this.alpha }));
		} catch {
			/* private mode — the setting just doesn't persist */
		}
	}
}

export const highlightDisplay = new HighlightDisplay();
