// The highlighter the next capture starts with — session memory, a leaf
// module (no component imports). Set by the capture picker when a highlight
// is published, and by the drawer's "Highlight with this style" on someone
// else's highlight. Never persisted, never published on its own.

import type { Highlighter } from './highlights';

class HighlighterChoice {
	current = $state<Highlighter | null>(null);

	set(h: Highlighter | null) {
		this.current = h ? { label: h.label, color: h.color, marks: h.marks ? [...h.marks] : undefined } : null;
	}
}

export const highlighterChoice = new HighlighterChoice();
