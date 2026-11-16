// The highlight being captured, painted live in the reader while its style
// is picked — a leaf module. HighlightCapture sets the target (section
// content + UTF-16 offsets) when a selection opens the capture and clears it
// on dismiss/post; RichContent adds a synthetic span over that run in the
// current `highlighterChoice` style. Never published.

class HighlightPreview {
	target = $state<{ content: string; start: number; end: number } | null>(null);
}

export const highlightPreview = new HighlightPreview();
