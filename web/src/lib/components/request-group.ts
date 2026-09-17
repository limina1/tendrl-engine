// Group a section's answer notes under the request they answer. A note
// carries `request` (the request coordinate, engine-derived from its tag);
// a request read from its signed event carries `coordinate`. Notes that
// answer a request shown here fold under its card; everything else stays a
// free-standing card. Pure.
import type { AnnotationRequest, LoadedInsertion } from '$lib/api';

export interface RequestGroups {
	/** request id → the notes that answer it, in load order. */
	answers: Map<string, LoadedInsertion[]>;
	/** Notes with no request on this section. */
	rest: LoadedInsertion[];
}

export function groupAnswers(asks: AnnotationRequest[], notes: LoadedInsertion[]): RequestGroups {
	const byCoord = new Map<string, string>();
	for (const r of asks) if (r.coordinate) byCoord.set(r.coordinate, r.id);
	const answers = new Map<string, LoadedInsertion[]>();
	const rest: LoadedInsertion[] = [];
	for (const n of notes) {
		const id = n.request ? byCoord.get(n.request) : undefined;
		if (id === undefined) {
			rest.push(n);
			continue;
		}
		const list = answers.get(id) ?? [];
		list.push(n);
		answers.set(id, list);
	}
	return { answers, rest };
}
