/**
 * Mobile-shell back navigation — Android-grade Back over SvelteKit shallow
 * routing. The mobile shell pushes one history entry per panel/buffer
 * navigation (any source: bottom bar, drawer, palette, renderer openBuffer)
 * and per teleport-class view transition inside a buffer (view-mode switch,
 * TOC jump — see ViewPlace below and docs/zettel/idea-place-routing.org);
 * Back first closes the topmost open overlay, then walks those entries, and
 * at the baseline falls through to native page exit (which is what lets an
 * Android WebView leave the app).
 *
 * Kit's router owns popstate in this SPA, so entries go through
 * pushState/replaceState from $app/navigation and come back reactively via
 * page.state — never window.history directly. `pushState('', state)`
 * resolves '' against the current URL, so ?shell= survives every entry.
 *
 * Echo/loop prevention is structural, not flag-based: every entry carries a
 * monotonic `seq` (our own push echoing back through page.state is detected
 * by seq equality), and traversals set `current` BEFORE mutating the store,
 * so the central watcher's equality guard suppresses a re-push.
 *
 * Desktop stays history-free: the watchers live in MobileShell, which only
 * mounts when shell.mode === 'mobile'. After a mobile→desktop flip, stale
 * mnav entries remain in history but nobody watches page.state — Back
 * no-ops through them and then exits; re-entering mobile re-baselines.
 *
 * Leaf module: runtime imports are $app only; wm imports are type-only.
 */

import { pushState, replaceState } from '$app/navigation';
import { page } from '$app/state';
import type { ClassName } from './types';
import type { BufferStore } from './buffer-store.svelte';

export type MobileNavEntry = {
	/** Per-page-load session id. History can hold same-URL entries from
	 *  PREVIOUS loads of this app (reused browser tab); without the sid,
	 *  an old entry's seq can collide with ours and get misread as an
	 *  echo. Foreign-sid entries are absorbed instead (see syncFromHistory). */
	sid: string;
	/** Monotonic per-session; detects our own push echoing back. */
	seq: number;
	/** Active class panel. */
	cls: ClassName;
	/** The work slot's focused buffer id — tracked even while another class
	 *  is active, so Back restores the work panel as the user left it. */
	workBuf: string | null;
	/** Buffer-level place (docs/zettel/idea-place-routing.org): a small
	 *  serializable descriptor the focused work buffer contributes via its
	 *  ViewProvider (e.g. reader {mode, section}). Teleport-class view
	 *  transitions push entries carrying it; traversal updates it in place. */
	view?: ViewPlace | null;
};

/** String-only so entries stay structured-cloneable for history state and
 *  (phase 2) serialize losslessly into the hash route. */
export type ViewPlace = Record<string, string>;

export type ViewProvider = {
	/** Snapshot the buffer's current place; null = nothing worth recording. */
	capture: () => ViewPlace | null;
	/** Restore a previously captured place (best-effort — content may still
	 *  be loading; appliers must tolerate that). */
	apply: (v: ViewPlace) => void;
};

function viewEq(a: ViewPlace | null | undefined, b: ViewPlace | null | undefined): boolean {
	if (!a || !b) return !a === !b;
	const ka = Object.keys(a);
	return ka.length === Object.keys(b).length && ka.every((k) => a[k] === b[k]);
}

/** One per page load — entries carrying another sid are from a previous
 *  load's history region. */
const SID = typeof crypto !== 'undefined' && crypto.randomUUID
	? crypto.randomUUID().slice(0, 8)
	: String(Date.now());

export type BackCloser = {
	isOpen: () => boolean;
	close: () => void;
};

class MobileNav {
	/** The work-buffer drawer — lives here (not MobileShell-local) so Back
	 *  can close it; it is always the topmost overlay in the chain. */
	drawerOpen = $state(false);

	/** Last entry we know to be current. Deliberately non-reactive
	 *  bookkeeping — nothing renders from it. */
	private current: MobileNavEntry | null = null;

	/** True while WE are writing history state. Kit can flush effects
	 *  synchronously inside push/replaceState, so the history watcher may
	 *  observe an intermediate state (e.g. the guard, mid-persistBaseline)
	 *  and misread it as a user traversal — this flag makes our own writes
	 *  invisible to syncFromHistory. */
	private writing = false;

	private closers = new Map<string, { priority: number } & BackCloser>();

	/** Buffer-level place providers, keyed by buffer id. Registered by the
	 *  buffer component while mounted (via $effect, teardown unregisters). */
	private viewProviders = new Map<string, ViewProvider>();

	/** A popped entry's view that couldn't apply because its buffer's
	 *  component wasn't mounted yet (cross-buffer Back remounts the
	 *  renderer); delivered when that provider registers. */
	private pendingView: { bufId: string; view: ViewPlace } | null = null;

	/** Idempotent by id — safe to call at +page script top level (the single
	 *  always-mounted route); HMR just overwrites. */
	registerCloser(id: string, priority: number, closer: BackCloser) {
		this.closers.set(id, { priority, ...closer });
	}

	registerViewProvider(bufId: string, provider: ViewProvider) {
		this.viewProviders.set(bufId, provider);
		if (this.pendingView && this.pendingView.bufId === bufId) {
			const v = this.pendingView.view;
			this.pendingView = null;
			provider.apply(v);
		}
	}

	unregisterViewProvider(bufId: string) {
		this.viewProviders.delete(bufId);
	}

	private captureView(workBuf: string | null): ViewPlace | null {
		if (!workBuf) return null;
		return this.viewProviders.get(workBuf)?.capture() ?? null;
	}

	/** A teleport-class view transition inside the focused work buffer
	 *  (view-mode switch, TOC jump, outline drill) — records the NEW place
	 *  as a history entry so Back returns to the previous one. Call AFTER
	 *  mutating the view state. No-op on desktop (no baseline) and for
	 *  non-focused buffers. */
	pushViewChange(bufId: string) {
		if (!this.current || this.current.workBuf !== bufId) return;
		const view = this.captureView(bufId);
		if (viewEq(view, this.current.view)) return;
		this.current = { ...this.current, seq: this.current.seq + 1, view };
		this.persist('push');
	}

	/** A traversal-class move (page turn) — the place is remembered on the
	 *  CURRENT entry instead of stacking; Back exits the document rather
	 *  than unwinding every page turn. Call AFTER mutating the view state. */
	replaceViewChange(bufId: string) {
		if (!this.current || this.current.workBuf !== bufId) return;
		const view = this.captureView(bufId);
		if (viewEq(view, this.current.view)) return;
		this.current = { ...this.current, view };
		this.persist('replace');
	}

	/** Write `current` to the history entry. On the very first mount tick
	 *  kit's router isn't initialized yet and push/replaceState throw —
	 *  retry once on the next macrotask, which lands after kit's start().
	 *  The retry re-reads `current`, so it always persists the latest nav
	 *  state, never a stale capture. */
	private persist(kind: 'push' | 'replace', attempt = 0) {
		const entry = this.current;
		if (!entry) return;
		this.writing = true;
		try {
			(kind === 'push' ? pushState : replaceState)('', { mnav: entry });
		} catch {
			if (attempt === 0) setTimeout(() => this.persist(kind, 1), 0);
		} finally {
			this.writing = false;
		}
	}

	/** Baseline writes TWO entries: a guard (seq -1) replacing the current
	 *  entry, then the live entry pushed above it. Back at the app's root
	 *  therefore always fires popstate onto the guard — where an open
	 *  overlay can consume the press — instead of silently no-opping
	 *  (bottom of stack) or exiting with an overlay still open. */
	private persistBaseline(guard: MobileNavEntry, attempt = 0) {
		this.writing = true;
		try {
			replaceState('', { mnav: guard });
			pushState('', { mnav: this.current! });
		} catch {
			if (attempt === 0) setTimeout(() => this.persistBaseline(guard, 1), 0);
		} finally {
			this.writing = false;
		}
	}

	/** Close the topmost open overlay: drawer first, then registered closers
	 *  by priority (highest = topmost). True if something was closed. */
	closeTopOverlay(): boolean {
		if (this.drawerOpen) {
			this.drawerOpen = false;
			return true;
		}
		const open = [...this.closers.values()]
			.filter((c) => c.isOpen())
			.sort((a, b) => b.priority - a.priority);
		if (open.length === 0) return false;
		open[0].close();
		return true;
	}

	/** Central-watcher entry point: called (untracked) whenever the active
	 *  class or work buffer changes, from ANY source. Synchronous. */
	syncFromApp(cls: ClassName, workBuf: string | null) {
		if (this.current === null) {
			// Baseline. Adopt an existing seq (reload / shell re-entry) so
			// monotonicity holds across the session. The write is ALWAYS
			// deferred a macrotask: on the mount tick kit's router accepts
			// replaceState but throws on pushState, which would strand the
			// guard as the visible entry — an async effect flush then reads
			// it as a user back-at-root and exits the app. One hop later
			// both writes land atomically. (A user can't navigate within
			// the ~0ms window, so ordering is safe.)
			const seq = Math.max(0, page.state.mnav?.seq ?? 0);
			this.current = { sid: SID, seq, cls, workBuf, view: this.captureView(workBuf) };
			const guard: MobileNavEntry = { sid: SID, seq: -1, cls, workBuf };
			setTimeout(() => this.persistBaseline(guard), 0);
			return;
		}
		if (cls === this.current.cls && workBuf === this.current.workBuf) return;
		this.current = {
			sid: SID,
			seq: this.current.seq + 1,
			cls,
			workBuf,
			view: this.captureView(workBuf)
		};
		this.persist('push');
	}

	/** History-watcher entry point: called (untracked) when page.state
	 *  changes — i.e. on back/forward traversal or our own push echo. */
	syncFromHistory(entry: MobileNavEntry | undefined, store: BufferStore) {
		if (this.writing) return; // our own write flushing synchronously
		if (!this.current) return; // pre-baseline transient
		// No mnav on the entry: this is the watcher's initial mount run
		// (before the baseline persists), NOT a traversal — never react.
		// Real root-backs land on the guard entry below, which carries
		// state; that is the only exit path.
		if (!entry) return;

		// Guard (seq -1) or foreign territory (an entry written by a
		// PREVIOUS load of this app in the same tab): the user pressed Back
		// at the app's root. An open overlay consumes the press (close +
		// restore the live entry above the guard); otherwise it's a
		// deliberate exit — keep going back out of the app. In an Android
		// WebView the next back() leaves clean history and the host exits
		// the app; in a bottom-of-stack tab it no-ops harmlessly.
		if (entry.sid !== this.current.sid || entry.seq < 0) {
			if (this.closeTopOverlay()) {
				this.persist('push');
			} else {
				window.history.back();
			}
			return;
		}

		if (entry.seq === this.current.seq) return; // echo of our own push

		// Genuine traversal. An open overlay consumes it: close, then re-add
		// the entry Back just popped (same seq → the echo is ignored).
		if (this.closeTopOverlay()) {
			this.persist('push');
			return;
		}

		// Apply the popped entry. Set current first so the central watcher's
		// equality guard suppresses a re-push from these store writes.
		this.current = entry;
		const pos = store.findSlotForClass(entry.cls);
		if (pos) store.focusSlot(pos);
		const wpos = store.findSlotForClass('work');
		if (entry.workBuf && wpos) {
			const ob = store.openBuffers.find((b) => b.buffer.id === entry.workBuf);
			if (ob && store.focusedLeaf(wpos)?.buffer.id !== entry.workBuf) {
				store.setLeaf(wpos, ob.buffer);
			}
		}
		// Restore the entry's buffer-level place. If the target buffer's
		// component isn't mounted yet (cross-buffer Back — setLeaf above
		// swaps the renderer on the next flush), park it for delivery when
		// its provider registers.
		if (entry.workBuf && entry.view) {
			const provider = this.viewProviders.get(entry.workBuf);
			if (provider) provider.apply(entry.view);
			else this.pendingView = { bufId: entry.workBuf, view: entry.view };
		}
		// Self-heal against partially applicable entries (killed buffer,
		// missing slot): re-read what actually resulted.
		this.current = {
			sid: SID,
			seq: entry.seq,
			cls: store.focusedSlotClass() ?? entry.cls,
			workBuf: wpos ? (store.focusedLeaf(wpos)?.buffer.id ?? null) : null,
			view: entry.view ?? null
		};
	}

	/** MobileShell teardown (shell flip to desktop). Closers persist —
	 *  registration is idempotent and +page registers once. */
	reset() {
		this.current = null;
		this.drawerOpen = false;
		this.pendingView = null;
	}
}

export const mobileNav = new MobileNav();
