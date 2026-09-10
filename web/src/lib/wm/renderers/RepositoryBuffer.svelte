<script lang="ts">
	// NIP-34 repository buffer — a profile-shaped page for one kind-30617
	// announcement: the repo's identity card, its issues (kind 1621, status
	// folded in engine-side), and per-issue NIP-22 threads with a reply box.
	//
	// Everything algorithmic (parsing, status resolution, template building,
	// relay selection) lives in src/git.rs; this component renders and holds
	// view state only. Buffer id: `repository:<owner-hex>:<d-tag>` — the
	// d-tag is the trailing segment so it may itself contain ':'.
	//
	// No Svelte lifecycle hooks (buffer renderers unmount on every switch):
	// props + $effect teardown, view state parked in store.bufferState.
	import { untrack } from 'svelte';
	import * as api from '$lib/api';
	import { getAppState } from '$lib/state.svelte';
	import { getActiveStore, type NavAction } from '../buffer-store.svelte';
	import { mobileNav } from '../mobile-nav.svelte';
	import type { Buffer } from '../types';
	import { identityCanSign } from '$lib/identity/signer';
	import ProfileName from '$lib/components/ProfileName.svelte';
	import RichContent from '$lib/components/RichContent.svelte';
	import CommentThread from '$lib/components/CommentThread.svelte';
	import ReplyBox from '$lib/components/ReplyBox.svelte';
	import type { ThreadNode } from '$lib/discussions/thread';
	import type { NostrEvent } from '$lib/types';
	import { prefetchAuthors } from '$lib/discussions/authors.svelte';

	let { buffer }: { buffer: Buffer } = $props();
	const store = getActiveStore();
	const app = getAppState();

	type View = 'issues' | 'about' | 'issue' | 'new';
	type Filter = 'open' | 'closed' | 'all';
	type RepoViewState = { view: View; issueId: string | null; filter: Filter; cursor: number };

	const coord = $derived(parseBufferId(buffer.id));
	function parseBufferId(id: string): { pubkey: string; dTag: string } | null {
		const m = id.match(/^repository:([0-9a-f]{64}):(.*)$/);
		return m ? { pubkey: m[1], dTag: m[2] } : null;
	}

	// svelte-ignore state_referenced_locally
	const saved = store.bufferState.get(buffer.id) as RepoViewState | undefined;
	let view = $state<View>(saved?.view ?? 'issues');
	let issueId = $state<string | null>(saved?.issueId ?? null);
	let filter = $state<Filter>(saved?.filter ?? 'open');
	let cursor = $state(saved?.cursor ?? 0);
	$effect(() => {
		return () => {
			store.bufferState.set(buffer.id, { view, issueId, filter, cursor } satisfies RepoViewState);
		};
	});

	// Mobile Back walks list → issue → list (a teleport, not a page turn).
	$effect(() => {
		const id = buffer.id;
		mobileNav.registerViewProvider(id, {
			capture: () => ({ view, issue: issueId ?? '' }),
			apply: (v) => {
				const nv = v.view as View;
				if (['issues', 'about', 'issue', 'new'].includes(nv)) view = nv;
				issueId = v.issue || null;
			}
		});
		return () => mobileNav.unregisterViewProvider(id);
	});

	let loaded = $state<api.LoadedRepository | null>(null);
	let loading = $state(true);
	let fetching = $state(false);
	let error = $state<string | null>(null);
	let listEl: HTMLDivElement | undefined = $state();

	const repo = $derived(loaded?.repository ?? null);
	const me = $derived(app.identityStatus?.pubkey ?? null);
	const canSign = $derived(identityCanSign(app.identityStatus));
	const isMaintainer = $derived(
		!!me && !!repo && (repo.pubkey === me || repo.maintainers.includes(me))
	);

	const filteredIssues = $derived.by(() => {
		const all = loaded?.issues ?? [];
		if (filter === 'all') return all;
		if (filter === 'open') return all.filter((i) => i.status === 'open' || i.status === 'draft');
		return all.filter((i) => i.status === 'resolved' || i.status === 'closed');
	});
	const closedCount = $derived(
		(loaded?.issues ?? []).filter((i) => i.status === 'resolved' || i.status === 'closed').length
	);
	const currentIssue = $derived(
		issueId ? (loaded?.issues.find((i) => i.id === issueId) ?? null) : null
	);

	async function load(pubkey: string, dTag: string, policy: 'local_only' | 'local_first' | 'fetch_always') {
		if (policy === 'fetch_always') fetching = true;
		else loading = true;
		error = null;
		try {
			const res = await api.getRepository(pubkey, dTag, policy);
			loaded = res;
			prefetchAuthors([
				res.repository.pubkey,
				...res.repository.maintainers,
				...res.issues.map((i) => i.pubkey)
			]);
			if (policy === 'fetch_always') {
				const n = res.fetched_from.length;
				app.pushToast(
					n ? `Repository refreshed from ${n} relay${n === 1 ? '' : 's'}` : 'Fetch declined — showing local copy',
					n ? 'success' : 'info'
				);
			}
		} catch (e) {
			if (!loaded) error = api.errorMessage(e, 'Failed to load repository');
			else app.pushToast(api.errorMessage(e, 'Refresh failed'), 'error', 4000);
		} finally {
			loading = false;
			fetching = false;
		}
	}

	$effect(() => {
		const c = coord;
		if (!c) return;
		untrack(() => {
			void load(c.pubkey, c.dTag, 'local_first');
		});
	});

	function refresh() {
		const c = coord;
		if (!c || fetching) return;
		void load(c.pubkey, c.dTag, 'fetch_always');
	}

	async function reloadLocal() {
		const c = coord;
		if (!c) return;
		try {
			loaded = await api.getRepository(c.pubkey, c.dTag, 'local_only');
		} catch {
			// Non-fatal: the toast for the write already reported the outcome.
		}
	}

	// ----- Issue thread -----
	let threadNodes = $state<ThreadNode[]>([]);
	let threadLoading = $state(false);
	let threadError = $state<string | null>(null);
	// Plain (not $state): only the async loaders compare against it, never
	// the template — a reactive read here would re-arm the thread $effect.
	let threadFor: string | null = null;

	async function loadThread(id: string) {
		threadLoading = true;
		threadError = null;
		try {
			const opts = { eventIds: [id], kinds: [1111], limit: 500, threaded: true };
			const local = await api.getDiscussionList({ ...opts, policy: 'local_only' });
			if (threadFor === id) threadNodes = local.threads ?? [];
			const fresh = await api.getDiscussionList({
				...opts,
				policy: 'fetch_always',
				bypassOffline: true
			});
			if (threadFor === id) threadNodes = fresh.threads ?? [];
		} catch (e) {
			threadError = api.errorMessage(e, 'Failed to load comments');
		} finally {
			threadLoading = false;
		}
	}

	async function refreshThreadLocal() {
		const id = threadFor;
		if (!id) return;
		try {
			const resp = await api.getDiscussionList({
				eventIds: [id],
				kinds: [1111],
				limit: 500,
				threaded: true,
				policy: 'local_only'
			});
			if (threadFor === id) threadNodes = resp.threads ?? [];
		} catch {
			// Non-fatal.
		}
		await reloadLocal();
	}

	$effect(() => {
		const id = view === 'issue' ? issueId : null;
		if (!id) return;
		untrack(() => {
			threadFor = id;
			threadNodes = [];
			void loadThread(id);
		});
	});

	function openIssue(id: string) {
		issueId = id;
		view = 'issue';
		mobileNav.pushViewChange(buffer.id);
	}
	function backToList() {
		view = 'issues';
		mobileNav.pushViewChange(buffer.id);
	}
	function switchView(v: View) {
		if (view === v) return;
		view = v;
		mobileNav.pushViewChange(buffer.id);
	}

	// ----- Status changes -----
	let statusBusy = $state(false);
	async function setStatus(issue: api.GitIssue, status: api.GitIssueStatus) {
		if (statusBusy || !canSign) return;
		statusBusy = true;
		try {
			const resp = await api.setIssueStatus({ issue_id: issue.id, status });
			const { successful, total } = resp.broadcast;
			app.pushToast(
				total === 0
					? `Marked ${status} (saved locally, no relays)`
					: `Marked ${status} (${successful}/${total} relays)`,
				successful === 0 && total > 0 ? 'error' : 'success'
			);
			await reloadLocal();
		} catch (e) {
			app.pushToast(api.errorMessage(e, 'Status change failed'), 'error', 5000);
		} finally {
			statusBusy = false;
		}
	}

	// ----- New issue -----
	let newSubject = $state('');
	let newLabels = $state('');
	let newBody = $state('');
	let filing = $state(false);
	let previewRelays = $state<string[] | null>(null);

	async function fileIssue() {
		const r = repo;
		if (!r || filing || !canSign) return;
		const subject = newSubject.trim();
		const content = newBody.trim();
		if (!subject || !content) {
			app.pushToast('An issue needs a subject and a body', 'error');
			return;
		}
		filing = true;
		try {
			const resp = await api.fileIssue({
				repo: `30617:${r.pubkey}:${r.d_tag}`,
				subject,
				content,
				labels: newLabels.split(',').map((s) => s.trim()).filter(Boolean)
			});
			const { successful, total } = resp.broadcast;
			app.pushToast(
				total === 0
					? 'Issue saved locally (no relays)'
					: successful === 0
						? `Issue saved locally — 0/${total} relays accepted`
						: `Issue filed (${successful}/${total} relays)`,
				successful === 0 && total > 0 ? 'error' : 'success',
				successful === 0 && total > 0 ? 5000 : 2000
			);
			newSubject = '';
			newLabels = '';
			newBody = '';
			previewRelays = null;
			await reloadLocal();
			filter = 'open';
			openIssue(resp.issue.id);
		} catch (e) {
			app.pushToast(api.errorMessage(e, 'Filing failed'), 'error', 5000);
		} finally {
			filing = false;
		}
	}

	async function showRelays() {
		const r = repo;
		if (!r) return;
		try {
			const p = await api.previewIssue({
				repo: `30617:${r.pubkey}:${r.d_tag}`,
				subject: newSubject.trim() || 'preview',
				content: newBody.trim() || 'preview'
			});
			previewRelays = p.relays;
		} catch (e) {
			app.pushToast(api.errorMessage(e, 'Preview failed'), 'error');
		}
	}

	// ----- Keyboard nav (j/k/Enter/m via the global keymap) -----
	function scrollCursorIntoView() {
		listEl?.querySelector<HTMLElement>(`[data-cursor="${cursor}"]`)?.scrollIntoView({ block: 'nearest' });
	}
	async function openCursorMenu() {
		const it = filteredIssues[cursor];
		if (!it) return;
		const res = await api.getEvent(it.id, { policy: 'local_only' });
		if (res.event) app.eventModalData = res.event as NostrEvent;
	}
	function handleNav(action: NavAction): boolean {
		if (view === 'issue') {
			if (action === 'left' || action === 'back') {
				backToList();
				return true;
			}
			return false;
		}
		if (view !== 'issues') return false;
		const total = filteredIssues.length;
		if (total === 0) return false;
		switch (action) {
			case 'down':
				cursor = Math.min(total - 1, cursor + 1);
				queueMicrotask(scrollCursorIntoView);
				return true;
			case 'up':
				cursor = Math.max(0, cursor - 1);
				queueMicrotask(scrollCursorIntoView);
				return true;
			case 'top':
				cursor = 0;
				queueMicrotask(scrollCursorIntoView);
				return true;
			case 'bottom':
				cursor = total - 1;
				queueMicrotask(scrollCursorIntoView);
				return true;
			case 'select':
			case 'right': {
				const it = filteredIssues[cursor];
				if (it) openIssue(it.id);
				return true;
			}
			case 'menu':
				void openCursorMenu();
				return true;
		}
		return false;
	}
	$effect(() => {
		const id = buffer.id;
		const handler = handleNav;
		untrack(() => store.registerNavHandler(id, handler));
		return () => untrack(() => store.unregisterNavHandler(id));
	});

	// ----- Helpers -----
	function fmtDate(ts: number): string {
		return new Date(ts * 1000).toLocaleDateString();
	}
	function ago(ts: number): string {
		const s = Math.max(0, Math.floor(Date.now() / 1000) - ts);
		if (s < 60) return 'just now';
		const m = Math.floor(s / 60);
		if (m < 60) return `${m}m ago`;
		const h = Math.floor(m / 60);
		if (h < 48) return `${h}h ago`;
		const d = Math.floor(h / 24);
		if (d < 60) return `${d}d ago`;
		return fmtDate(ts);
	}
	function host(url: string): string {
		try {
			return new URL(url).host;
		} catch {
			return url;
		}
	}
	async function copy(text: string, what: string) {
		try {
			await navigator.clipboard.writeText(text);
			app.pushToast(`Copied ${what}`, 'success');
		} catch {
			app.pushToast('Copy failed', 'error');
		}
	}
	function shortCommit(c: string): string {
		return c.slice(0, 7);
	}
	const heads = $derived((loaded?.state?.refs ?? []).filter((r) => r.name.startsWith('refs/heads/')));
	const tags = $derived((loaded?.state?.refs ?? []).filter((r) => r.name.startsWith('refs/tags/')));
</script>

<div class="repo-view">
	{#if !coord}
		<div class="empty">Buffer id does not encode a repository coordinate</div>
	{:else if loading && !loaded}
		<div class="empty">Loading repository…</div>
	{:else if error && !loaded}
		<div class="empty">
			<p>{error}</p>
			<button class="btn" onclick={refresh} disabled={fetching}>
				{fetching ? 'Fetching…' : 'Fetch from relays'}
			</button>
		</div>
	{:else if loaded && repo}
		<div class="repo-bar">
			<button class="back-btn" onclick={() => store.killFocused()} title="Close this buffer">←</button>
			<div class="repo-id">
				<div class="repo-name-row">
					<span class="repo-name">{repo.name || repo.d_tag}</span>
					<span class="repo-kind">repository</span>
					{#if repo.fork_of}
						<span class="pill pill--muted" title={repo.fork_of.target}>fork</span>
					{/if}
					{#if repo.seen_on.length === 0}
						<span class="pill pill--muted" title="Announcement is only in the local store">local</span>
					{/if}
				</div>
				<div class="repo-byline">
					<span class="repo-dtag">{repo.d_tag}</span>
					<span class="repo-sep">·</span>
					<ProfileName pubkey={repo.pubkey} />
					{#if repo.maintainers.length}
						<span class="repo-sep">·</span>
						<span>+{repo.maintainers.length} maintainer{repo.maintainers.length === 1 ? '' : 's'}</span>
					{/if}
				</div>
				{#if repo.description}
					<p class="repo-desc">{repo.description}</p>
				{/if}
				{#if repo.hashtags.length}
					<p class="repo-tags">{repo.hashtags.map((t) => `#${t}`).join(' ')}</p>
				{/if}
			</div>
			<div class="repo-actions">
				<button
					class="btn"
					onclick={refresh}
					disabled={fetching}
					title="Pull the announcement, issues, statuses, and comments from the read relays plus the repo's own"
				>{fetching ? '…' : '↻'}</button>
				<button
					class="btn"
					onclick={() => app.openAddressableInModal({ kind: 30617, pubkey: repo.pubkey, d_tag: repo.d_tag })}
					title="Open the announcement in the event menu"
				>menu</button>
			</div>
		</div>

		<div class="tabs">
			<button class="tab" class:active={view === 'issues' || view === 'issue' || view === 'new'} onclick={() => switchView('issues')}>
				Issues ({loaded.open_count} open)
			</button>
			<button class="tab" class:active={view === 'about'} onclick={() => switchView('about')}>About</button>
		</div>

		<div class="content" bind:this={listEl}>
			{#if view === 'about'}
				<div class="about">
					{#if repo.web.length}
						<div class="about-row">
							<span class="about-k">web</span>
							<span class="about-v">
								{#each repo.web as url (url)}
									<a href={url} target="_blank" rel="noopener noreferrer">{host(url)}</a>
								{/each}
							</span>
						</div>
					{/if}
					{#if repo.clone.length}
						<div class="about-row">
							<span class="about-k">clone</span>
							<span class="about-v about-v--col">
								{#each repo.clone as url (url)}
									<span class="clone-row">
										<code class="clone-url">{url}</code>
										<button class="btn btn--xs" onclick={() => copy(url, 'clone URL')}>copy</button>
									</span>
								{/each}
							</span>
						</div>
					{/if}
					<div class="about-row">
						<span class="about-k">maintainers</span>
						<span class="about-v about-v--col">
							<span><ProfileName pubkey={repo.pubkey} /> <span class="muted">(owner)</span></span>
							{#each repo.maintainers as m (m)}
								<span><ProfileName pubkey={m} /></span>
							{/each}
						</span>
					</div>
					{#if repo.relays.length}
						<div class="about-row">
							<span class="about-k">relays</span>
							<span class="about-v about-v--col">
								{#each repo.relays as r (r)}
									<code class="clone-url">{r}</code>
								{/each}
							</span>
						</div>
					{/if}
					{#if repo.euc}
						<div class="about-row">
							<span class="about-k">euc</span>
							<span class="about-v"><code class="clone-url" title="Earliest unique commit — groups forks and mirrors of one project">{repo.euc}</code></span>
						</div>
					{/if}
					{#if repo.fork_of}
						<div class="about-row">
							<span class="about-k">fork of</span>
							<span class="about-v"><code class="clone-url">{repo.fork_of.target}</code></span>
						</div>
					{/if}
					{#if loaded.state}
						<div class="about-row">
							<span class="about-k">state</span>
							<span class="about-v about-v--col">
								{#if loaded.state.head}
									<span>HEAD → <code class="clone-url">{loaded.state.head}</code></span>
								{/if}
								{#each heads as r (r.name)}
									<span><code class="clone-url">{r.name.slice('refs/heads/'.length)}</code> <span class="muted">{shortCommit(r.commit)}</span></span>
								{/each}
								{#if tags.length}
									<span class="muted">{tags.length} tag{tags.length === 1 ? '' : 's'}: {tags.map((t) => t.name.slice('refs/tags/'.length)).join(', ')}</span>
								{/if}
								<span class="muted">announced {ago(loaded.state.created_at)} by <ProfileName pubkey={loaded.state.pubkey} /></span>
							</span>
						</div>
					{/if}
					<div class="about-row">
						<span class="about-k">announced</span>
						<span class="about-v">{fmtDate(repo.created_at)}{repo.seen_on.length ? ` · seen on ${repo.seen_on.map(host).join(', ')}` : ' · local only'}</span>
					</div>
				</div>
			{:else if view === 'new'}
				<div class="new-issue">
					<div class="new-head">
						<button class="btn btn--xs" onclick={backToList}>← issues</button>
						<span class="new-title">New issue on {repo.name || repo.d_tag}</span>
					</div>
					{#if !canSign}
						<div class="empty">Sign in (settings) to file an issue.</div>
					{:else}
						<input class="field" placeholder="Subject" bind:value={newSubject} data-entry disabled={filing} />
						<input class="field" placeholder="Labels, comma-separated (optional)" bind:value={newLabels} data-entry disabled={filing} />
						<textarea class="field field--body" placeholder="Describe the bug, request, or question (markdown)" bind:value={newBody} data-entry rows="10" disabled={filing}></textarea>
						<div class="new-foot">
							<button class="btn btn--primary" onclick={fileIssue} disabled={filing || !newSubject.trim() || !newBody.trim()}>
								{filing ? 'Filing…' : 'File issue'}
							</button>
							<button class="btn" onclick={showRelays} title="Which relays this would be published to (the publish set plus the repo's own)">relays?</button>
							{#if previewRelays}
								<span class="muted">→ {previewRelays.length ? previewRelays.map(host).join(', ') : 'no relays (saved locally)'}</span>
							{/if}
						</div>
					{/if}
				</div>
			{:else if view === 'issue'}
				{#if !currentIssue}
					<div class="empty">
						<p>Issue not found in this repository.</p>
						<button class="btn" onclick={backToList}>← issues</button>
					</div>
				{:else}
					{@const issue = currentIssue}
					{@const canChange = canSign && !!me && (issue.pubkey === me || isMaintainer)}
					<div class="issue">
						<div class="issue-head">
							<button class="btn btn--xs" onclick={backToList}>← issues</button>
							<span class="pill pill--{issue.status}">{issue.status}</span>
							{#each issue.labels as l (l)}
								<span class="pill pill--label">{l}</span>
							{/each}
						</div>
						<h2 class="issue-subject">{issue.subject ?? '[no subject]'}</h2>
						<div class="issue-meta">
							<ProfileName pubkey={issue.pubkey} />
							<span class="repo-sep">·</span>
							<span title={new Date(issue.created_at * 1000).toLocaleString()}>{ago(issue.created_at)}</span>
							{#if issue.status_event}
								<span class="repo-sep">·</span>
								<span>{issue.status_event.status} {ago(issue.status_event.created_at)} by <ProfileName pubkey={issue.status_event.pubkey} /></span>
							{/if}
							<button
								class="btn btn--xs"
								onclick={async () => { const res = await api.getEvent(issue.id, { policy: 'local_only' }); if (res.event) app.eventModalData = res.event as NostrEvent; }}
							>menu</button>
						</div>
						{#if canChange}
							<div class="issue-actions">
								{#if issue.status === 'open' || issue.status === 'draft'}
									{#if issue.status === 'draft'}
										<button class="btn btn--xs" disabled={statusBusy} onclick={() => setStatus(issue, 'open')}>mark open</button>
									{:else}
										<button class="btn btn--xs" disabled={statusBusy} onclick={() => setStatus(issue, 'draft')}>mark draft</button>
									{/if}
									<button class="btn btn--xs" disabled={statusBusy} onclick={() => setStatus(issue, 'resolved')}>resolve</button>
									<button class="btn btn--xs" disabled={statusBusy} onclick={() => setStatus(issue, 'closed')}>close</button>
								{:else}
									<button class="btn btn--xs" disabled={statusBusy} onclick={() => setStatus(issue, 'open')}>reopen</button>
								{/if}
							</div>
						{/if}
						{#if issue.status_event?.content}
							<blockquote class="status-note">{issue.status_event.content}</blockquote>
						{/if}
						<div class="issue-body">
							<RichContent content={issue.content} />
						</div>

						<div class="thread">
							<div class="thread-head">
								<span>{threadNodes.length ? 'Comments' : 'No comments yet'}</span>
								{#if threadLoading}<span class="muted">loading…</span>{/if}
							</div>
							{#if threadError}
								<div class="thread-error">{threadError}</div>
							{/if}
							{#if threadNodes.length}
								<CommentThread nodes={threadNodes} replyable onposted={refreshThreadLocal} />
							{/if}
							<ReplyBox
								root={{ event_id: issue.id, kind: 1621, pubkey: issue.pubkey }}
								placeholder="Comment on this issue…"
								onposted={refreshThreadLocal}
							/>
						</div>
					</div>
				{/if}
			{:else}
				<div class="list-head">
					<div class="filters">
						<button class="chip" class:active={filter === 'open'} onclick={() => { filter = 'open'; cursor = 0; }}>open ({loaded.open_count})</button>
						<button class="chip" class:active={filter === 'closed'} onclick={() => { filter = 'closed'; cursor = 0; }}>closed ({closedCount})</button>
						<button class="chip" class:active={filter === 'all'} onclick={() => { filter = 'all'; cursor = 0; }}>all ({loaded.issues.length})</button>
					</div>
					<button
						class="btn btn--primary btn--xs"
						onclick={() => switchView('new')}
						disabled={!canSign}
						title={canSign ? 'File a kind-1621 issue against this repository' : 'Sign in to file an issue'}
					>+ New issue</button>
				</div>
				{#if filteredIssues.length === 0}
					<div class="empty">
						{loaded.issues.length === 0 ? 'No issues in the store — ↻ pulls them from the relays.' : `No ${filter} issues`}
					</div>
				{:else}
					{#each filteredIssues as issue, i (issue.id)}
						<!-- svelte-ignore a11y_no_static_element_interactions -->
						<div
							class="item"
							class:item--cursor={i === cursor}
							data-cursor={i}
							onclick={() => { cursor = i; openIssue(issue.id); }}
							onkeydown={(e) => { if (e.key === 'Enter') openIssue(issue.id); }}
							onfocus={() => (cursor = i)}
							role="button"
							tabindex="0"
						>
							<div class="item-main">
								<span class="item-title">
									<span class="pill pill--{issue.status}">{issue.status}</span>
									{issue.subject ?? '[no subject]'}
								</span>
								<p class="item-preview">{issue.content.slice(0, 200)}</p>
								<span class="item-meta">
									<ProfileName pubkey={issue.pubkey} />
									<span class="repo-sep">·</span>
									{ago(issue.last_activity)}
									{#each issue.labels as l (l)}
										<span class="pill pill--label">{l}</span>
									{/each}
								</span>
							</div>
							<div class="item-rail">
								<span class="item-count" title="comments">{issue.comment_count} 💬</span>
							</div>
						</div>
					{/each}
				{/if}
			{/if}
		</div>
	{/if}
</div>

<style>
	.repo-view {
		flex: 1;
		display: flex;
		flex-direction: column;
		min-height: 0;
		height: 100%;
		overflow: hidden;
	}
	.empty {
		padding: 24px;
		text-align: center;
		color: var(--fg-muted);
		font-size: var(--t-xs);
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 8px;
	}
	.muted { color: var(--fg-muted); }

	.repo-bar {
		display: flex;
		align-items: flex-start;
		gap: 10px;
		padding: 10px 16px;
		border-bottom: 1px solid var(--border);
	}
	.back-btn {
		background: none;
		border: none;
		color: var(--fg-muted);
		font-size: var(--t-md);
		cursor: pointer;
		padding: 2px 6px;
	}
	.back-btn:hover { color: var(--fg); }
	.repo-id { flex: 1; min-width: 0; }
	.repo-name-row { display: flex; align-items: baseline; gap: 8px; flex-wrap: wrap; }
	.repo-name { font-size: var(--t-md); font-weight: 700; }
	.repo-kind { font-size: var(--t-3xs); color: var(--fg-muted); text-transform: uppercase; letter-spacing: 0.05em; }
	.repo-byline { font-size: var(--t-2xs); color: var(--fg-muted); display: flex; gap: 6px; flex-wrap: wrap; align-items: center; margin-top: 2px; }
	.repo-dtag { font-family: var(--font-mono); }
	.repo-sep { opacity: 0.5; }
	.repo-desc { font-size: var(--t-xs); margin: 6px 0 0; line-height: 1.4; overflow-wrap: anywhere; }
	.repo-tags { font-size: var(--t-3xs); color: var(--accent); margin: 4px 0 0; }
	.repo-actions { display: flex; gap: 4px; flex-shrink: 0; }

	.btn {
		background: none;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		color: var(--fg-muted);
		font-size: var(--t-2xs);
		padding: 3px 8px;
		cursor: pointer;
		line-height: 1.5;
	}
	.btn:hover:not(:disabled) { color: var(--accent); border-color: var(--accent); }
	.btn:disabled { opacity: 0.4; cursor: not-allowed; }
	.btn--xs { font-size: var(--t-3xs); padding: 1px 6px; font-family: var(--font-mono); }
	.btn--primary { color: var(--fg); border-color: var(--accent); }

	.tabs { display: flex; border-bottom: 1px solid var(--border); }
	.tab {
		flex: 1;
		padding: 8px 12px;
		font-size: var(--t-2xs);
		background: none;
		border: none;
		border-bottom: 2px solid transparent;
		color: var(--fg-muted);
		cursor: pointer;
	}
	.tab:hover { color: var(--fg); }
	.tab.active { color: var(--fg); border-bottom-color: var(--accent); }

	.content { flex: 1; overflow-y: auto; min-height: 0; }

	.list-head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
		padding: 6px 12px;
		border-bottom: 1px solid var(--border);
	}
	.filters { display: flex; gap: 4px; }
	.chip {
		background: none;
		border: 1px solid var(--border);
		border-radius: 999px;
		color: var(--fg-muted);
		font-size: var(--t-3xs);
		padding: 1px 8px;
		cursor: pointer;
	}
	.chip.active { color: var(--fg); border-color: var(--accent); background: color-mix(in srgb, var(--accent) 12%, transparent); }

	.item {
		padding: 10px 16px;
		border-bottom: 1px solid var(--border);
		border-left: 3px solid var(--selection);
		display: flex;
		align-items: flex-start;
		gap: 8px;
		cursor: pointer;
	}
	.item:hover { background: var(--bg-surface); }
	.item--cursor {
		background: color-mix(in srgb, var(--id-yours) 18%, transparent);
		border-left-color: var(--id-yours);
		border-left-width: 5px;
		padding-left: 14px;
	}
	.item--cursor .item-title { color: var(--fg); font-weight: 700; }
	.item-main { flex: 1; min-width: 0; }
	.item-rail { flex-shrink: 0; display: flex; flex-direction: column; align-items: flex-end; }
	.item-title {
		display: block;
		font-size: var(--t-sm);
		font-weight: 600;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		margin-bottom: 2px;
	}
	.item-preview {
		font-size: var(--t-2xs);
		color: var(--fg-muted);
		line-height: 1.4;
		margin: 2px 0;
		overflow: hidden;
		display: -webkit-box;
		-webkit-line-clamp: 2;
		line-clamp: 2;
		-webkit-box-orient: vertical;
		overflow-wrap: anywhere;
	}
	.item-meta { font-size: var(--t-3xs); color: var(--fg-muted); display: flex; gap: 6px; align-items: center; flex-wrap: wrap; }
	.item-count { font-size: var(--t-3xs); color: var(--fg-muted); white-space: nowrap; }

	.pill {
		display: inline-block;
		font-size: var(--t-3xs);
		font-family: var(--font-mono);
		padding: 0 6px;
		border-radius: 999px;
		border: 1px solid var(--border);
		color: var(--fg-muted);
		vertical-align: middle;
		line-height: 1.6;
	}
	.pill--open { color: var(--state-online); border-color: var(--state-online); }
	.pill--draft { color: var(--fg-muted); border-style: dashed; }
	.pill--resolved { color: var(--accent); border-color: var(--accent); }
	.pill--closed { color: var(--fg-muted); background: var(--bg-surface); }
	.pill--label { color: var(--accent); }
	.pill--muted { color: var(--fg-muted); }

	.about { padding: 12px 16px; display: flex; flex-direction: column; gap: 10px; font-size: var(--t-xs); }
	.about-row { display: grid; grid-template-columns: 12ch 1fr; gap: 8px; align-items: start; }
	.about-k { color: var(--fg-muted); font-size: var(--t-3xs); text-transform: uppercase; letter-spacing: 0.05em; padding-top: 2px; }
	.about-v { min-width: 0; display: flex; gap: 8px; flex-wrap: wrap; overflow-wrap: anywhere; }
	.about-v--col { flex-direction: column; gap: 4px; }
	.about-v a { color: var(--accent); }
	.clone-row { display: flex; gap: 6px; align-items: center; min-width: 0; }
	.clone-url { font-family: var(--font-mono); font-size: var(--t-3xs); background: var(--bg-surface); padding: 1px 4px; border-radius: var(--r-sm); overflow-wrap: anywhere; }

	.new-issue { padding: 12px 16px; display: flex; flex-direction: column; gap: 8px; }
	.new-head { display: flex; align-items: center; gap: 8px; }
	.new-title { font-size: var(--t-sm); font-weight: 600; }
	.field {
		width: 100%;
		box-sizing: border-box;
		background: var(--bg-surface);
		border: 1px solid var(--border);
		border-radius: var(--radius);
		color: var(--fg);
		font-size: var(--t-xs);
		padding: 6px 8px;
		font-family: inherit;
	}
	.field:focus { outline: none; border-color: var(--accent); }
	.field--body { resize: vertical; min-height: 8em; line-height: 1.4; }
	.new-foot { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; font-size: var(--t-3xs); }

	.issue { padding: 12px 16px; display: flex; flex-direction: column; gap: 8px; }
	.issue-head { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
	.issue-subject { font-size: var(--t-md); margin: 0; overflow-wrap: anywhere; }
	.issue-meta { font-size: var(--t-2xs); color: var(--fg-muted); display: flex; gap: 6px; align-items: center; flex-wrap: wrap; }
	.issue-actions { display: flex; gap: 4px; flex-wrap: wrap; }
	.status-note { margin: 0; padding: 4px 10px; border-left: 3px solid var(--border); color: var(--fg-muted); font-size: var(--t-xs); white-space: pre-wrap; }
	.issue-body { font-size: var(--t-sm); line-height: 1.55; padding: 8px 0; border-bottom: 1px solid var(--border); }
	.thread { display: flex; flex-direction: column; gap: 8px; }
	.thread-head { display: flex; gap: 8px; align-items: baseline; font-size: var(--t-2xs); font-weight: 600; }
	.thread-error { color: var(--state-error, #c44); font-size: var(--t-2xs); }
</style>
