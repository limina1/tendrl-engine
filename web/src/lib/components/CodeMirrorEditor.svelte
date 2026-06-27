<script lang="ts">
	import { untrack } from 'svelte';
	import { EditorState, Compartment, type Extension } from '@codemirror/state';
	import { EditorView, keymap, lineNumbers as lineNumbersExt } from '@codemirror/view';
	import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
	import { vim, getCM } from '@replit/codemirror-vim';

	let {
		value = $bindable<string>(''),
		vimMode = false,
		lineNumbers = false,
		onLeave,
		onBlur,
		extensions = [],
		editorView = $bindable<EditorView | null>(null)
	}: {
		value?: string;
		vimMode?: boolean;
		lineNumbers?: boolean;
		onLeave?: () => void;
		onBlur?: () => void;
		extensions?: Extension[];
		editorView?: EditorView | null;
	} = $props();

	// Compartments let us reconfigure line-numbers and vim mode without
	// tearing down the editor (which would lose cursor / undo history).
	const lineNumbersCompartment = new Compartment();
	const vimCompartment = new Compartment();

	let view: EditorView | null = $state(null);

	// Svelte action: attaches CM6 once when the element mounts, cleans up
	// when it unmounts. Lives entirely outside Svelte's effect tracking, so
	// no reactive dep can spuriously rebuild the editor and steal focus.
	function attach(el: HTMLDivElement) {
		const sizeTheme = EditorView.theme({
			'&': { height: '100%' },
			'.cm-scroller': { overflow: 'auto' }
		});

		// Snapshot once. Subsequent changes flow through the sync $effect.
		let initialDoc = '';
		untrack(() => {
			initialDoc = value ?? '';
		});

		// Snapshot toggles once for initial config; later changes flow through
		// the sync $effects via their compartments.
		let initialLineNumbers = false;
		let initialVimMode = false;
		untrack(() => {
			initialLineNumbers = !!lineNumbers;
			initialVimMode = !!vimMode;
		});

		const baseExt: Extension[] = [
			vimCompartment.of(initialVimMode ? vim() : []),
			lineNumbersCompartment.of(initialLineNumbers ? lineNumbersExt() : []),
			history(),
			keymap.of([...defaultKeymap, ...historyKeymap]),
			EditorView.lineWrapping,
			sizeTheme,
			EditorView.updateListener.of((u) => {
				if (u.docChanged) {
					value = u.state.doc.toString();
				}
			}),
			EditorView.domEventHandlers({
				keydown: (e, v) => {
					if (e.key !== 'Escape') return false;
					const cm = getCM(v) as { state?: { vim?: { insertMode?: boolean } } } | null;
					if (cm?.state?.vim && cm.state.vim.insertMode === false) {
						e.preventDefault();
						v.contentDOM.blur();
						onLeave?.();
						return true;
					}
					return false;
				},
				blur: () => {
					onBlur?.();
					return false;
				}
			}),
			...extensions
		];

		const v = new EditorView({
			state: EditorState.create({ doc: initialDoc, extensions: baseExt }),
			parent: el
		});
		view = v;
		editorView = v;

		return {
			destroy() {
				v.destroy();
				view = null;
				editorView = null;
			}
		};
	}

	// External value → editor sync. Direct doc compare avoids the round-trip
	// from our own updateListener triggering a redundant dispatch (which
	// would reset the cursor mid-edit).
	$effect(() => {
		const v = value ?? '';
		untrack(() => {
			if (!view) return;
			const cur = view.state.doc.toString();
			if (cur === v) return;
			view.dispatch({ changes: { from: 0, to: cur.length, insert: v } });
		});
	});

	// Line-number toggle: reconfigure the compartment without remounting.
	$effect(() => {
		const want = !!lineNumbers;
		untrack(() => {
			if (!view) return;
			view.dispatch({
				effects: lineNumbersCompartment.reconfigure(want ? lineNumbersExt() : [])
			});
		});
	});

	// Vim-mode toggle. Reconfigure the vim compartment live; CM rebuilds
	// the keymap and the user keeps their cursor + undo stack.
	$effect(() => {
		const want = !!vimMode;
		untrack(() => {
			if (!view) return;
			view.dispatch({
				effects: vimCompartment.reconfigure(want ? vim() : [])
			});
		});
	});
</script>

<div class="cm-host" use:attach></div>

<style>
	.cm-host {
		flex: 1;
		min-height: 0;
		overflow: hidden;
		display: flex;
	}
	.cm-host :global(.cm-editor) {
		flex: 1;
		min-width: 0;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		background: var(--bg-surface, var(--bg));
		color: var(--fg);
	}
	.cm-host :global(.cm-scroller) {
		font-family: inherit;
		line-height: 1.5;
	}
	.cm-host :global(.cm-content) {
		padding: 12px;
		caret-color: var(--id-yours);
	}
	.cm-host :global(.cm-focused) {
		outline: none;
	}
	.cm-host :global(.cm-cursor) {
		border-left-color: var(--id-yours);
	}
	.cm-host :global(.cm-fat-cursor) {
		background: color-mix(in srgb, var(--id-yours) 50%, transparent);
		color: var(--fg);
	}
	.cm-host :global(.cm-gutters) {
		background: var(--panel-bg-soft);
		border-right-color: var(--panel-border);
		color: var(--base5);
	}
</style>
