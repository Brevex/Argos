import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// The shell is served to a local web view, never to a network. `clearScreen`
// off keeps Tauri's own output visible when it runs this as a subprocess.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: { target: 'es2022', sourcemap: true },
});
