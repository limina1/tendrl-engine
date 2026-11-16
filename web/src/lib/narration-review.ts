// Transcript review — the narrator's decisions on the engine's flags
// (src/transcript.rs). View/interaction state only: the engine flags the
// spans and resolves the final text; this module turns decisions into the
// `TranscriptOverride` the engine applies, and splits a section's spoken
// text into display segments. Leaf module (no store / registry imports).
import type {
	NarrationPlanNode,
	TranscriptEdit,
	TranscriptFlag,
	TranscriptOverride
} from '$lib/api';

/** `pending` = a review flag nobody has decided on yet (narrated as written). */
export type Decision = 'skip' | 'keep' | 'replace' | 'pending';

export const KIND_LABEL: Record<TranscriptFlag['kind'], string> = {
	footnote_section: 'footnotes',
	footnote_definition: 'footnote',
	footnote_marker: 'note ref',
	citation: 'citation',
	foreign_script: 'other script',
	all_caps: 'capitals',
	number_dense: 'numbers',
	symbols: 'variable',
	url: 'web address',
	markup: 'markup',
	run_on: 'run-on'
};

export const flagKey = (coord: string, i: number) => `${coord}|${i}`;

export function decisionOf(f: TranscriptFlag, coord: string, i: number, decisions: Record<string, Decision>): Decision {
	return decisions[flagKey(coord, i)] ?? (f.action === 'skip' ? 'skip' : 'pending');
}

export function replacementOf(f: TranscriptFlag, coord: string, i: number, reps: Record<string, string>): string {
	return reps[flagKey(coord, i)] ?? f.suggest ?? '';
}

/** The edits a section's decisions amount to (byte offsets, engine-applied). */
export function editsFor(
	node: NarrationPlanNode,
	decisions: Record<string, Decision>,
	reps: Record<string, string>
): TranscriptEdit[] {
	const out: TranscriptEdit[] = [];
	(node.review?.flags ?? []).forEach((f, i) => {
		const d = decisionOf(f, node.coord, i, decisions);
		if (d === 'skip') out.push({ start: f.start, end: f.end, with: '' });
		else if (d === 'replace') out.push({ start: f.start, end: f.end, with: replacementOf(f, node.coord, i, reps) });
	});
	return out;
}

/** Per-section overrides for estimate / generate: a hand-edited text wins;
 *  otherwise the decisions' edits (sections with none are left out). */
export function overridesFor(
	nodes: NarrationPlanNode[],
	decisions: Record<string, Decision>,
	reps: Record<string, string>,
	handText: Record<string, string>
): Record<string, TranscriptOverride> {
	const out: Record<string, TranscriptOverride> = {};
	for (const n of nodes) {
		if (n.is_index || n.spoken == null) continue;
		const hand = handText[n.coord];
		if (hand !== undefined) {
			out[n.coord] = { event_id: n.event_id ?? null, text: hand };
			continue;
		}
		const edits = editsFor(n, decisions, reps);
		if (edits.length) out[n.coord] = { event_id: n.event_id ?? null, edits };
	}
	return out;
}

/** Skips and unresolved checks in a section. A hand-edited section has
 *  none: its text replaces the decisions (pass `handText`). */
export function countsFor(
	node: NarrationPlanNode,
	decisions: Record<string, Decision>,
	handText?: Record<string, string>
): { skips: number; pending: number } {
	if (handText && handText[node.coord] !== undefined) return { skips: 0, pending: 0 };
	let skips = 0;
	let pending = 0;
	(node.review?.flags ?? []).forEach((f, i) => {
		const d = decisionOf(f, node.coord, i, decisions);
		if (d === 'skip') skips++;
		else if (d === 'pending') pending++;
	});
	return { skips, pending };
}

export interface Segment {
	text: string;
	/** Index into the section's flags; absent = plain text. */
	flag?: number;
}

/** Split spoken text at the flags (UTF-16 offsets). Flags arrive sorted by
 *  start; one overlapping an earlier flag stays part of that one. */
export function segmentsOf(spoken: string, flags: TranscriptFlag[]): Segment[] {
	const out: Segment[] = [];
	let cursor = 0;
	flags.forEach((f, i) => {
		if (f.js_start < cursor || f.js_end <= f.js_start) return;
		if (f.js_start > cursor) out.push({ text: spoken.slice(cursor, f.js_start) });
		out.push({ text: spoken.slice(f.js_start, f.js_end), flag: i });
		cursor = f.js_end;
	});
	if (cursor < spoken.length) out.push({ text: spoken.slice(cursor) });
	return out;
}
