// Available UI themes. Each theme is a palette defined in
// lib/styles/tokens.css: the dark theme lives in `:root` (the default), and
// every other theme in a `:root[data-theme='<id>']` block. Adding a theme is
// two steps — one tokens.css block + one entry here.
//
// `mode` drives the sun/moon quick toggle (it flips to the first theme of the
// opposite mode) and lets Settings group themes if we ever want to.
export type ThemeMode = 'dark' | 'light';

export interface ThemeDef {
	id: string;
	label: string;
	mode: ThemeMode;
}

export const THEMES: ThemeDef[] = [
	{ id: 'iceberg-dark', label: 'Iceberg Dark', mode: 'dark' },
	{ id: 'iceberg-light', label: 'Iceberg Light', mode: 'light' }
];

// The default theme uses no `data-theme` attribute, so it matches both the
// :root token defaults and the pre-paint state (no attribute set). Keep this
// in sync with the inline bootstrap script in app.html.
export const DEFAULT_THEME = 'iceberg-dark';
export const THEME_STORAGE_KEY = 'tendrl.theme';

export function themeById(id: string): ThemeDef | undefined {
	return THEMES.find((t) => t.id === id);
}

export function isValidTheme(id: string | null | undefined): id is string {
	return !!id && THEMES.some((t) => t.id === id);
}

// Toggle the `<html data-theme>` attribute to activate a theme. The default
// (dark) theme clears the attribute so it falls back to the :root defaults;
// every other theme sets it to the theme id.
export function applyThemeAttribute(id: string): void {
	if (typeof document === 'undefined') return;
	const root = document.documentElement;
	if (id === DEFAULT_THEME) root.removeAttribute('data-theme');
	else root.setAttribute('data-theme', id);
}
