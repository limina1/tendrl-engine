import type { MobileNavEntry } from '$lib/wm/mobile-nav.svelte';

// Shallow-routing state carried on history entries (SvelteKit page.state).
// `mnav` is the mobile shell's back-navigation payload — see
// $lib/wm/mobile-nav.svelte.ts.
declare global {
	namespace App {
		interface PageState {
			mnav?: MobileNavEntry;
		}
	}
}

export {};
