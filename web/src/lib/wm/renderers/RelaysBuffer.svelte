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
	type RelayRow = {
		url: string;
		read: boolean;
		write: boolean;
		auth: boolean;
		broadcast: boolean;
		search: boolean;
		indexer: boolean;
	};

	let rows = $state<RelayRow[]>([]);
	let initialRelays = $state<string[]>([]);
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
	let pulled = $state<PulledRelay[] | null>(null);
	let pulling = $state(false);
	let pullError = $state<string | null>(null);
	let pullCreatedAt = $state<number | null>(null);
	let pullEncryptedKinds = $state<number[]>([]);
	// Per-kind result tracking — the user wants to know which kinds
	// returned events vs. which came up empty when "pulled in indexer
	// and search relays" doesn't show what they expected.
	type PullKindResult = 'parsed' | 'encrypted' | 'not_found';
	let pullKindResults = $state<Record<10002 | 10007 | 10086 | 10088, PullKindResult> | null>(null);
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
			const map = new Map<string, RelayRow>();
			const ensure = (url: string): RelayRow => {
				let r = map.get(url);
				if (!r) {
					r = {
						url,
						read: false,
						write: false,
						auth: false,
						broadcast: false,
						search: false,
						indexer: false
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
			for (const url of cfg.search?.urls ?? []) ensure(url).search = true;
			for (const url of cfg.indexer?.urls ?? []) ensure(url).indexer = true;
			rows = [...map.values()].sort((a, b) => a.url.localeCompare(b.url));
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

	type ToggleField = 'read' | 'write' | 'auth' | 'broadcast' | 'search' | 'indexer';
	async function toggle(url: string, field: ToggleField) {
		const row = rows.find((r) => r.url === url);
		if (!row) return;
		const next = { ...row, [field]: !row[field] };
		rows = rows.map((r) => (r.url === url ? next : r)); // optimistic

		// `auth` has no config home yet — keep it cosmetic.
		if (field === 'auth') return;

		try {
			if (field === 'broadcast' || field === 'search' || field === 'indexer') {
				await (next[field] ? api.addRelay(field, url) : api.removeRelay(field, url));
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
		if (initialRelays.length === 0) {
			pullError = 'No initial relays configured in config.toml. Add `initial_relays = [...]` under `[relay]` to seed.';
			return;
		}
		pulling = true;
		pullError = null;
		pullEncryptedKinds = [];
		pullKindResults = null;
		pullFetchedCount = 0;
		pullDecryptReason = null;
		pullDecryptErrors = {};
		try {
			// 1. Pull all four relay-list kinds from the seed relays.
			//    10002 = read/write (NIP-65, public `r` tags).
			//    10007 = search (NIP-50). 10086 = indexer. 10088 = broadcast.
			//    The 100xx kinds are Amethyst-defined and published as a
			//    NIP-44-encrypted PrivateTagArrayEvent (Amethyst convention)
			//    — the actual relay URLs sit in the encrypted `content` as a
			//    JSON tag-array of `["relay", url]` entries. If the user is
			//    signed in via a NIP-07 extension that exposes nip44.decrypt,
			//    we attempt decryption and merge those entries with any
			//    public `r` tags. Engine-side decrypt is tracked as T32.
			const kinds = [10002, 10007, 10086, 10088] as const;
			const fetchResult = await api.fetchFromRelay(
				initialRelays,
				[...kinds],
				[pubkey],
				20
			);
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

			// 2. Read them back from the local cache. Newest per kind wins.
			const entries: PulledRelay[] = [];
			const seen = new Set<string>(); // dedup by `${source_kind}:${normalizedUrl}`
			let maxCreatedAt = 0;
			const encrypted: number[] = [];
			const kindResults: Record<10002 | 10007 | 10086 | 10088, PullKindResult> = {
				10002: 'not_found',
				10007: 'not_found',
				10086: 'not_found',
				10088: 'not_found'
			};
			const push = (s: PulledRelay) => {
				const key = `${s.source_kind}:${normalizeRelayUrl(s.url)}`;
				if (seen.has(key)) return;
				seen.add(key);
				entries.push(s);
			};

			for (const kind of kinds) {
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
		} catch (e) {
			pullError = e instanceof Error ? e.message : String(e);
		} finally {
			pulling = false;
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
			if (role === 'search') await api.addRelay('search', s.url);
			if (role === 'indexer') await api.addRelay('indexer', s.url);
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
		const field: keyof RelayRow = kind === 10007 ? 'search' : kind === 10086 ? 'indexer' : 'broadcast';
		return rows.filter((r) => r[field]).map((r) => r.url);
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

	let snapshotting = $state(false);
	async function snapshotToConfig() {
		snapshotting = true;
		try {
			const resp = await api.snapshotConfig();
			app.pushToast(resp.message, 'success', 3500);
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
		<span>Relay configuration</span>
		<span class="relays-hint">read/write apply live and persist · auth is cosmetic</span>
	</div>

	<!-- Pull-from-profile: fetches the user's kind 10002 (NIP-65) from
	     the configured initial_relays and surfaces it as suggestions.
	     Suggestions never auto-apply; the user picks per relay. -->
	<div class="pull-bar">
		{#if !pulled && !pulling && !pullError}
			<button
				class="btn-pull"
				onclick={pullFromProfile}
				disabled={!app.myPubkey || initialRelays.length === 0}
				title={!app.myPubkey
					? 'Sign in first'
					: initialRelays.length === 0
						? 'No initial_relays in config.toml'
						: `Fetch your published relay list (kind 10002) from ${initialRelays.length} initial relay${initialRelays.length === 1 ? '' : 's'}`}
			>
				Pull from your profile
			</button>
			{#if !app.myPubkey}
				<span class="pull-hint pull-hint--warn">Sign in first to fetch your relay list.</span>
			{:else if initialRelays.length === 0}
				<span class="pull-hint pull-hint--warn">
					No <code>initial_relays</code> configured. Add a few in <code>config.toml</code> under <code>[relay]</code> (e.g. <code>initial_relays = ["wss://relay.damus.io", "wss://nos.lol"]</code>) and restart — or add relays directly below.
				</span>
			{:else}
				<span class="pull-hint">Reads your kind 10002 from <code>initial_relays</code> ({initialRelays.length} configured); you choose what to import.</span>
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
		</div>
	{/if}

	{#if (pulled && pulled.length > 0) || pullEncryptedKinds.length > 0}
		<div class="pulled-list">
			<div class="pulled-label">From your profile · suggestions</div>

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
									{#if !existing?.search}
										<button class="pull-add" onclick={() => importSuggestion(s, 'search')}>+ search</button>
									{/if}
								{:else if kind === 10086}
									{#if !existing?.indexer}
										<button class="pull-add" onclick={() => importSuggestion(s, 'indexer')}>+ indexer</button>
									{/if}
								{:else if kind === 10088}
									{#if !existing?.broadcast}
										<button class="pull-add" onclick={() => importSuggestion(s, 'broadcast')}>+ broadcast</button>
									{/if}
								{/if}
								{#if existing && existing[kind === 10002 ? 'read' : kind === 10007 ? 'search' : kind === 10086 ? 'indexer' : 'broadcast']}
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
								class="pill toggle-pill toggle-pill--search"
								class:toggle-pill--on={row.search}
								onclick={() => toggle(row.url, 'search')}
								title="Mark this relay as NIP-50 search-capable. Used for `~:` semantic queries when per-class routing lands."
							>search</button>
							<button
								class="pill toggle-pill toggle-pill--indexer"
								class:toggle-pill--on={row.indexer}
								onclick={() => toggle(row.url, 'indexer')}
								title="Mark this relay as an indexer / discovery fallback (purplepag.es etc.). Queried only when the read set misses on profile / kind 10002 / metadata lookups."
							>indexer</button>
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

		<div class="relays-footer">
			<button class="btn-add" onclick={promptAdd} title="Add a new relay (defaults to read + write — toggle either off after)">+ Add relay</button>
			<button class="btn-refresh" onclick={() => load(true)}>Refresh</button>
			{#each [10002, 10007, 10086, 10088] as kind (kind)}
				{@const count = relaysForPublish(kind as PublishableKind).length}
				<button
					class="btn-publish-list"
					class:btn-publish-list--read={kind === 10002}
					class:btn-publish-list--search={kind === 10007}
					class:btn-publish-list--indexer={kind === 10086}
					class:btn-publish-list--broadcast={kind === 10088}
					onclick={() => publishRelayListByKind(kind as PublishableKind)}
					disabled={publishingKind !== null || count === 0 || !app.myPubkey}
					title={!app.myPubkey
						? 'Sign in first to publish.'
						: count === 0
							? `No ${classLabelForPublish(kind as PublishableKind)} relays toggled.`
							: `Sign a kind ${kind} (${classLabelForPublish(kind as PublishableKind)}) with ${count} relay${count === 1 ? '' : 's'} and broadcast to your publish set. ${kind !== 10002 ? 'Content is NIP-44 encrypted to your own pubkey (Amethyst convention).' : 'Public r-tags per NIP-65.'}`}
				>
					{publishingKind === kind ? 'Publishing…' : `Publish kind ${kind} (${count})`}
				</button>
			{/each}
			<button
				class="btn-snapshot"
				onclick={snapshotToConfig}
				disabled={snapshotting || rows.length === 0}
				title="Write the current relay set into config.toml's `initial_relays` — a portable bootstrap seed for another machine or a fresh data dir. relays.json stays the runtime source of truth."
			>
				{snapshotting ? 'Saving…' : 'Save settings'}
			</button>
		</div>
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
		align-items: baseline;
		justify-content: space-between;
		padding: 10px 14px;
		font-size: var(--t-xs);
		font-weight: 600;
		color: var(--base6);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		border-bottom: 1px solid var(--panel-border);
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

	.relays-footer {
		display: flex;
		gap: 8px;
		padding: 10px 14px;
		border-top: 1px solid var(--panel-border);
		margin-top: 8px;
		/* Pin to the bottom of the scrollable buffer so the action row
		   (especially "Save settings") is always reachable even when
		   the relay list scrolls. Background prevents row text from
		   showing through. */
		position: sticky;
		bottom: 0;
		background: var(--panel-bg, var(--bg));
		z-index: 1;
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
