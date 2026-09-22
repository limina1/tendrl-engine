// Leaf helpers for the paid-service (ppq.ai / routstr) surfaces: the drawer
// rows, the Agent services modal, the Settings mirror. No store imports, so
// WM components can use it without registry cycles.

import type { ProviderKind, ProvidersState, ServiceBalance, ServiceStatus } from './api';

export const SERVICE_LABEL: Record<ProviderKind, string> = { ppq: 'ppq.ai', routstr: 'routstr' };

export function fmtUsd(n: number): string {
	if (n >= 100) return `$${n.toFixed(0)}`;
	if (n >= 1) return `$${n.toFixed(2)}`;
	return `$${n.toFixed(3).replace(/0$/, '')}`;
}

export function fmtSats(sats: number): string {
	return `${Math.round(sats).toLocaleString()} sat`;
}

/** Balance as the user reads it: ppq in USD, routstr in sats (from msats). */
export function fmtBalance(b: ServiceBalance | null | undefined): string {
	if (!b) return '—';
	return b.unit === 'usd' ? fmtUsd(b.amount) : fmtSats(b.amount / 1000);
}

/** Pill class per service status (tokens.css pill family). */
export function pillFor(status: ServiceStatus): string {
	switch (status) {
		case 'funded':
			return 'pill--online';
		case 'low':
			return 'pill--draft';
		case 'offline':
			return 'pill--offline';
		default:
			return 'pill--ghost';
	}
}

export function pillLabel(status: ServiceStatus, b: ServiceBalance | null | undefined): string {
	switch (status) {
		case 'funded':
			return fmtBalance(b);
		case 'low':
			return `low · ${fmtBalance(b)}`;
		case 'offline':
			return 'offline';
		default:
			return 'unfunded';
	}
}

export interface ServiceRow {
	kind: ProviderKind;
	label: string;
	active: boolean;
	status: ServiceStatus;
	balance: ServiceBalance | null;
	pillClass: string;
	pillLabel: string;
	model: string | null;
	detail: string;
}

/** One row per configured service, ppq first. */
export function serviceRows(s: ProvidersState | null | undefined): ServiceRow[] {
	if (!s) return [];
	const rows: ServiceRow[] = [];
	if (s.ppq) {
		rows.push({
			kind: 'ppq',
			label: SERVICE_LABEL.ppq,
			active: s.active === 'ppq',
			status: s.ppq.status,
			balance: s.ppq.balance,
			pillClass: pillFor(s.ppq.status),
			pillLabel: pillLabel(s.ppq.status, s.ppq.balance),
			model: s.ppq.model,
			detail: s.ppq.model ?? 'no model'
		});
	}
	if (s.routstr) {
		rows.push({
			kind: 'routstr',
			label: SERVICE_LABEL.routstr,
			active: s.active === 'routstr',
			status: s.routstr.status,
			balance: s.routstr.balance,
			pillClass: pillFor(s.routstr.status),
			pillLabel: pillLabel(s.routstr.status, s.routstr.balance),
			model: s.routstr.model,
			detail: [s.routstr.node.name ?? s.routstr.node.url, s.routstr.model ?? 'no model'].join(' · ')
		});
	}
	return rows;
}

export function activeRow(s: ProvidersState | null | undefined): ServiceRow | null {
	return serviceRows(s).find((r) => r.active) ?? null;
}

/** Is the service's balance above its per-turn ceiling? */
export function fundedForTurn(s: ProvidersState, kind: ProviderKind): boolean {
	const svc = kind === 'ppq' ? s.ppq : s.routstr;
	if (!svc?.balance) return false;
	return kind === 'ppq'
		? svc.balance.amount >= s.ceiling_usd
		: svc.balance.amount / 1000 >= s.ceiling_sats;
}

export function shortNpub(npub?: string | null): string {
	return npub ? `${npub.slice(0, 10)}…${npub.slice(-4)}` : '';
}

export function mintName(url: string): string {
	try {
		const u = new URL(url);
		const host = u.hostname.replace(/^mint\./, '');
		const path = u.pathname.replace(/\/$/, '');
		return path && path !== '/' ? `${host}${path}` : host;
	} catch {
		return url;
	}
}
