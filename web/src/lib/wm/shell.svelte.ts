/**
 * Shell selection — which shell renders the app: the desktop WM (class
 * slots, splits, modeline, leader) or the mobile shell (bottom-bar class
 * panels, work rail). Both shells consume the same BufferStore; only the
 * rendering differs, so this is presentation state, not app state.
 *
 * Resolution order: `?shell=` URL param (session-only preview) → persisted
 * preference (Settings / the cycle-shell command) → viewport auto-detect.
 *
 * Leaf module by design — imports nothing from wm/ (registry import cycles
 * are a build-order TDZ hazard, see wm/tours.ts).
 */

export type ShellPref = 'auto' | 'desktop' | 'mobile';
export type ShellMode = 'desktop' | 'mobile';

const STORAGE_KEY = 'tendrl:shell';
const MOBILE_QUERY = '(max-width: 768px)';

class ShellState {
	pref = $state<ShellPref>('auto');
	private compact = $state(false);
	/** Session-only override from `?shell=` — wins over the stored pref,
	 *  never persisted; cleared by an explicit setPref. */
	private urlOverride = $state<ShellMode | null>(null);

	constructor() {
		if (typeof window === 'undefined') return;
		const stored = localStorage.getItem(STORAGE_KEY);
		if (stored === 'desktop' || stored === 'mobile' || stored === 'auto') this.pref = stored;
		const param = new URLSearchParams(window.location.search).get('shell');
		if (param === 'desktop' || param === 'mobile') this.urlOverride = param;
		const mq = window.matchMedia(MOBILE_QUERY);
		this.compact = mq.matches;
		mq.addEventListener('change', (e) => (this.compact = e.matches));
	}

	get mode(): ShellMode {
		if (this.urlOverride) return this.urlOverride;
		if (this.pref !== 'auto') return this.pref;
		return this.compact ? 'mobile' : 'desktop';
	}

	setPref(p: ShellPref) {
		this.pref = p;
		this.urlOverride = null;
		if (typeof window !== 'undefined') localStorage.setItem(STORAGE_KEY, p);
	}
}

export const shell = new ShellState();
