// Available UI themes. Each theme is a palette defined in
// lib/styles/tokens.css: the dark theme lives in `:root` (the default), and
// every other theme in a `:root[data-theme='<id>']` block. Adding a theme is
// two steps — one tokens.css block + one entry here.
//
// Each theme belongs to a `family` (the named palette, e.g. "Iceberg") and has
// a `mode` (its light/dark variant). The Settings dropdown groups by family;
// `mode` drives the sun/moon quick toggle, which flips the variant within the
// current family. Adding a theme is two steps — one tokens.css block + one
// entry here.
export type ThemeMode = 'dark' | 'light';

export interface ThemeDef {
	id: string;
	family: string;
	familyLabel: string;
	mode: ThemeMode;
}

export const THEMES: ThemeDef[] = [
	{ id: 'iceberg-dark', family: 'iceberg', familyLabel: 'Iceberg', mode: 'dark' },
	{ id: 'iceberg-light', family: 'iceberg', familyLabel: 'Iceberg', mode: 'light' },
	{ id: 'solarized-dark', family: 'solarized', familyLabel: 'Solarized', mode: 'dark' },
	{ id: 'solarized-light', family: 'solarized', familyLabel: 'Solarized', mode: 'light' },
	{ id: 'gruvbox-dark', family: 'gruvbox', familyLabel: 'Gruvbox', mode: 'dark' },
	{ id: 'gruvbox-light', family: 'gruvbox', familyLabel: 'Gruvbox', mode: 'light' }
];

// Themes grouped by family, in declaration order — for the Settings dropdown's
// <optgroup>s. Each family lists its variants (dark/light).
export interface ThemeFamily {
	family: string;
	label: string;
	variants: ThemeDef[];
}

export function themeFamilies(): ThemeFamily[] {
	const out: ThemeFamily[] = [];
	for (const t of THEMES) {
		let fam = out.find((f) => f.family === t.family);
		if (!fam) {
			fam = { family: t.family, label: t.familyLabel, variants: [] };
			out.push(fam);
		}
		fam.variants.push(t);
	}
	return out;
}

// The variant of `family` matching `mode`, falling back to the family's first
// variant (then any theme of that mode) — used by the sun/moon toggle to stay
// in-family when flipping light↔dark.
export function variantFor(family: string, mode: ThemeMode): ThemeDef | undefined {
	return (
		THEMES.find((t) => t.family === family && t.mode === mode) ??
		THEMES.find((t) => t.family === family) ??
		THEMES.find((t) => t.mode === mode)
	);
}

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

// --- High contrast ---------------------------------------------------------
// An accessibility modifier orthogonal to the theme: `data-contrast="high"`
// on <html> lifts text/border tokens toward WCAG AAA over whatever theme is
// active (see the [data-contrast='high'] blocks in tokens.css). Stored
// separately from the theme so it composes with every palette.
export const CONTRAST_STORAGE_KEY = 'tendrl.contrast';

// The OS-level "increase contrast" setting (Windows/macOS/GNOME), exposed to
// the web as the prefers-contrast media query. Seeds the default when the
// user hasn't made an explicit choice.
export function prefersMoreContrast(): boolean {
	return (
		typeof window !== 'undefined' &&
		typeof window.matchMedia === 'function' &&
		window.matchMedia('(prefers-contrast: more)').matches
	);
}

export function applyContrastAttribute(high: boolean): void {
	if (typeof document === 'undefined') return;
	const root = document.documentElement;
	if (high) root.setAttribute('data-contrast', 'high');
	else root.removeAttribute('data-contrast');
}
