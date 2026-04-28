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

export type Command = {
	id: string;
	name: string;
	description: string;
	category: CommandCat;
	keybinding?: string;
};
