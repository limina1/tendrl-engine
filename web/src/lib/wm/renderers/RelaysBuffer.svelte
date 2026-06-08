<script lang="ts">
	import * as api from '$lib/api';
	import { getAppState } from '$lib/state.svelte';
	import { getRelayInfo, normalizeRelayUrl, type Nip11Status, type Nip11Doc } from '$lib/relay/nip11';
	import { relayFocus, consumeRelayFocus } from '$lib/relay/focus.svelte';
	import ProfileName from '$lib/components/ProfileName.svelte';
	import type { Buffer } from '../types';

	let { buffer: _buffer }: { buffer: Buffer } = $props();

	const app = getAppState();

	// Per docs/relay-classes-and-info-port.md, a relay row carries the
	// role-agnostic shell (URL + runtime metadata + NIP-11 derived
	// flags) while role membership (read/write) lives in role-specific
	// list events. Auth here is a placeholder for the eventual
	// blocked/auth-required taxonomy; toggles don't persist yet.
	//
	// Phase 5: the main row no longer carries `search` / `indexer` —
	// those moved into the dedicated Discovery section below, where
	// each URL appears in a default-or-fallback tier explicitly.
	type RelayRow = {
		url: string;
		read: boolean;
		write: boolean;
		auth: boolean;
		broadcast: boolean;
	};

	// Discovery section rows — one entry per (URL, tier) in a class.
	// Per the composition matrix, a URL is EITHER default OR fallback
	// within a class, never both.
	type DiscoveryRow = { url: string; tier: 'default' | 'fallback' };

	let rows = $state<RelayRow[]>([]);
	let searchRows = $state<DiscoveryRow[]>([]);
	let indexerRows = $state<DiscoveryRow[]>([]);
	let searchExclusive = $state(false);
	let indexerExclusive = $state(false);
	let initialRelays = $state<string[]>([]);
	let namedSets = $state<api.NamedRelaySet[]>([]);
	let expandedSet = $state<string | null>(null);
	let publishingSetTag = $state<string | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let expanded = $state(new Set<string>());
	// Map<normalizedUrl, Nip11Status> — refreshed reactively as fetches
	// resolve. Fresh object each update so $derived sees a change.
	let infoMap = $state<Record<string, Nip11Status>>({});
	// Per-row DOM refs so we can scroll a focused row into view when
	// the EventViewModal hands us a URL via the relayFocus signal.
	let rowEls: Record<string, HTMLDivElement | undefined> = {};
	// Pulled suggestions from the user's published relay-list events
	// (kinds 10002 / 10007 / 10086 / 10088). Surfaced as suggestions
	// only — never auto-applied. The user picks per relay which set to
	// import into. Amethyst publishes 10007/10086/10088 as NIP-44-
	// encrypted content; we only parse public `r` tags here, and flag
	// when an event was found but looked encrypted (decryption is a
	// separate task).
	type PulledRelay = {
		url: string;
		// kind 10002 markers
		read?: boolean;
		write?: boolean;
		// new classes
		search?: boolean;
		indexer?: boolean;
		broadcast?: boolean;
		source_kind: 10002 | 10007 | 10086 | 10088;
	};
	/** A NIP-51 kind 30002 named relay set pulled from the user's
	 *  profile. Each set is addressable, identified by `d_tag`. The
	 *  user can import the whole set into their local named_sets via
	 *  the "+ import as set" button. */
	type PulledNamedSet = {
		d_tag: string;
		title: string;
		urls: string[];
		created_at: number;
		event_id: string;
	};
	let pulled = $state<PulledRelay[] | null>(null);
	let pulledNamedSets = $state<PulledNamedSet[]>([]);
	let pulling = $state(false);
	let pullError = $state<string | null>(null);
	let pullCreatedAt = $state<number | null>(null);
	let pullEncryptedKinds = $state<number[]>([]);
	// Per-kind result tracking — the user wants to know which kinds
	// returned events vs. which came up empty when "pulled in indexer
	// and search relays" doesn't show what they expected.
	type PullKindResult = 'parsed' | 'encrypted' | 'not_found';
	type PullKind = 10002 | 10007 | 10086 | 10088 | 30002;
	let pullKindResults = $state<Record<PullKind, PullKindResult> | null>(null);
	let pullFetchedCount = $state(0);
	// Why the encrypted notice was triggered — distinguishes "no extension
	// reachable" from "extension refused / wrong identity / errored".
	let pullDecryptReason = $state<'no-signer' | 'failed' | null>(null);
	// Per-kind decrypt error message so a user with a partial decrypt
	// (e.g. allowed kind 10088 but denied 10086) can see which failed
	// and why. Cleared with the pulled state.
	let pullDecryptErrors = $state<Record<number, string>>({});

	async function load(force = false) {
		loading = true;
		try {
			const cfg = await api.getRelayConfig();
			initialRelays = cfg.initial_relays ?? [];
			namedSets = cfg.named_sets ?? [];
			const map = new Map<string, RelayRow>();
			const ensure = (url: string): RelayRow => {
				let r = map.get(url);
				if (!r) {
					r = {
						url,
						read: false,
						write: false,
						auth: false,
						broadcast: false
					};
					map.set(url, r);
				}
				return r;
			};
			for (const url of cfg.general?.urls ?? []) {
				const r = ensure(url);
				r.read = true;
				r.write = true;
			}
			for (const url of cfg.fetch?.urls ?? []) ensure(url).read = true;
			for (const url of cfg.publish?.urls ?? []) ensure(url).write = true;
			for (const url of cfg.broadcast?.urls ?? []) ensure(url).broadcast = true;
			rows = [...map.values()].sort((a, b) => a.url.localeCompare(b.url));

			// Phase 5: Discovery section has its own per-tier rows so
			// the user can see explicitly which tier each URL belongs
			// to (and switch between them with the radio toggles).
			searchRows = [
				...(cfg.search?.default ?? []).map((url): DiscoveryRow => ({ url, tier: 'default' })),
				...(cfg.search?.fallback ?? []).map((url): DiscoveryRow => ({ url, tier: 'fallback' }))
			].sort((a, b) => a.url.localeCompare(b.url));
			indexerRows = [
				...(cfg.indexer?.default ?? []).map((url): DiscoveryRow => ({ url, tier: 'default' })),
				...(cfg.indexer?.fallback ?? []).map((url): DiscoveryRow => ({ url, tier: 'fallback' }))
			].sort((a, b) => a.url.localeCompare(b.url));
			searchExclusive = cfg.exclusive?.search ?? false;
			indexerExclusive = cfg.exclusive?.indexer ?? false;
			// Kick off NIP-11 fetches up-front so the badges fill in
			// without the user expanding each row.
			for (const r of rows) primeInfo(r.url, force);
		} catch (e) {
			error = String(e);
		} finally {
			loading = false;
		}
	}

	function primeInfo(url: string, force = false) {
		const key = normalizeRelayUrl(url);
		if (force) infoMap = { ...infoMap, [key]: { state: 'loading' } };
		const status = getRelayInfo(
			url,
			(s) => {
				infoMap = { ...infoMap, [key]: s };
			},
			{ force }
		);
		if (!force) infoMap = { ...infoMap, [key]: status };
	}

	$effect(() => {
		load();
	});

	// Consume the one-shot focus signal once rows have populated: expand
	// the matching row and scroll it into view. Matched by normalized URL
	// so trailing-slash / case / port differences don't miss.
	$effect(() => {
		const focus = relayFocus.url;
		if (!focus || rows.length === 0) return;
		const target = normalizeRelayUrl(focus);
		const row = rows.find((r) => normalizeRelayUrl(r.url) === target);
		if (!row) return;
		consumeRelayFocus();
		const next = new Set(expanded);
		next.add(row.url);
		expanded = next;
		primeInfo(row.url);
		// Wait a frame so the {#if expanded} detail block is in the DOM
		// before scrolling — gives a smoother "lands at the right place".
		queueMicrotask(() => {
			rowEls[row.url]?.scrollIntoView({ behavior: 'smooth', block: 'center' });
		});
	});

	type ToggleField = 'read' | 'write' | 'auth' | 'broadcast';
	async function toggle(url: string, field: ToggleField) {
		const row = rows.find((r) => r.url === url);
		if (!row) return;
		const next = { ...row, [field]: !row[field] };
		rows = rows.map((r) => (r.url === url ? next : r)); // optimistic

		// `auth` has no config home yet — keep it cosmetic.
		if (field === 'auth') return;

		try {
			if (field === 'broadcast') {
				await (next.broadcast ? api.addRelay('broadcast', url) : api.removeRelay('broadcast', url));
			} else {
				// Reconcile the row's read/write into explicit fetch + publish set
				// membership, and drop it from the legacy `general` set (which means
				// read+write) so a toggle-off actually takes effect after restart.
				await api.removeRelay('general', url);
				await (next.read ? api.addRelay('fetch', url) : api.removeRelay('fetch', url));
				await (next.write ? api.addRelay('publish', url) : api.removeRelay('publish', url));
			}
			app.pushToast('Relay config saved', 'success', 2000);
		} catch (e) {
			rows = rows.map((r) => (r.url === url ? row : r)); // revert on failure
			app.pushToast(
				`Couldn't save relay config: ${e instanceof Error ? e.message : String(e)}`,
				'error',
				5000
			);
		}
	}

	// ------------------------------------------------------------------
	// Discovery section — Search + Indexer subsections
	// ------------------------------------------------------------------
	type DiscoveryClass = 'search' | 'indexer';

	function rowsFor(klass: DiscoveryClass): DiscoveryRow[] {
		return klass === 'search' ? searchRows : indexerRows;
	}
	function setRowsFor(klass: DiscoveryClass, next: DiscoveryRow[]) {
		if (klass === 'search') searchRows = next;
		else indexerRows = next;
	}

	/** Switch a discovery row's tier (default ⇄ fallback). Adding to
	 *  the target tier auto-strips from the sibling on the engine via
	 *  the mutex enforced in relay_store::RelayStore::add. */
	async function setTier(klass: DiscoveryClass, url: string, tier: 'default' | 'fallback') {
		const before = rowsFor(klass);
		const next = before.map((r) => (r.url === url ? { ...r, tier } : r));
		setRowsFor(klass, next);
		try {
			await api.addRelay(`${klass}.${tier}`, url);
			app.pushToast(`${shorten(url)} → ${klass}.${tier}`, 'success', 1800);
		} catch (e) {
			setRowsFor(klass, before);
			app.pushToast(
				`Couldn't move tier: ${e instanceof Error ? e.message : String(e)}`,
				'error',
				4000
			);
		}
	}

	/** Remove a URL from a discovery class entirely (both tiers). */
	async function removeDiscovery(klass: DiscoveryClass, url: string) {
		const before = rowsFor(klass);
		setRowsFor(
			klass,
			before.filter((r) => r.url !== url)
		);
		try {
			await api.removeRelay(`${klass}.default`, url);
			await api.removeRelay(`${klass}.fallback`, url);
			app.pushToast(`Removed ${shorten(url)} from ${klass}`, 'info', 1800);
		} catch (e) {
			setRowsFor(klass, before);
			app.pushToast(
				`Couldn't remove: ${e instanceof Error ? e.message : String(e)}`,
				'error',
				4000
			);
		}
	}

	/** Toggle the per-class `exclusive` flag. ON = read relays bypassed
	 *  for this class's lookup type. */
	async function toggleExclusive(klass: DiscoveryClass) {
		const before = klass === 'search' ? searchExclusive : indexerExclusive;
		const next = !before;
		if (klass === 'search') searchExclusive = next;
		else indexerExclusive = next;
		try {
			await api.setDiscoveryExclusive(klass, next);
			app.pushToast(
				`${klass} exclusive: ${next ? 'on' : 'off'}`,
				'success',
				1800
			);
		} catch (e) {
			if (klass === 'search') searchExclusive = before;
			else indexerExclusive = before;
			app.pushToast(
				`Couldn't toggle exclusive: ${e instanceof Error ? e.message : String(e)}`,
				'error',
				4000
			);
		}
	}

	let restoringDefaults = $state(false);
	/** Merge the engine's well-known indexer/search defaults. Useful
	 *  when relays.json predates the discovery defaults (e.g. existing
	 *  users from before Phase 3) — fresh installs already get them
	 *  via seed_from_initial. */
	async function restoreDefaults() {
		restoringDefaults = true;
		try {
			const resp = await api.restoreDiscoveryDefaults();
			app.pushToast(resp.message, 'success', 3000);
			await load();
		} catch (e) {
			app.pushToast(
				`Couldn't restore defaults: ${e instanceof Error ? e.message : String(e)}`,
				'error',
				4500
			);
		} finally {
			restoringDefaults = false;
		}
	}

	/** Prompt for a new URL and add it to a discovery class's default
	 *  tier. The user can move it to fallback afterward with the radio. */
	async function promptAddDiscovery(klass: DiscoveryClass) {
		const raw = window.prompt(
			`New ${klass} relay URL (will land in ${klass}.default — move to fallback after if desired):`,
			''
		);
		if (!raw) return;
		const url = raw.trim();
		if (!url) return;
		// Local dedup against the current rows
		if (rowsFor(klass).some((r) => normalizeRelayUrl(r.url) === normalizeRelayUrl(url))) {
			app.pushToast(`${shorten(url)} already in ${klass}`, 'info', 2500);
			return;
		}
		try {
			await api.addRelay(`${klass}.default`, url);
			await load();
			app.pushToast(`Added ${shorten(url)} to ${klass}.default`, 'success', 2500);
		} catch (e) {
			app.pushToast(
				`Couldn't add: ${e instanceof Error ? e.message : String(e)}`,
				'error',
				4000
			);
		}
	}

	// Pull the user's kind 10002 (NIP-65 read/write relays) from the
	// configured `initial_relays` and surface them as **suggestions** —
	// never auto-applied, never re-published. The user picks per-relay
	// what to import into their working sets. See
	// `project_publishing_philosophy.md`.
	async function pullFromProfile() {
		const pubkey = app.myPubkey;
		if (!pubkey) {
			pullError = 'Sign in first — no pubkey to look up.';
			return;
		}
		pulling = true;
		pullError = null;
		pullEncryptedKinds = [];
		pullKindResults = null;
		pullFetchedCount = 0;
		pullDecryptReason = null;
		pullDecryptErrors = {};
		pulledNamedSets = [];
		try {
			// 1. Phase 4.1: pull through the engine's indexer composition
			//    rather than hitting `initial_relays` directly. The
			//    engine fans out across read relays (joined with
			//    indexer.default unless `exclusive.indexer` is set) and
			//    falls through to indexer.fallback when the primary
			//    returns zero — so the kind 10002 that lives only on
			//    purplepag.es shows up automatically. Activity toast +
			//    modal display the per-phase, per-relay status.
			//
			//    Kinds pulled:
			//      10002 = read/write (NIP-65, public `r` tags)
			//      10007 = search relays   (NIP-44 encrypted, Amethyst)
			//      10086 = indexer relays  (NIP-44 encrypted, Amethyst)
			//      10088 = broadcast list  (NIP-44 encrypted, Amethyst)
			//      30002 = NIP-51 named relay sets (one per d_tag)
			const kinds = [10002, 10007, 10086, 10088, 30002] as const;
			const fetchResult = await api.pullUserData(pubkey);
			pullFetchedCount = fetchResult.fetched;

			// Try decrypt whenever a NIP-07 extension is reachable. We try
			// nip44 first (current Amethyst convention), then fall back to
			// nip04 (legacy format used by older Amethyst versions and many
			// other clients pre-nip44). Detection: NIP-04 ciphertext has
			// "?iv=" in it; NIP-44 is a single base64 blob. The extension
			// runs its own permission flow per call.
			const canDecrypt =
				typeof window !== 'undefined' &&
				!!(window.nostr?.nip44?.decrypt || window.nostr?.nip04?.decrypt);
			// Kept as `canNip44` below for diagnostic-state naming continuity.
			const canNip44 = canDecrypt;

			// 2. Read them back from the local cache.
			//    - For replaceable 100xx: newest per kind wins.
			//    - For addressable 30002: take ALL events; each is a
			//      separate named set keyed by `d` tag.
			const entries: PulledRelay[] = [];
			const namedSets: PulledNamedSet[] = [];
			const seen = new Set<string>(); // dedup by `${source_kind}:${normalizedUrl}`
			let maxCreatedAt = 0;
			const encrypted: number[] = [];
			const kindResults: Record<PullKind, PullKindResult> = {
				10002: 'not_found',
				10007: 'not_found',
				10086: 'not_found',
				10088: 'not_found',
				30002: 'not_found'
			};
			const push = (s: PulledRelay) => {
				const key = `${s.source_kind}:${normalizeRelayUrl(s.url)}`;
				if (seen.has(key)) return;
				seen.add(key);
				entries.push(s);
			};

			// --- Kind 30002 readback (addressable, many per author) ---
			{
				const result = await api.search(`by:${pubkey} k:30002`, 64, pubkey, 'local_only');
				const events = result.results ?? [];
				// One PulledNamedSet per latest event per `d` tag — older
				// versions of the same set get superseded by the newer one.
				const byDtag = new Map<string, typeof events[number]>();
				for (const ev of events) {
					const dTag = (ev.tags ?? []).find((t) => t[0] === 'd')?.[1];
					if (!dTag) continue;
					const existing = byDtag.get(dTag);
					if (!existing || (ev.created_at ?? 0) > (existing.created_at ?? 0)) {
						byDtag.set(dTag, ev);
					}
				}
				for (const ev of byDtag.values()) {
					const dTag = (ev.tags ?? []).find((t) => t[0] === 'd')?.[1] ?? '';
					const title =
						(ev.tags ?? []).find((t) => t[0] === 'title')?.[1] ?? dTag;
					// NIP-51 kind 30002 relay sets carry URLs in `relay` tags
					// (kind 10002 / NIP-65 uses `r`). Accept `r` as a tolerant
					// fallback for non-conformant publishers.
					const urls = (ev.tags ?? [])
						.filter((t) => (t[0] === 'relay' || t[0] === 'r') && typeof t[1] === 'string')
						.map((t) => t[1] as string);
					namedSets.push({
						d_tag: dTag,
						title,
						urls,
						created_at: ev.created_at ?? 0,
						event_id: ev.event_id
					});
					if ((ev.created_at ?? 0) > maxCreatedAt) maxCreatedAt = ev.created_at ?? 0;
				}
				if (namedSets.length > 0) kindResults[30002] = 'parsed';
			}

			// --- Kinds 10002/10007/10086/10088 (replaceable; newest wins) ---
			const replaceableKinds = [10002, 10007, 10086, 10088] as const;
			for (const kind of replaceableKinds) {
				const result = await api.search(`by:${pubkey} k:${kind}`, 3, pubkey, 'local_only');
				const newest = (result.results ?? []).sort(
					(a, b) => (b.created_at ?? 0) - (a.created_at ?? 0)
				)[0];
				if (!newest) continue;
				// Sanity: nip44 self-decrypt requires the event's author to
				// match the recipient pubkey we pass. If the search returned
				// an event authored by someone else, the extension will
				// always fail with the opaque "Failed to decrypt message".
				if (newest.author && newest.author !== pubkey) {
					console.warn(
						`kind ${kind}: search returned event by ${newest.author.slice(0, 16)}… but my pubkey is ${pubkey.slice(0, 16)}… — decrypt will fail`
					);
				}
				if ((newest.created_at ?? 0) > maxCreatedAt) maxCreatedAt = newest.created_at ?? 0;

				// Public r-tags (any kind may use this format).
				const rTags = (newest.tags ?? []).filter(
					(t) => t[0] === 'r' && typeof t[1] === 'string'
				);

				// Decrypted private tags (Amethyst's PrivateTagArrayEvent).
				// Only attempted for 100xx kinds with non-empty content.
				let privateRelayTags: string[][] = [];
				let decryptAttempted = false;
				let decryptFailed = false;
				if (kind !== 10002 && (newest.preview?.length ?? 0) > 0) {
					decryptAttempted = true;
					if (canDecrypt) {
						// `newest.preview` is truncated to 200 chars in the
						// engine's SearchResult (search.rs:492); decrypt
						// needs the FULL event content. Fetch by id.
						let fullContent: string;
						try {
							const fullEvent = (await api.getEvent(newest.event_id)).event as {
								content?: string;
							};
							fullContent = fullEvent?.content ?? newest.preview;
						} catch {
							fullContent = newest.preview;
						}
						// Mirror Amethyst's EncryptedInfo (nip04Dm/crypto):
						// strip the `-null` suffix some clients tack on, then
						// detect NIP-04 by `?iv=` at *position* length-28
						// (24 base64 chars of IV + 4 for "?iv="). A loose
						// `.includes('?iv=')` would false-positive on NIP-44
						// base64 that happens to contain those chars.
						const raw = fullContent;
						const ciphertext = raw.endsWith('-null') ? raw.slice(0, -5) : raw;
						const l = ciphertext.length;
						const looksNip04 =
							l >= 28 &&
							ciphertext[l - 28] === '?' &&
							ciphertext[l - 27] === 'i' &&
							ciphertext[l - 26] === 'v' &&
							ciphertext[l - 25] === '=';
						const tryPaths: Array<'nip04' | 'nip44'> = looksNip04
							? ['nip04', 'nip44']
							: ['nip44', 'nip04'];
						let plaintext: string | null = null;
						let lastErr: unknown = null;
						let usedPath: 'nip04' | 'nip44' | null = null;
						for (const path of tryPaths) {
							const fn = window.nostr?.[path]?.decrypt;
							if (!fn) {
								lastErr = lastErr ?? new Error(`extension has no ${path}.decrypt`);
								continue;
							}
							try {
								plaintext = await fn.call(window.nostr![path]!, pubkey, ciphertext);
								usedPath = path;
								break;
							} catch (err) {
								lastErr = err;
							}
						}
						if (plaintext === null) {
							decryptFailed = true;
							const msg = lastErr instanceof Error ? lastErr.message : String(lastErr);
							// Include a fingerprint of the ciphertext to help
							// diagnose mismatch / unknown format issues —
							// length, looks_nip04, head, tail.
							const head = ciphertext.slice(0, 12);
							const tail = ciphertext.slice(-12);
							const authorMismatch =
								newest.author && newest.author !== pubkey
									? ` author≠me`
									: '';
							const fingerprint = `len=${ciphertext.length} nip04=${looksNip04} tried=${tryPaths.join('→')}${authorMismatch} head="${head}…${tail}"`;
							pullDecryptErrors = {
								...pullDecryptErrors,
								[kind]: `${msg} (${fingerprint})`
							};
							console.warn(
								`decrypt failed for kind ${kind} (tried ${tryPaths.join(' → ')}):\n  error:`,
								lastErr,
								`\n  event id:`,
								newest.event_id,
								`\n  event author:`,
								newest.author,
								`\n  my pubkey:`,
								pubkey,
								`\n  ciphertext (${ciphertext.length} chars):`,
								ciphertext
							);
						} else {
							try {
								const parsed = JSON.parse(plaintext);
								if (!Array.isArray(parsed)) {
									throw new Error('decrypted JSON is not an array of tags');
								}
								privateRelayTags = (parsed as unknown[]).filter(
									(t): t is string[] =>
										Array.isArray(t) && t[0] === 'relay' && typeof t[1] === 'string'
								);
								console.debug(`decrypted kind ${kind} via ${usedPath}, ${privateRelayTags.length} relay tag(s)`);
							} catch (parseErr) {
								decryptFailed = true;
								const msg = `decrypted via ${usedPath} but not valid JSON: ${(parseErr as Error).message}`;
								pullDecryptErrors = { ...pullDecryptErrors, [kind]: msg };
								console.warn(`decrypt JSON parse failed for kind ${kind}:`, parseErr);
							}
						}
					} else {
						// No decrypt path available → can't decrypt.
						decryptFailed = true;
						pullDecryptErrors = {
							...pullDecryptErrors,
							[kind]: 'no NIP-07 extension reachable'
						};
					}
				}

				if (rTags.length === 0 && privateRelayTags.length === 0) {
					if (decryptAttempted && decryptFailed) {
						encrypted.push(kind);
						kindResults[kind] = 'encrypted';
						// Record why so the notice can be specific.
						if (!canNip44) pullDecryptReason = 'no-signer';
						else if (!pullDecryptReason) pullDecryptReason = 'failed';
					}
					continue;
				}

				kindResults[kind] = 'parsed';

				// Merge: public r-tags then decrypted private relay tags.
				for (const t of rTags) {
					const url = t[1] as string;
					const marker = (t[2] ?? '').toLowerCase();
					if (kind === 10002) {
						push({
							url,
							source_kind: 10002,
							read: marker === 'read' || marker === '',
							write: marker === 'write' || marker === ''
						});
					} else if (kind === 10007) push({ url, source_kind: 10007, search: true });
					else if (kind === 10086) push({ url, source_kind: 10086, indexer: true });
					else if (kind === 10088) push({ url, source_kind: 10088, broadcast: true });
				}
				for (const t of privateRelayTags) {
					const url = t[1];
					if (kind === 10007) push({ url, source_kind: 10007, search: true });
					else if (kind === 10086) push({ url, source_kind: 10086, indexer: true });
					else if (kind === 10088) push({ url, source_kind: 10088, broadcast: true });
				}
			}

			pullCreatedAt = maxCreatedAt > 0 ? maxCreatedAt : null;
			pullEncryptedKinds = encrypted;
			pullKindResults = kindResults;
			pulled = entries;
			pulledNamedSets = namedSets;
		} catch (e) {
			pullError = e instanceof Error ? e.message : String(e);
		} finally {
			pulling = false;
		}
	}

	/** Import a pulled NIP-51 kind 30002 named set into local
	 *  `named_sets`. Creates the set (if it doesn't already exist) then
	 *  adds each URL as a member. Idempotent at the engine level — the
	 *  named_set CRUD already silently skips existing dups. */
	let importingSetTag = $state<string | null>(null);
	// Member URLs of an already-imported set that are missing from / extra to
	// the pulled event. Empty arrays on both sides ⇒ the local copy is in sync.
	function namedSetDrift(set: PulledNamedSet): { toAdd: string[]; toRemove: string[] } {
		const stored = namedSets.find((s) => s.d_tag === set.d_tag);
		const storedKeys = new Map((stored?.urls ?? []).map((u) => [normalizeRelayUrl(u), u]));
		const pulledKeys = new Map(set.urls.map((u) => [normalizeRelayUrl(u), u]));
		const toAdd = [...pulledKeys].filter(([k]) => !storedKeys.has(k)).map(([, u]) => u);
		const toRemove = [...storedKeys].filter(([k]) => !pulledKeys.has(k)).map(([, u]) => u);
		return { toAdd, toRemove };
	}

	// Import a pulled named set, or — if it already exists locally — reconcile
	// its members to match the pulled event (add new, drop removed).
	async function importNamedSet(set: PulledNamedSet) {
		const existing = namedSets.some((s) => s.d_tag === set.d_tag);
		importingSetTag = set.d_tag;
		try {
			await api.createNamedSet(set.d_tag, set.title); // no-op if it already exists
			const { toAdd, toRemove } = existing
				? namedSetDrift(set)
				: { toAdd: set.urls, toRemove: [] as string[] };
			for (const url of toAdd) await api.addToNamedSet(set.d_tag, url);
			for (const url of toRemove) await api.removeFromNamedSet(set.d_tag, url);
			const verb = existing ? 'Updated' : 'Imported';
			app.pushToast(
				`${verb} "${set.title}" (${set.urls.length} relay${set.urls.length === 1 ? '' : 's'})`,
				'success',
				2500
			);
			await load();
		} catch (e) {
			app.pushToast(
				`Couldn't import "${set.title}": ${e instanceof Error ? e.message : String(e)}`,
				'error',
				4500
			);
		} finally {
			importingSetTag = null;
		}
	}

	function rowKeyFor(url: string): string {
		return normalizeRelayUrl(url);
	}

	function alreadyConfigured(url: string): RelayRow | undefined {
		const key = rowKeyFor(url);
		return rows.find((r) => rowKeyFor(r.url) === key);
	}

	type ImportRole = 'fetch' | 'publish' | 'both' | 'broadcast' | 'search' | 'indexer';
	async function importSuggestion(s: PulledRelay, role: ImportRole) {
		try {
			if (role === 'fetch' || role === 'both') await api.addRelay('fetch', s.url);
			if (role === 'publish' || role === 'both') await api.addRelay('publish', s.url);
			if (role === 'broadcast') await api.addRelay('broadcast', s.url);
			// Pull-from-profile suggestions default to the `.default`
			// tier — the user can move them to fallback later from the
			// Phase-5 Discovery section.
			if (role === 'search') await api.addRelay('search.default', s.url);
			if (role === 'indexer') await api.addRelay('indexer.default', s.url);
			app.pushToast(`Added ${shorten(s.url)} to ${role}`, 'success', 2500);
			await load();
		} catch (e) {
			app.pushToast(
				`Couldn't add ${shorten(s.url)}: ${e instanceof Error ? e.message : String(e)}`,
				'error',
				4000
			);
		}
	}

	function dismissPulled() {
		pulled = null;
		pullError = null;
		pullCreatedAt = null;
		pullEncryptedKinds = [];
		pullKindResults = null;
		pullFetchedCount = 0;
		pullDecryptReason = null;
		pullDecryptErrors = {};
	}

	const pulledByKind = $derived.by(() => {
		const groups: Record<10002 | 10007 | 10086 | 10088, PulledRelay[]> = {
			10002: [],
			10007: [],
			10086: [],
			10088: []
		};
		for (const s of pulled ?? []) groups[s.source_kind].push(s);
		return groups;
	});

	function classNameForKind(k: 10002 | 10007 | 10086 | 10088): string {
		return k === 10002
			? 'read/write (NIP-65)'
			: k === 10007
				? 'search (NIP-50)'
				: k === 10086
					? 'indexer'
					: 'broadcast';
	}

	// Add a new relay via the prompt — defaults to read+write so the
	// relay is fully active; user can toggle either side off after.
	async function promptAdd() {
		const raw = window.prompt('Relay URL (bare hostname OK — wss:// is added if missing):');
		if (!raw) return;
		const trimmed = raw.trim();
		if (!trimmed) return;
		// Client-side normalization for nice display; the engine
		// normalizes again on the receiving end, so this is purely UX.
		const url = normalizeRelayUrl(trimmed);
		if (rows.some((r) => normalizeRelayUrl(r.url) === url)) {
			app.pushToast(`${shorten(url)} is already configured`, 'info', 2500);
			return;
		}
		try {
			await api.addRelay('fetch', url);
			await api.addRelay('publish', url);
			app.pushToast(`Added ${shorten(url)} (read + write)`, 'success', 2500);
			await load();
		} catch (e) {
			app.pushToast(
				`Couldn't add ${shorten(url)}: ${e instanceof Error ? e.message : String(e)}`,
				'error',
				4000
			);
		}
	}

	// Publish a relay-list event. Explicit, user-initiated — never fires
	// on toggle/add/remove. Per project_publishing_philosophy: pulling
	// these is for context; publishing is deliberate. Amethyst-style
	// kinds (10007/10086/10088) store the relay list as NIP-44 encrypted
	// private tags in `content`; kind 10002 uses public `r` tags only.
	type PublishableKind = 10002 | 10007 | 10086 | 10088;
	let publishingKind = $state<PublishableKind | null>(null);

	function relaysForPublish(kind: PublishableKind): string[] {
		if (kind === 10002) {
			return rows.filter((r) => r.read || r.write).map((r) => r.url);
		}
		if (kind === 10088) {
			return rows.filter((r) => r.broadcast).map((r) => r.url);
		}
		// Phase 5: kind 10007 (search) and 10086 (indexer) are now the
		// union of both tiers from the Discovery section.
		if (kind === 10007) return searchRows.map((r) => r.url);
		if (kind === 10086) return indexerRows.map((r) => r.url);
		return [];
	}

	function classLabelForPublish(kind: PublishableKind): string {
		return kind === 10002
			? 'read/write'
			: kind === 10007
				? 'search'
				: kind === 10086
					? 'indexer'
					: 'broadcast';
	}

	async function publishRelayListByKind(kind: PublishableKind) {
		if (!app.myPubkey) {
			app.pushToast('Sign in first — no identity to sign the event.', 'error', 4000);
			return;
		}
		const urls = relaysForPublish(kind);
		if (urls.length === 0) {
			app.pushToast(`No ${classLabelForPublish(kind)} relays to publish.`, 'info', 3000);
			return;
		}

		publishingKind = kind;
		try {
			let tags: string[][] = [];
			let content = '';
			if (kind === 10002) {
				// NIP-65: public r-tags with read/write markers.
				for (const r of rows) {
					if (r.read && r.write) tags.push(['r', r.url]);
					else if (r.read) tags.push(['r', r.url, 'read']);
					else if (r.write) tags.push(['r', r.url, 'write']);
				}
			} else {
				// Amethyst PrivateTagArrayEvent convention: tags array is
				// empty, the relay list lives in NIP-44-encrypted content
				// as a JSON tag-array of ["relay", url] entries.
				const privateTags = urls.map((url) => ['relay', url]);
				const plaintext = JSON.stringify(privateTags);
				if (!window.nostr?.nip44?.encrypt) {
					throw new Error(
						'NIP-07 extension does not expose nip44.encrypt — cannot publish encrypted private list (kind ' +
							kind +
							').'
					);
				}
				content = await window.nostr.nip44.encrypt(app.myPubkey, plaintext);
			}
			const { signed_event } = await api.signTemplate({
				template: {
					kind,
					created_at: Math.floor(Date.now() / 1000),
					tags,
					content,
					pubkey: app.myPubkey
				}
			});
			const resp = await api.broadcastEvent({ event: signed_event });
			app.pushToast(
				`Published kind ${kind} (${urls.length} ${classLabelForPublish(kind)} relay${urls.length === 1 ? '' : 's'}) to ${resp.successful}/${resp.total} publish relays`,
				resp.successful > 0 ? 'success' : 'error',
				4000
			);
		} catch (e) {
			app.pushToast(
				`Publish kind ${kind} failed: ${e instanceof Error ? e.message : String(e)}`,
				'error',
				5000
			);
		} finally {
			publishingKind = null;
		}
	}

	// ---- Named sets (NIP-51 kind 30002) ---------------------------------
	async function promptNewSet() {
		const title = window.prompt('Name for the new relay set (e.g. "research", "friends"):');
		if (!title || !title.trim()) return;
		const d_tag = crypto.randomUUID();
		try {
			await api.createNamedSet(d_tag, title.trim());
			app.pushToast(`Created set "${title.trim()}"`, 'success', 2000);
			await load();
			expandedSet = d_tag;
		} catch (e) {
			app.pushToast(
				`Couldn't create set: ${e instanceof Error ? e.message : String(e)}`,
				'error',
				4000
			);
		}
	}

	async function promptAddToSet(d_tag: string) {
		const raw = window.prompt('Relay URL to add to this set:');
		if (!raw || !raw.trim()) return;
		const url = normalizeRelayUrl(raw.trim());
		try {
			await api.addToNamedSet(d_tag, url);
			app.pushToast(`Added ${shorten(url)}`, 'success', 2000);
			await load();
		} catch (e) {
			app.pushToast(
				`Couldn't add ${shorten(url)}: ${e instanceof Error ? e.message : String(e)}`,
				'error',
				4000
			);
		}
	}

	async function removeMemberFromSet(d_tag: string, url: string) {
		try {
			await api.removeFromNamedSet(d_tag, url);
			await load();
		} catch (e) {
			app.pushToast(
				`Couldn't remove ${shorten(url)}: ${e instanceof Error ? e.message : String(e)}`,
				'error',
				4000
			);
		}
	}

	async function deleteSet(d_tag: string, title: string) {
		if (!window.confirm(`Delete the "${title}" relay set?\n\n(This only removes it locally. To take it down from Nostr you'd publish a delete event — not done here.)`)) {
			return;
		}
		try {
			await api.deleteNamedSet(d_tag);
			app.pushToast(`Deleted set "${title}"`, 'info', 2000);
			if (expandedSet === d_tag) expandedSet = null;
			await load();
		} catch (e) {
			app.pushToast(
				`Couldn't delete set: ${e instanceof Error ? e.message : String(e)}`,
				'error',
				4000
			);
		}
	}

	async function renameSet(d_tag: string, currentTitle: string) {
		const next = window.prompt('Rename set:', currentTitle);
		if (!next || !next.trim() || next.trim() === currentTitle) return;
		try {
			await api.renameNamedSet(d_tag, next.trim());
			await load();
		} catch (e) {
			app.pushToast(
				`Couldn't rename: ${e instanceof Error ? e.message : String(e)}`,
				'error',
				4000
			);
		}
	}

	async function publishNamedSet(set: api.NamedRelaySet) {
		if (!app.myPubkey) {
			app.pushToast('Sign in first.', 'error', 4000);
			return;
		}
		if (set.urls.length === 0) {
			app.pushToast(`"${set.title}" has no relays.`, 'info', 3000);
			return;
		}
		publishingSetTag = set.d_tag;
		try {
			// NIP-51 kind 30002: `d` for the addressable id, `title` for
			// the human label, `r` tags for each public relay entry. We
			// publish the public form; encrypted private members would
			// go in NIP-44-encrypted content (deferred).
			const tags: string[][] = [
				['d', set.d_tag],
				['title', set.title],
				['alt', `Relay set: ${set.title}`],
				...set.urls.map((url) => ['r', url])
			];
			const { signed_event } = await api.signTemplate({
				template: {
					kind: 30002,
					created_at: Math.floor(Date.now() / 1000),
					tags,
					content: '',
					pubkey: app.myPubkey
				}
			});
			const resp = await api.broadcastEvent({ event: signed_event });
			app.pushToast(
				`Published "${set.title}" (kind 30002, ${set.urls.length} relays) to ${resp.successful}/${resp.total} publish relays`,
				resp.successful > 0 ? 'success' : 'error',
				4000
			);
		} catch (e) {
			app.pushToast(
				`Publish failed: ${e instanceof Error ? e.message : String(e)}`,
				'error',
				5000
			);
		} finally {
			publishingSetTag = null;
		}
	}

	let snapshotting = $state(false);
	async function snapshotToConfig() {
		snapshotting = true;
		try {
			const resp = await api.snapshotConfig();
			app.pushToast(resp.message, 'success', 3500);
			// Re-load so initialRelays picks up the just-snapshotted
			// value and the dirty-flag clears.
			await load();
		} catch (e) {
			app.pushToast(
				`Snapshot failed: ${e instanceof Error ? e.message : String(e)}`,
				'error',
				5000
			);
		} finally {
			snapshotting = false;
		}
	}

	/** True when the current relay union the snapshot would write
	 *  (general ∪ fetch ∪ publish — i.e. anything toggled read OR
	 *  write) differs from what's currently in config.toml's
	 *  `initial_relays`. Broadcast-only / search-only / indexer-only
	 *  URLs DON'T count — those aren't in the snapshot. */
	const settingsDirty = $derived.by(() => {
		// Only URLs in fetch (read), publish (write), or general
		// (both) end up in the snapshot. A row toggled broadcast-only
		// would show up in `rows` but not in `initial_relays`, so
		// matching against the wider rows set kept the button
		// permanently dirty.
		const live = rows
			.filter((r) => r.read || r.write)
			.map((r) => normalizeRelayUrl(r.url))
			.sort();
		const saved = initialRelays.map((u) => normalizeRelayUrl(u)).sort();
		if (live.length !== saved.length) return true;
		for (let i = 0; i < live.length; i++) {
			if (live[i] !== saved[i]) return true;
		}
		return false;
	});

	function toggleExpanded(url: string) {
		const next = new Set(expanded);
		if (next.has(url)) next.delete(url);
		else next.add(url);
		expanded = next;
	}

	function shorten(url: string): string {
		return url.replace(/^wss?:\/\//, '').replace(/\/$/, '');
	}

	function statusFor(url: string): Nip11Status {
		return infoMap[normalizeRelayUrl(url)] ?? { state: 'pending' };
	}

	function docFor(url: string): Nip11Doc | null {
		const s = statusFor(url);
		return s.state === 'loaded' ? s.doc : null;
	}
</script>

<div class="relays-view">
	<div class="relays-header">
		<div class="relays-header-title">
			<span>Relay configuration</span>
			<span class="relays-hint">read/write apply live and persist · auth is cosmetic</span>
		</div>
		<div class="relays-header-actions">
			<button class="btn-refresh" onclick={() => load(true)}>Refresh</button>
			<button
				class="btn-snapshot"
				class:btn-snapshot--dirty={settingsDirty}
				onclick={snapshotToConfig}
				disabled={!settingsDirty || snapshotting || rows.length === 0}
				title={!settingsDirty
					? 'No unsaved changes — current relays already match config.toml.'
					: 'Snapshot the current relay set into config.toml `initial_relays` — portable bootstrap seed. relays.json stays the runtime source of truth.'}
			>
				{snapshotting ? 'Saving…' : settingsDirty ? 'Save settings *' : 'Save settings'}
			</button>
		</div>
	</div>

	<!-- Pull-from-profile: fetches the user's relay-list events
	     (10002/10007/10086/10088/30002) through the engine's indexer
	     composition — read relays first, falling through to
	     indexer.default → indexer.fallback. Suggestions never
	     auto-apply; the user picks per relay. -->
	<div class="pull-bar">
		{#if !pulled && !pulling && !pullError}
			<button
				class="btn-pull"
				onclick={pullFromProfile}
				disabled={!app.myPubkey}
				title={!app.myPubkey
					? 'Sign in first'
					: 'Fetch your published relay lists (kinds 10002 / 10007 / 10086 / 10088 / 30002) through the indexer composition.'}
			>
				Pull from your profile
			</button>
			{#if !app.myPubkey}
				<span class="pull-hint pull-hint--warn">Sign in first to fetch your relay list.</span>
			{:else if initialRelays.length === 0 && rows.length === 0}
				<span class="pull-hint pull-hint--warn">
					No relays configured. Add at least one read relay below — or seed via <code>initial_relays</code> in <code>config.toml</code> under <code>[relay]</code>.
				</span>
			{:else}
				<span class="pull-hint">Walks the indexer composition; you choose what to import. Pulls kind 10002 / 10007 / 10086 / 10088 / 30002.</span>
			{/if}
		{:else if pulling}
			<span class="pull-status">Fetching your relay list…</span>
		{:else if pullError}
			<span class="pull-status pull-status--err">{pullError}</span>
			<button class="btn-pull btn-pull--small" onclick={dismissPulled}>dismiss</button>
			<button class="btn-pull btn-pull--small" onclick={pullFromProfile}>retry</button>
		{:else if pulled}
			<span class="pull-status">
				Pulled {pullFetchedCount} event{pullFetchedCount === 1 ? '' : 's'} from {initialRelays.length} relay{initialRelays.length === 1 ? '' : 's'}{#if pullCreatedAt}
					· newest {new Date(pullCreatedAt * 1000).toLocaleDateString()}
				{/if}
			</span>
			<button class="btn-pull btn-pull--small" onclick={dismissPulled}>dismiss</button>
		{/if}
	</div>

	{#if pullKindResults}
		<div class="pull-diagnostics">
			{#each [10002, 10007, 10086, 10088] as kind (kind)}
				{@const status = pullKindResults[kind as 10002 | 10007 | 10086 | 10088]}
				{@const group = pulledByKind[kind as 10002 | 10007 | 10086 | 10088]}
				<span class="pull-diag" class:pull-diag--ok={status === 'parsed'} class:pull-diag--enc={status === 'encrypted'} class:pull-diag--missing={status === 'not_found'}>
					kind {kind} ({classNameForKind(kind as 10002 | 10007 | 10086 | 10088)}):
					{#if status === 'parsed'}{group.length} relay{group.length === 1 ? '' : 's'}
					{:else if status === 'encrypted'}encrypted (NIP-44)
					{:else}not found
					{/if}
				</span>
			{/each}
			<span
				class="pull-diag"
				class:pull-diag--ok={pullKindResults[30002] === 'parsed'}
				class:pull-diag--missing={pullKindResults[30002] === 'not_found'}
			>
				kind 30002 (named sets):
				{#if pullKindResults[30002] === 'parsed'}{pulledNamedSets.length} set{pulledNamedSets.length === 1 ? '' : 's'}
				{:else}not found
				{/if}
			</span>
		</div>
	{/if}

	{#if (pulled && pulled.length > 0) || pullEncryptedKinds.length > 0 || pulledNamedSets.length > 0}
		<div class="pulled-list">
			<div class="pulled-label">From your profile · suggestions</div>

			{#if pulledNamedSets.length > 0}
				<div class="pulled-kind-label">kind 30002 · named sets</div>
				{#each pulledNamedSets as set (set.d_tag)}
					{@const alreadyImported = namedSets.some((s) => s.d_tag === set.d_tag)}
					{@const drift = alreadyImported ? namedSetDrift(set) : { toAdd: [], toRemove: [] }}
					{@const driftCount = drift.toAdd.length + drift.toRemove.length}
					<div class="pulled-row pulled-row--set">
						<span class="pulled-set-title" title={`d=${set.d_tag}`}>{set.title}</span>
						<span class="pulled-set-meta">{set.urls.length} relay{set.urls.length === 1 ? '' : 's'}</span>
						<div class="pulled-actions">
							{#if alreadyImported && driftCount > 0}
								<button
									class="pull-add pull-add--strong"
									onclick={() => importNamedSet(set)}
									disabled={importingSetTag !== null}
									title={`Sync local set to the pulled event: +${drift.toAdd.length} / −${drift.toRemove.length} relay${driftCount === 1 ? '' : 's'}.`}
								>
									{importingSetTag === set.d_tag ? 'Updating…' : `↻ update (${set.urls.length})`}
								</button>
							{:else if alreadyImported}
								<span class="pulled-state">already imported</span>
							{:else}
								<button
									class="pull-add pull-add--strong"
									onclick={() => importNamedSet(set)}
									disabled={importingSetTag !== null}
									title={`Create a local named set with d="${set.d_tag.slice(0, 8)}…" and add ${set.urls.length} member relay${set.urls.length === 1 ? '' : 's'}.`}
								>
									{importingSetTag === set.d_tag ? 'Importing…' : '+ import as set'}
								</button>
							{/if}
						</div>
					</div>
				{/each}
			{/if}

			{#each [10002, 10007, 10086, 10088] as kind (kind)}
				{@const group = pulledByKind[kind as 10002 | 10007 | 10086 | 10088]}
				{#if group.length > 0}
					<div class="pulled-kind-label">kind {kind} · {classNameForKind(kind as 10002 | 10007 | 10086 | 10088)}</div>
					{#each group as s (`${s.source_kind}:${s.url}`)}
						{@const existing = alreadyConfigured(s.url)}
						<div class="pulled-row">
							<span class="pulled-url" title={s.url}>{shorten(s.url)}</span>
							<span class="pulled-marker">
								{#if kind === 10002}
									{#if s.read && s.write}read+write
									{:else if s.read}read
									{:else if s.write}write
									{/if}
								{:else if kind === 10007}search
								{:else if kind === 10086}indexer
								{:else if kind === 10088}broadcast
								{/if}
							</span>
							<div class="pulled-actions">
								{#if kind === 10002}
									{#if !existing?.read && s.read}
										<button class="pull-add" onclick={() => importSuggestion(s, 'fetch')}>+ fetch</button>
									{/if}
									{#if !existing?.write && s.write}
										<button class="pull-add" onclick={() => importSuggestion(s, 'publish')}>+ publish</button>
									{/if}
									{#if !existing?.read && !existing?.write && s.read && s.write}
										<button class="pull-add pull-add--strong" onclick={() => importSuggestion(s, 'both')}>+ both</button>
									{/if}
								{:else if kind === 10007}
									{@const alreadyInSearch = searchRows.some((r) => r.url === s.url)}
									{#if !alreadyInSearch}
										<button class="pull-add" onclick={() => importSuggestion(s, 'search')}>+ search</button>
									{/if}
								{:else if kind === 10086}
									{@const alreadyInIndexer = indexerRows.some((r) => r.url === s.url)}
									{#if !alreadyInIndexer}
										<button class="pull-add" onclick={() => importSuggestion(s, 'indexer')}>+ indexer</button>
									{/if}
								{:else if kind === 10088}
									{#if !existing?.broadcast}
										<button class="pull-add" onclick={() => importSuggestion(s, 'broadcast')}>+ broadcast</button>
									{/if}
								{/if}
								{#if (kind === 10002 && existing && (existing.read || existing.write)) || (kind === 10007 && searchRows.some((r) => r.url === s.url)) || (kind === 10086 && indexerRows.some((r) => r.url === s.url)) || (kind === 10088 && existing?.broadcast)}
									<span class="pulled-state">already in {kind === 10002 ? 'read/write' : classNameForKind(kind as 10002 | 10007 | 10086 | 10088)}</span>
								{/if}
							</div>
						</div>
					{/each}
				{/if}
			{/each}

			{#if pullEncryptedKinds.length > 0}
				<div class="pulled-kind-label">encrypted (NIP-44) — couldn't decrypt</div>
				<p class="pulled-encrypted">
					Found encrypted private list event{pullEncryptedKinds.length === 1 ? '' : 's'} for kind
					{pullEncryptedKinds.join(', ')}.
					{#if pullDecryptReason === 'no-signer'}
						No NIP-07 extension reachable — install one (nos2x, Alby, …) that exposes <code>nip44.decrypt</code>, then retry. Engine-side decrypt with ncryptsec is queued as T32.
					{:else}
						Extension is reachable but didn't return plaintext.
						<button class="pull-add" onclick={pullFromProfile}>Retry decrypt</button>
					{/if}
				</p>
				{#if Object.keys(pullDecryptErrors).length > 0}
					<dl class="decrypt-errors">
						{#each pullEncryptedKinds as kind (kind)}
							{#if pullDecryptErrors[kind]}
								<dt>kind {kind}</dt>
								<dd>{pullDecryptErrors[kind]}</dd>
							{/if}
						{/each}
					</dl>
				{/if}
			{/if}
		</div>
	{/if}

	{#if loading}
		<p class="empty">Loading…</p>
	{:else if error}
		<p class="empty error">{error}</p>
	{:else if rows.length === 0}
		<p class="empty">No relays configured</p>
	{:else}
		{@const count10002 = relaysForPublish(10002).length}
		{@const count10088 = relaysForPublish(10088).length}
		<!-- Read / Write / Broadcast section. Per-class publish buttons
		     live in this header (their content is built from this
		     section's toggles), not in the global footer. -->
		<div class="rw-section">
			<div class="rw-section-head">
				<span class="rw-section-title">Read / Write / Broadcast</span>
				<div class="rw-section-actions">
					<button
						class="btn-add" onclick={promptAdd}
						title="Add a new relay (defaults to read + write — toggle either off after)"
					>+ Add relay</button>
					<button
						class="btn-publish-list btn-publish-list--read"
						onclick={() => publishRelayListByKind(10002)}
						disabled={publishingKind !== null || count10002 === 0 || !app.myPubkey}
						title={!app.myPubkey
							? 'Sign in first to publish.'
							: count10002 === 0
								? 'No read/write relays toggled.'
								: `Sign a kind 10002 (NIP-65) with ${count10002} read/write relay${count10002 === 1 ? '' : 's'} and broadcast to your publish set.`}
					>
						{publishingKind === 10002 ? 'Publishing…' : `Publish kind 10002 (${count10002})`}
					</button>
					<button
						class="btn-publish-list btn-publish-list--broadcast"
						onclick={() => publishRelayListByKind(10088)}
						disabled={publishingKind !== null || count10088 === 0 || !app.myPubkey}
						title={!app.myPubkey
							? 'Sign in first to publish.'
							: count10088 === 0
								? 'No broadcast relays toggled.'
								: `Sign a kind 10088 (broadcast list, encrypted) with ${count10088} relay${count10088 === 1 ? '' : 's'}.`}
					>
						{publishingKind === 10088 ? 'Publishing…' : `Publish kind 10088 (${count10088})`}
					</button>
				</div>
			</div>
		</div>
		<div class="relays-list">
			{#each rows as row (row.url)}
				{@const status = statusFor(row.url)}
				{@const doc = docFor(row.url)}
				{@const lim = doc?.limitation}
				<div class="relay-card" class:relay-card--expanded={expanded.has(row.url)} bind:this={rowEls[row.url]}>
					<div class="relay-row">
						<button
							class="relay-disclosure"
							onclick={() => toggleExpanded(row.url)}
							aria-expanded={expanded.has(row.url)}
							title={expanded.has(row.url) ? 'Collapse' : 'Show NIP-11 details'}
						>{expanded.has(row.url) ? '▾' : '▸'}</button>

						<div class="relay-id">
							<span class="relay-url">{shorten(row.url)}</span>
							<div class="relay-flags">
								{#if status.state === 'loading'}
									<span class="pill pill--ghost"><span class="dot dot--fetching"></span>info</span>
								{:else if status.state === 'failed'}
									<span class="pill pill--ghost" title={status.error}>info: {status.error.slice(0, 24)}</span>
								{:else if doc}
									{#if lim?.payment_required}
										<span class="pill pill--draft" title="Payment required">paid</span>
									{/if}
									{#if lim?.auth_required}
										<span class="pill pill--imported" title="NIP-42 auth required">auth</span>
									{/if}
									{#if lim?.restricted_writes}
										<span class="pill pill--diverged" title="Writes restricted">restricted</span>
									{/if}
									{#if doc.software}
										<span class="pill pill--ghost" title="{doc.software}{doc.version ? ` ${doc.version}` : ''}">{doc.software.split('/').pop()}</span>
									{/if}
								{/if}
							</div>
						</div>

						<div class="relay-toggles">
							<button
								class="pill toggle-pill"
								class:toggle-pill--on={row.read}
								onclick={() => toggle(row.url, 'read')}
								title="Read from this relay"
							>read</button>
							<button
								class="pill toggle-pill"
								class:toggle-pill--on={row.write}
								onclick={() => toggle(row.url, 'write')}
								title="Publish to this relay (your own signed events land here)"
							>write</button>
							<button
								class="pill toggle-pill toggle-pill--broadcast"
								class:toggle-pill--on={row.broadcast}
								onclick={() => toggle(row.url, 'broadcast')}
								title="Mark this relay as a broadcast / aggregator target. Never auto-published to — only when you explicitly opt in per event."
							>broadcast</button>
							<button
								class="pill toggle-pill"
								class:toggle-pill--on={row.auth}
								onclick={() => toggle(row.url, 'auth')}
								title="Authenticate (NIP-42) when this relay challenges"
							>auth</button>
						</div>
					</div>

					{#if expanded.has(row.url)}
						<div class="relay-detail">
							{#if status.state === 'loading'}
								<p class="empty muted">Fetching NIP-11…</p>
							{:else if status.state === 'failed'}
								<div class="failed-detail">
									<p class="empty error">Couldn't load NIP-11: {status.error}</p>
									<button class="btn-refresh" onclick={() => primeInfo(row.url, true)}>Retry</button>
								</div>
							{:else if doc}
								{#if doc.name || doc.description}
									<section class="info-section">
										{#if doc.name}<h3 class="info-title">{doc.name}</h3>{/if}
										{#if doc.description}<p class="info-desc">{doc.description}</p>{/if}
									</section>
								{/if}

								{#if doc.software || doc.version || doc.contact || doc.pubkey}
									<section class="info-section">
										<div class="info-section-title">Software</div>
										<dl class="kv">
											{#if doc.software}<dt>software</dt><dd class="mono">{doc.software}</dd>{/if}
											{#if doc.version}<dt>version</dt><dd class="mono">{doc.version}</dd>{/if}
											{#if doc.contact}<dt>contact</dt><dd>{doc.contact}</dd>{/if}
											{#if doc.pubkey}<dt>operator</dt><dd><ProfileName pubkey={doc.pubkey} /></dd>{/if}
										</dl>
									</section>
								{/if}

								{#if doc.supported_nips && doc.supported_nips.length > 0}
									<section class="info-section">
										<div class="info-section-title">Supported NIPs</div>
										<div class="nip-chips">
											{#each doc.supported_nips as nip (nip)}
												<a
													class="nip-chip"
													href={`https://github.com/nostr-protocol/nips/blob/master/${String(nip).padStart(2, '0')}.md`}
													target="_blank"
													rel="noopener noreferrer"
													title="Open NIP-{nip} in a new tab"
												>{nip}</a>
											{/each}
										</div>
									</section>
								{/if}

								{#if lim}
									<section class="info-section">
										<div class="info-section-title">Limitations</div>
										{#if lim.max_message_length || lim.max_event_tags || lim.max_content_length || lim.max_subscriptions || lim.max_limit || lim.min_pow_difficulty}
											<div class="info-subtitle">Sizes &amp; throughput</div>
											<dl class="kv">
												{#if lim.max_message_length}
													<dt title="Maximum bytes in any single client→relay message">max message</dt>
													<dd>{lim.max_message_length.toLocaleString()} bytes</dd>
												{/if}
												{#if lim.max_event_tags}
													<dt title="Maximum tags on a single event">max tags</dt>
													<dd>{lim.max_event_tags}</dd>
												{/if}
												{#if lim.max_content_length}
													<dt title="Maximum bytes in an event's content field">max content</dt>
													<dd>{lim.max_content_length.toLocaleString()} bytes</dd>
												{/if}
												{#if lim.max_subscriptions}
													<dt title="Maximum concurrent REQ subscriptions on one connection (not per second)">max subscriptions</dt>
													<dd>{lim.max_subscriptions}</dd>
												{/if}
												{#if lim.max_limit}
													<dt title="Largest value the relay accepts in a filter's `limit` field">max filter limit</dt>
													<dd>{lim.max_limit}</dd>
												{/if}
												{#if lim.min_pow_difficulty}
													<dt title="Minimum NIP-13 proof-of-work difficulty (leading zero bits)">min PoW</dt>
													<dd>{lim.min_pow_difficulty} bits</dd>
												{/if}
											</dl>
										{/if}

										{#if lim.auth_required || lim.payment_required || lim.restricted_writes}
											<div class="info-subtitle">Access</div>
											<dl class="kv">
												{#if lim.auth_required}
													<dt title="Relay challenges connections with NIP-42 auth before serving">auth required</dt>
													<dd>yes</dd>
												{/if}
												{#if lim.payment_required}
													<dt title="Relay requires payment (see Fees) before accepting events">payment required</dt>
													<dd>yes</dd>
												{/if}
												{#if lim.restricted_writes}
													<dt title="Anyone can read; only members can publish">restricted writes</dt>
													<dd>yes</dd>
												{/if}
											</dl>
										{/if}

										{#if (lim.created_at_lower_limit ?? 0) > 0 || (lim.created_at_upper_limit ?? 0) > 0}
											<div class="info-subtitle">Event time bounds</div>
											<dl class="kv">
												{#if (lim.created_at_lower_limit ?? 0) > 0}
													<dt title="Events with `created_at` older than this (Unix seconds) are rejected">created_at min</dt>
													<dd>{lim.created_at_lower_limit}</dd>
												{/if}
												{#if (lim.created_at_upper_limit ?? 0) > 0}
													<dt title="Events with `created_at` newer than this (Unix seconds) are rejected">created_at max</dt>
													<dd>{lim.created_at_upper_limit}</dd>
												{/if}
											</dl>
										{/if}
									</section>
								{/if}

								{#if doc.fees && (doc.fees.admission?.length || doc.fees.subscription?.length || doc.fees.publication?.length)}
									<section class="info-section">
										<div class="info-section-title">Fees</div>
										<dl class="kv">
											{#each doc.fees.admission ?? [] as fee, i (`a${i}`)}
												<dt>admission</dt><dd>{fee.amount} {fee.unit}</dd>
											{/each}
											{#each doc.fees.subscription ?? [] as fee, i (`s${i}`)}
												<dt>subscription</dt><dd>{fee.amount} {fee.unit}{fee.period ? ` / ${fee.period}s` : ''}</dd>
											{/each}
											{#each doc.fees.publication ?? [] as fee, i (`p${i}`)}
												<dt>publication{fee.kinds ? ` (k:${fee.kinds.join(',')})` : ''}</dt>
												<dd>{fee.amount} {fee.unit}</dd>
											{/each}
										</dl>
									</section>
								{/if}

								{#if (doc.tags && doc.tags.length) || (doc.relay_countries && doc.relay_countries.length) || (doc.language_tags && doc.language_tags.length)}
									<section class="info-section">
										<div class="info-section-title">Audience</div>
										<dl class="kv">
											{#if doc.tags?.length}<dt>tags</dt><dd>{doc.tags.join(', ')}</dd>{/if}
											{#if doc.relay_countries?.length}<dt>countries</dt><dd>{doc.relay_countries.join(', ')}</dd>{/if}
											{#if doc.language_tags?.length}<dt>languages</dt><dd>{doc.language_tags.join(', ')}</dd>{/if}
										</dl>
									</section>
								{/if}

								{#if doc.privacy_policy || doc.terms_of_service || doc.posting_policy}
									<section class="info-section">
										<div class="info-section-title">Policies</div>
										<dl class="kv">
											{#if doc.privacy_policy}<dt>privacy</dt><dd><a href={doc.privacy_policy} target="_blank" rel="noopener noreferrer">{doc.privacy_policy}</a></dd>{/if}
											{#if doc.terms_of_service}<dt>terms</dt><dd><a href={doc.terms_of_service} target="_blank" rel="noopener noreferrer">{doc.terms_of_service}</a></dd>{/if}
											{#if doc.posting_policy}<dt>posting</dt><dd><a href={doc.posting_policy} target="_blank" rel="noopener noreferrer">{doc.posting_policy}</a></dd>{/if}
										</dl>
									</section>
								{/if}
							{:else}
								<p class="empty muted">No NIP-11 fetched yet.</p>
							{/if}
						</div>
					{/if}
				</div>
			{/each}
		</div>

		<!-- Discovery section — Search + Indexer subsections. Each
		     relay is in EITHER default OR fallback per class (mutex
		     enforced server-side). Per-class `exclusive` toggle in the
		     subsection header controls whether the read relays are
		     bypassed for that lookup type. -->
		<div class="discovery">
			<div class="discovery-header-row">
				<span class="discovery-header">Discovery</span>
				<button
					class="btn-add"
					onclick={restoreDefaults}
					disabled={restoringDefaults}
					title="Add the engine's well-known indexer relays (purplepag.es, user.kindpag.es) to indexer.default if missing. Idempotent — relays already present are skipped."
				>
					{restoringDefaults ? 'Restoring…' : 'Restore defaults'}
				</button>
			</div>
			{#each ['search', 'indexer'] as klass (klass)}
				{@const rows0 = klass === 'search' ? searchRows : indexerRows}
				{@const excl = klass === 'search' ? searchExclusive : indexerExclusive}
				{@const publishKind = klass === 'search' ? 10007 : 10086}
				{@const publishCount = rows0.length}
				<div class="discovery-subsection">
					<div class="discovery-sub-head">
						<span class="discovery-sub-title">{klass === 'search' ? 'Search relays' : 'Indexer relays'}</span>
						<button
							class="pill toggle-pill discovery-excl"
							class:toggle-pill--on={excl}
							onclick={() => toggleExclusive(klass as DiscoveryClass)}
							title={excl
								? `Exclusive ON — ${klass} lookups bypass read relays entirely. Toggle off to ALSO fan out across read relays.`
								: `Exclusive OFF — ${klass}.default joins read relays in the primary fan-out. Toggle on to use ${klass} relays ONLY.`}
						>exclusive: {excl ? 'on' : 'off'}</button>
						<span class="discovery-spacer"></span>
						<button
							class="btn-add" onclick={() => promptAddDiscovery(klass as DiscoveryClass)}
							title="Add a {klass} relay (lands in .default — move to fallback after)"
						>+ Add</button>
						<button
							class="btn-publish-list btn-publish-list--{klass}"
							onclick={() => publishRelayListByKind(publishKind)}
							disabled={publishingKind !== null || publishCount === 0 || !app.myPubkey}
							title={!app.myPubkey
								? 'Sign in first to publish.'
								: publishCount === 0
									? `No ${klass} relays configured.`
									: `Sign a kind ${publishKind} (${klass} list, NIP-44 encrypted) with ${publishCount} relay${publishCount === 1 ? '' : 's'}.`}
						>
							{publishingKind === publishKind ? 'Publishing…' : `Publish kind ${publishKind} (${publishCount})`}
						</button>
					</div>
					{#if rows0.length === 0}
						<p class="empty muted">No {klass} relays. Add one above, or use "Pull from your profile" to import an existing list.</p>
					{:else}
						<div class="discovery-list">
							{#each rows0 as drow (drow.url)}
								<div class="discovery-row">
									<div class="discovery-tier-group">
										<label class="discovery-tier">
											<input
												type="radio"
												name="{klass}-tier-{drow.url}"
												checked={drow.tier === 'default'}
												onchange={() => setTier(klass as DiscoveryClass, drow.url, 'default')}
											/>
											<span>default</span>
										</label>
										<label class="discovery-tier">
											<input
												type="radio"
												name="{klass}-tier-{drow.url}"
												checked={drow.tier === 'fallback'}
												onchange={() => setTier(klass as DiscoveryClass, drow.url, 'fallback')}
											/>
											<span>fallback</span>
										</label>
									</div>
									<span class="discovery-url">{shorten(drow.url)}</span>
									<button
										class="pull-add discovery-remove"
										onclick={() => removeDiscovery(klass as DiscoveryClass, drow.url)}
										title="Remove from {klass} entirely"
									>×</button>
								</div>
							{/each}
						</div>
					{/if}
				</div>
			{/each}
		</div>

		<!-- Named sets — NIP-51 kind 30002 thematic groupings. Orthogonal
		     to the functional classes above; a relay can sit in any
		     combination of class toggles AND any number of named sets. -->
		<div class="named-sets">
			<div class="named-sets-header">
				<span>Named sets · kind 30002</span>
				<button class="btn-add" onclick={promptNewSet}>+ New set</button>
			</div>
			{#if namedSets.length === 0}
				<p class="empty muted">No named sets yet. Create one to publish a NIP-51 relay set (e.g. "research", "friends-only").</p>
			{:else}
				{#each namedSets as set (set.d_tag)}
					<div class="named-set" class:named-set--expanded={expandedSet === set.d_tag}>
						<div class="named-set-row">
							<button
								class="relay-disclosure"
								onclick={() => (expandedSet = expandedSet === set.d_tag ? null : set.d_tag)}
								aria-expanded={expandedSet === set.d_tag}
							>{expandedSet === set.d_tag ? '▾' : '▸'}</button>
							<button class="named-set-title" onclick={() => renameSet(set.d_tag, set.title)} title="Click to rename">
								{set.title}
							</button>
							<span class="named-set-count">{set.urls.length} relay{set.urls.length === 1 ? '' : 's'}</span>
							<div class="named-set-actions">
								<button
									class="pull-add"
									onclick={() => promptAddToSet(set.d_tag)}
									title="Add a relay URL to this set"
								>+ relay</button>
								<button
									class="btn-publish-list btn-publish-list--read"
									onclick={() => publishNamedSet(set)}
									disabled={publishingSetTag !== null || set.urls.length === 0 || !app.myPubkey}
									title={!app.myPubkey
										? 'Sign in first.'
										: set.urls.length === 0
											? 'Empty set — add relays first.'
											: `Sign + broadcast a kind 30002 with d="${set.d_tag.slice(0, 8)}…" and ${set.urls.length} relays.`}
								>
									{publishingSetTag === set.d_tag ? 'Publishing…' : 'Publish 30002'}
								</button>
								<button
									class="pull-add"
									onclick={() => deleteSet(set.d_tag, set.title)}
									title="Delete this set locally (does not publish a delete event)"
								>delete</button>
							</div>
						</div>
						{#if expandedSet === set.d_tag}
							<div class="named-set-members">
								{#if set.urls.length === 0}
									<p class="empty muted">No members yet.</p>
								{:else}
									{#each set.urls as url (url)}
										<div class="named-set-member">
											<span class="named-set-member-url">{shorten(url)}</span>
											<button
												class="pull-add"
												onclick={() => removeMemberFromSet(set.d_tag, url)}
												title="Remove this relay from the set"
											>−</button>
										</div>
									{/each}
								{/if}
							</div>
						{/if}
					</div>
				{/each}
			{/if}
		</div>

		<!-- Phase 5+: per-class publish buttons live in their section
		     headers; global Refresh + Save Settings moved to the top
		     header (no more sticky bottom footer). -->
	{/if}
</div>

<style>
	.relays-view {
		flex: 1;
		overflow-y: auto;
		min-height: 0;
		padding: 0 0 24px;
	}

	.relays-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
		padding: 10px 14px;
		font-size: var(--t-xs);
		font-weight: 600;
		color: var(--base6);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		border-bottom: 1px solid var(--panel-border);
	}
	.relays-header-title {
		display: flex;
		flex-direction: column;
		gap: 2px;
		min-width: 0;
	}
	.relays-header-actions {
		display: flex;
		gap: 8px;
		text-transform: none;
		letter-spacing: 0;
		font-weight: 400;
	}

	.relays-hint {
		font-weight: 400;
		text-transform: none;
		letter-spacing: 0;
		color: var(--base5);
		font-style: italic;
	}

	.empty {
		padding: 24px;
		text-align: center;
		color: var(--base5);
		font-size: var(--t-sm);
	}
	.empty.error { color: var(--id-draft); }
	.empty.muted { color: var(--base5); }

	.failed-detail {
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		gap: 6px;
	}
	.failed-detail .empty {
		padding: 8px 0;
		text-align: left;
	}

	/* ----- Read/Write/Broadcast section header (Phase 5) ----- */
	.rw-section {
		display: flex;
		flex-direction: column;
		margin: 8px 0 4px;
	}
	.rw-section-head {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 4px 0;
		flex-wrap: wrap;
	}
	.rw-section-title {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--muted);
		text-transform: uppercase;
		letter-spacing: 0.04em;
		margin-right: auto;
	}
	.rw-section-actions {
		display: flex;
		gap: 6px;
		flex-wrap: wrap;
		align-items: center;
	}

	/* ----- Discovery section (Phase 5) ----- */
	.discovery {
		display: flex;
		flex-direction: column;
		gap: 10px;
		margin: 12px 0;
		padding-top: 10px;
		border-top: 1px solid var(--panel-border);
	}
	.discovery-header-row {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
	}
	.discovery-header {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--muted);
		text-transform: uppercase;
		letter-spacing: 0.04em;
	}
	.discovery-subsection {
		display: flex;
		flex-direction: column;
		gap: 4px;
	}
	.discovery-sub-head {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 4px 0;
		flex-wrap: wrap;
	}
	.discovery-sub-title {
		font-weight: 500;
		font-size: var(--t-sm);
	}
	.discovery-excl {
		font-size: var(--t-xs);
		font-family: var(--font-mono);
		padding: 2px 8px;
	}
	.discovery-spacer {
		flex: 1;
	}
	.discovery-list {
		display: flex;
		flex-direction: column;
		gap: 2px;
		padding-left: 4px;
	}
	.discovery-row {
		display: grid;
		grid-template-columns: auto 1fr auto;
		gap: 10px;
		align-items: center;
		padding: 3px 6px;
		border-radius: 3px;
	}
	.discovery-row:hover {
		background: color-mix(in srgb, var(--fg) 4%, transparent);
	}
	.discovery-tier-group {
		display: flex;
		gap: 8px;
	}
	.discovery-tier {
		display: inline-flex;
		gap: 4px;
		align-items: center;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--muted);
		cursor: pointer;
	}
	.discovery-tier input[type='radio'] {
		margin: 0;
	}
	.discovery-tier:has(input:checked) {
		color: var(--fg);
	}
	.discovery-url {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.discovery-remove {
		font-size: var(--t-sm);
		line-height: 1;
		padding: 0 6px;
	}

	.relays-list {
		display: flex;
		flex-direction: column;
		gap: 4px;
		padding: 6px 0;
	}

	.relay-card {
		border-bottom: 1px solid var(--panel-border);
	}
	.relay-card--expanded {
		background: var(--bg-surface);
	}

	.relay-row {
		display: flex;
		align-items: center;
		gap: 10px;
		padding: 8px 14px;
	}

	.relay-disclosure {
		background: transparent;
		border: none;
		color: var(--fg-muted);
		font-size: 0.8rem;
		min-width: 18px;
		cursor: pointer;
		padding: 0;
	}
	.relay-disclosure:hover { color: var(--fg); }

	.relay-id {
		flex: 1;
		display: flex;
		align-items: center;
		gap: 8px;
		min-width: 0;
	}

	.relay-url {
		font-family: var(--font-mono);
		font-size: var(--t-sm);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.relay-flags {
		display: flex;
		gap: 4px;
		flex-wrap: wrap;
	}

	.relay-toggles {
		display: flex;
		gap: 4px;
		flex-shrink: 0;
	}

	/* Toggle pills: ghost outline when off, filled-tinted when on. The
	   "on" tints reuse pill--online so all three toggles read as "this
	   relay carries this role." */
	.toggle-pill {
		border: 1px solid var(--base3);
		background: transparent;
		color: var(--base6);
		cursor: pointer;
		font-family: var(--font-mono);
		padding: 1px 8px;
	}
	.toggle-pill:hover {
		color: var(--fg);
	}
	.toggle-pill--on {
		background: rgba(180, 190, 130, 0.14);
		color: var(--state-online);
		border-color: color-mix(in srgb, var(--state-online) 50%, transparent);
	}
	/* Broadcast / search / indexer are functionally distinct from
	   read/write — different tints so the user reads them as "different
	   class, deliberate opt-in" rather than read/write variants. */
	.toggle-pill--broadcast.toggle-pill--on {
		background: color-mix(in srgb, var(--id-draft) 14%, transparent);
		color: var(--id-draft);
		border-color: color-mix(in srgb, var(--id-draft) 50%, transparent);
	}
	.toggle-pill--search.toggle-pill--on {
		background: color-mix(in srgb, var(--id-remote, var(--id-yours)) 14%, transparent);
		color: var(--id-remote, var(--id-yours));
		border-color: color-mix(in srgb, var(--id-remote, var(--id-yours)) 50%, transparent);
	}
	.toggle-pill--indexer.toggle-pill--on {
		background: color-mix(in srgb, var(--id-imported, var(--id-yours)) 14%, transparent);
		color: var(--id-imported, var(--id-yours));
		border-color: color-mix(in srgb, var(--id-imported, var(--id-yours)) 50%, transparent);
	}
	.toggle-pill--on:hover {
		filter: brightness(1.15);
	}

	.relay-detail {
		padding: 4px 14px 16px 38px;
		display: flex;
		flex-direction: column;
		gap: 12px;
	}

	.info-section {
		display: flex;
		flex-direction: column;
		gap: 4px;
	}
	.info-section-title {
		font-size: var(--t-xs);
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--base5);
	}
	.info-subtitle {
		font-size: var(--t-xs);
		color: var(--base6);
		margin-top: 4px;
	}
	.info-title {
		font-size: var(--t-md);
		margin: 0;
	}
	.info-desc {
		font-size: var(--t-sm);
		margin: 0;
		color: var(--fg);
	}

	.kv {
		display: grid;
		grid-template-columns: 110px 1fr;
		gap: 2px 12px;
		margin: 0;
		font-size: var(--t-xs);
	}
	.kv dt {
		color: var(--base5);
		font-family: var(--font-mono);
	}
	.kv dd {
		margin: 0;
		color: var(--fg);
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.kv .mono {
		font-family: var(--font-mono);
	}
	.kv a {
		color: var(--accent);
	}

	.nip-chips {
		display: flex;
		flex-wrap: wrap;
		gap: 4px;
	}
	.nip-chip {
		display: inline-block;
		padding: 1px 8px;
		border-radius: var(--r-md);
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		background: rgba(137, 184, 194, 0.12);
		color: var(--id-remote);
		text-decoration: none;
	}
	.nip-chip:hover {
		filter: brightness(1.15);
	}

	.btn-add,
	.btn-refresh {
		font-size: var(--t-xs);
		padding: 4px 10px;
		font-family: var(--font-mono);
	}
	.btn-snapshot[disabled],
	.btn-publish-list[disabled] {
		opacity: 0.5;
		cursor: not-allowed;
	}
	/* Dirty state — current in-memory relay set differs from
	   config.toml. Warm tint signals "you have unsaved changes." */
	.btn-snapshot--dirty:not([disabled]) {
		background: color-mix(in srgb, var(--id-forked) 22%, transparent);
		border-color: var(--id-forked);
		color: var(--id-forked);
	}
	.btn-snapshot--dirty:hover:not([disabled]) {
		background: color-mix(in srgb, var(--id-forked) 32%, transparent);
	}
	.btn-publish-list {
		font-size: var(--t-xs);
		padding: 4px 10px;
		font-family: var(--font-mono);
		background: color-mix(in srgb, var(--id-yours) 14%, transparent);
		border: 1px solid color-mix(in srgb, var(--id-yours) 35%, transparent);
		color: var(--id-yours);
		cursor: pointer;
		border-radius: var(--r-sm);
	}
	.btn-publish-list:hover:not([disabled]) {
		background: color-mix(in srgb, var(--id-yours) 24%, transparent);
	}
	/* Per-class tints so the four publish buttons line up visually with
	   the matching row toggles. */
	.btn-publish-list--broadcast {
		background: color-mix(in srgb, var(--id-draft) 14%, transparent);
		border-color: color-mix(in srgb, var(--id-draft) 35%, transparent);
		color: var(--id-draft);
	}
	.btn-publish-list--broadcast:hover:not([disabled]) {
		background: color-mix(in srgb, var(--id-draft) 24%, transparent);
	}
	.btn-publish-list--search {
		background: color-mix(in srgb, var(--id-remote, var(--id-yours)) 14%, transparent);
		border-color: color-mix(in srgb, var(--id-remote, var(--id-yours)) 35%, transparent);
		color: var(--id-remote, var(--id-yours));
	}
	.btn-publish-list--search:hover:not([disabled]) {
		background: color-mix(in srgb, var(--id-remote, var(--id-yours)) 24%, transparent);
	}
	.btn-publish-list--indexer {
		background: color-mix(in srgb, var(--id-imported, var(--id-yours)) 14%, transparent);
		border-color: color-mix(in srgb, var(--id-imported, var(--id-yours)) 35%, transparent);
		color: var(--id-imported, var(--id-yours));
	}
	.btn-publish-list--indexer:hover:not([disabled]) {
		background: color-mix(in srgb, var(--id-imported, var(--id-yours)) 24%, transparent);
	}

	/* Named sets — NIP-51 kind 30002. Sits between the functional-class
	   relay rows and the footer. */
	.named-sets {
		padding: 4px 14px 12px;
		border-top: 1px solid var(--panel-border);
	}
	.named-sets-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 8px 0 4px;
		font-size: var(--t-xs);
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--base5);
	}
	.named-set {
		border-top: 1px dashed var(--panel-border);
		padding: 6px 0;
	}
	.named-set:first-of-type {
		border-top: none;
	}
	.named-set-row {
		display: flex;
		align-items: center;
		gap: 8px;
	}
	.named-set-title {
		font-weight: 600;
		font-family: var(--font-mono);
		background: none;
		border: none;
		padding: 2px 4px;
		color: var(--fg);
		cursor: text;
		border-radius: var(--r-sm);
	}
	.named-set-title:hover {
		background: color-mix(in srgb, var(--fg) 6%, transparent);
	}
	.named-set-count {
		font-size: var(--t-xs);
		color: var(--base5);
		font-family: var(--font-mono);
	}
	.named-set-actions {
		display: flex;
		gap: 4px;
		margin-left: auto;
	}
	.named-set-members {
		padding: 4px 0 4px 24px;
		display: flex;
		flex-direction: column;
		gap: 2px;
	}
	.named-set-member {
		display: flex;
		align-items: center;
		gap: 8px;
		font-size: var(--t-xs);
	}
	.named-set-member-url {
		font-family: var(--font-mono);
		flex: 1;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.btn-snapshot {
		font-size: var(--t-xs);
		padding: 4px 10px;
		font-family: var(--font-mono);
		background: color-mix(in srgb, var(--id-yours) 14%, transparent);
		border: 1px solid color-mix(in srgb, var(--id-yours) 35%, transparent);
		color: var(--id-yours);
		cursor: pointer;
		margin-left: auto;
		border-radius: var(--r-sm);
	}
	.btn-snapshot:hover:not([disabled]) {
		background: color-mix(in srgb, var(--id-yours) 24%, transparent);
	}

	/* Pull-from-profile bar + suggestion list. Suggestions are deliberately
	   chip-styled (not row-styled like configured relays) so they read as
	   "external suggestion, click to accept" rather than "live config."
	   Per project_publishing_philosophy.md, suggestions never auto-apply. */
	.pull-bar {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 8px;
		padding: 8px 14px;
		border-bottom: 1px solid var(--panel-border);
	}
	.btn-pull {
		font-size: var(--t-xs);
		padding: 3px 10px;
		font-family: var(--font-mono);
		background: color-mix(in srgb, var(--id-remote, var(--id-yours)) 14%, transparent);
		border: 1px solid color-mix(in srgb, var(--id-remote, var(--id-yours)) 35%, transparent);
		color: var(--id-remote, var(--fg));
		cursor: pointer;
		border-radius: var(--r-sm);
	}
	.btn-pull:hover:not([disabled]) {
		background: color-mix(in srgb, var(--id-remote, var(--id-yours)) 24%, transparent);
	}
	.btn-pull[disabled] {
		opacity: 0.45;
		cursor: not-allowed;
	}
	.btn-pull--small {
		font-size: 0.7rem;
		padding: 2px 8px;
	}
	.pull-hint {
		font-size: var(--t-xs);
		color: var(--base5);
		font-style: italic;
	}
	.pull-hint--warn {
		color: var(--id-draft);
		font-style: normal;
	}
	.pull-hint code {
		font-family: var(--font-mono);
		font-style: normal;
	}
	.pull-status {
		font-size: var(--t-xs);
		color: var(--base6);
	}
	.pull-status--err {
		color: var(--id-draft);
	}
	.pulled-list {
		padding: 8px 14px 10px 14px;
		display: flex;
		flex-direction: column;
		gap: 4px;
		background: color-mix(in srgb, var(--id-remote, var(--id-yours)) 4%, transparent);
		border-bottom: 1px solid var(--panel-border);
	}
	.pulled-label {
		font-size: var(--t-xs);
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--base5);
		margin-bottom: 2px;
	}
	.pulled-kind-label {
		font-size: 0.7rem;
		font-family: var(--font-mono);
		color: var(--base5);
		margin-top: 6px;
		padding-top: 4px;
		border-top: 1px dashed var(--panel-border);
	}
	.pulled-kind-label:first-of-type {
		border-top: none;
		padding-top: 0;
	}
	.pulled-encrypted {
		font-size: var(--t-xs);
		color: var(--base6);
		font-style: italic;
		margin: 4px 0 0;
	}
	.pull-diagnostics {
		display: flex;
		flex-wrap: wrap;
		gap: 4px 10px;
		padding: 4px 14px 10px;
		border-bottom: 1px solid var(--panel-border);
		font-size: 0.7rem;
		font-family: var(--font-mono);
	}
	.pull-diag--ok { color: var(--state-online); }
	.pull-diag--enc { color: var(--id-draft); }
	.pull-diag--missing { color: var(--base5); }
	.decrypt-errors {
		display: grid;
		grid-template-columns: auto 1fr;
		gap: 2px 10px;
		margin: 4px 0 0;
		font-family: var(--font-mono);
		font-size: 0.7rem;
	}
	.decrypt-errors dt { color: var(--id-draft); }
	.decrypt-errors dd { color: var(--base6); margin: 0; }
	.pulled-row {
		display: flex;
		align-items: center;
		gap: 8px;
		font-size: var(--t-xs);
	}
	.pulled-row--set {
		padding: 2px 0;
	}
	.pulled-set-title {
		font-weight: 500;
		flex: 1;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.pulled-set-meta {
		color: var(--muted);
		font-family: var(--font-mono);
		font-size: 0.7rem;
	}
	.pulled-url {
		font-family: var(--font-mono);
		flex: 1;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.pulled-marker {
		font-family: var(--font-mono);
		color: var(--base5);
		font-size: 0.7rem;
	}
	.pulled-state {
		font-family: var(--font-mono);
		font-size: 0.7rem;
		color: var(--base5);
		font-style: italic;
	}
	.pulled-actions {
		display: flex;
		gap: 4px;
	}
	.pull-add {
		font-size: 0.7rem;
		padding: 2px 7px;
		font-family: var(--font-mono);
		background: none;
		border: 1px solid var(--base3);
		color: var(--base6);
		cursor: pointer;
		border-radius: var(--r-sm);
	}
	.pull-add:hover {
		color: var(--fg);
		border-color: var(--fg-muted);
	}
	.pull-add--strong {
		background: color-mix(in srgb, var(--state-online) 14%, transparent);
		border-color: color-mix(in srgb, var(--state-online) 40%, transparent);
		color: var(--state-online);
	}
</style>
