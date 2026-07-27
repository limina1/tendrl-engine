export type SlotState = 'open' | 'rail' | 'hidden';
export type ClassName = 'chat' | 'work' | 'research';
export type Position = 'left' | 'center' | 'right';

export type Buffer = {
	id: string;
	kind: string;
	label: string;
	kicker?: string;
	modified?: boolean;
};

export type SplitNode =
	| { type: 'leaf'; buffer: Buffer }
	| { type: 'split'; orient: 'h' | 'v'; children: SplitNode[] };

export type Slot = {
	className: ClassName;
	state: SlotState;
	tree: SplitNode;
};

export type LayoutConfig = {
	name: string;
	desc: string;
	slots: Partial<Record<Position, Slot>>;
};

export type OpenBuf = { className: ClassName; buffer: Buffer };

export type MinibufferMode = 'closed' | 'class' | 'global' | 'recent' | 'mx' | 'split';

export type CommandCat =
	| 'Buffer'
	| 'Window'
	| 'Layout'
	| 'Navigation'
	| 'Compose'
	| 'Application'
	| 'Configuration'
	| 'Versioning'
	| 'View';

/** What invoking a command actually gets the user — the axis the settings
 *  registry groups by. 'action' acts immediately from anywhere; 'opener'
 *  takes you to the surface where the real action happens; 'contextual' is
 *  a real action but only meaningful in its `context` (inert elsewhere);
 *  'nav' is window/buffer plumbing. */
export type CommandScope = 'action' | 'opener' | 'contextual' | 'nav';

export type Command = {
	id: string;
	name: string;
	description: string;
	category: CommandCat;
	scope: CommandScope;
	/** For contextual commands: the situation the command needs to bite. */
	context?: string;
	/** Listed for discoverability but not executable yet — the palette
	 *  toasts instead of silently closing, and registries badge it. */
	deferred?: boolean;
	/** Ships palette-hidden: stays in the Settings registry (re-checkable)
	 *  and any binding keeps working, it just doesn't clutter SPC : until
	 *  the user opts it in. User prefs override in either direction. */
	hiddenByDefault?: boolean;
	keybinding?: string;
};
