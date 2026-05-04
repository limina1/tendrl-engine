// Pure derivation helpers for compose state.
//
// SectionState classifies each ContextItem by provenance and divergence:
//
//   imported  — has a source addr, locked, content matches original.
//               No new 30041 on publish — TOC references the source addr.
//   claimed   — has a source addr, unlocked, content unchanged.
//               Same publish behavior as imported (transclusion); the
//               unlocked state is a UX signal that may trigger a
//               confirmation popup before publish.
//   forked    — has a source addr, content has diverged from the original.
//               Publishes a new 30041 carrying fork-marker `a`/`e` tags.
//   original  — no source addr at all (authored fresh in this draft).
//               Publishes a plain 30041 with no fork lineage.

import type { ComposeState, ContextItem, SectionState, NAddr } from '$lib/types';

export function sectionState(item: ContextItem): SectionState {
	if (!item.source_addr) return 'original';
	if (item.content !== item.original_content) return 'forked';
	return item.readonly ? 'imported' : 'claimed';
}

/** Did the user change anything that warrants a new 30040?
 *
 * Triggers:
 *   - Section count differs from the source publication's TOC.
 *   - Any source addr is missing (a section was inserted) or out of order.
 *   - Any section is forked (content diverged).
 *   - Any section is original (authored fresh in this draft).
 *
 * Note: claimed (unlocked but untouched) does NOT trigger by itself —
 * unlocking without modifying is a UX warning, not a publish trigger.
 *
 * If there is no source publication at all (a from-scratch draft), the
 * mere presence of any section counts as structural change. */
export function hasStructuralChange(state: ComposeState): boolean {
	if (state.sections.length === 0) return false;
	const order = state.source_section_order ?? [];

	// From-scratch draft: any non-empty content is a publishable change.
	if (!state.source_publication_addr) {
		return state.sections.some((s) => s.content.trim().length > 0);
	}

	// Seeded draft: compare against the original 30040's section order.
	if (state.sections.length !== order.length) return true;

	for (let i = 0; i < state.sections.length; i++) {
		const item = state.sections[i];
		const orig = order[i];
		const st = sectionState(item);
		if (st === 'forked' || st === 'original') return true;
		if (!item.source_addr) return true;
		if (!naddrEq(item.source_addr, orig)) return true;
	}
	return false;
}

/** Sections that have been unlocked but not modified — used to trigger a
 * "publish anyway?" confirmation. */
export function claimedUntouchedSections(state: ComposeState): ContextItem[] {
	return state.sections.filter((s) => sectionState(s) === 'claimed');
}

/** Group consecutive imported sections into single movable segments.
 *
 * Reorder rule (per the design): runs of imported sections always travel
 * together (you don't reorder transcluded content of someone else's work
 * mid-stream). Claimed/forked/original sections are atomic — each is its
 * own segment.
 *
 * Returned segments preserve original indices so drag-handlers can splice
 * the underlying array. */
export interface SectionSegment {
	indices: number[];
	state: SectionState;
}

export function segmentSections(state: ComposeState): SectionSegment[] {
	const segments: SectionSegment[] = [];
	let run: number[] = [];
	for (let i = 0; i < state.sections.length; i++) {
		const st = sectionState(state.sections[i]);
		if (st === 'imported') {
			run.push(i);
		} else {
			if (run.length) {
				segments.push({ indices: run, state: 'imported' });
				run = [];
			}
			segments.push({ indices: [i], state: st });
		}
	}
	if (run.length) segments.push({ indices: run, state: 'imported' });
	return segments;
}

function naddrEq(a: NAddr, b: NAddr | undefined): boolean {
	if (!b) return false;
	return a.kind === b.kind && a.pubkey === b.pubkey && a.d_tag === b.d_tag;
}
