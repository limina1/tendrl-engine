/// <reference types="vitest/config" />
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

const proxy = {
	'/api': {
		target: 'http://localhost:3030',
		changeOrigin: true
	},
	'/health': {
		target: 'http://localhost:3030',
		changeOrigin: true
	}
};

export default defineConfig({
	plugins: [sveltekit()],
	server: { proxy },
	preview: { proxy },
	// Pure-TS unit tests (rendering-layer tokenizers, segment invariants):
	// `pnpm test`. No DOM — anything touching Svelte runtime stays out.
	test: { include: ['src/**/*.test.ts'], environment: 'node' }
});
