/// <reference types="vitest/config" />
import tailwindcss from '@tailwindcss/vite';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

// The dev server proxies the API to a running Observer; set OBSERVER_URL to point elsewhere.
const observer = process.env.OBSERVER_URL ?? 'http://127.0.0.1:40125';

export default defineConfig({
    plugins: [react(), tailwindcss()],
    server: {
        port: 41820,
        strictPort: true,
        proxy: {
            '/api': { target: observer, ws: true },
        },
        // The documentation's tests read the contract as the binary embeds it, from `resource/`.
        fs: { allow: ['.', '../resource'] },
    },
    build: {
        outDir: 'dist',
        emptyOutDir: true,
        target: 'es2023',
        // Inlined assets become data: URIs, which the content security policy refuses for fonts.
        assetsInlineLimit: 0,
    },
    test: {
        environment: 'jsdom',
        include: ['src/**/*.test.{ts,tsx}'],
    },
});
