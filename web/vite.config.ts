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
	preview: { proxy }
});
