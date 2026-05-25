import { sveltekit } from '@sveltejs/kit/vite';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vite';

export default defineConfig({
	plugins: [sveltekit(), tailwindcss()],
	server: {
		host: true,
		port: 5167,
		proxy: {
			'/api': {
				target: 'http://localhost:3067',
				timeout: 0
			}
		}
	}
});
