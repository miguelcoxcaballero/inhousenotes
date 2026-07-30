import { defineConfig } from 'vite';
import { viteSingleFile } from 'vite-plugin-singlefile';
import { rename } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

// La entrada de la app nueva es app.html (la raíz aún sirve el index.html
// legacy durante la migración). El build produce un único dist/index.html,
// que es el artefacto que consume `android app/html_to_apk_builder.py`.
const rootDir = fileURLToPath(new URL('.', import.meta.url));

export default defineConfig({
  build: {
    rollupOptions: { input: resolve(rootDir, 'app.html') },
    target: 'es2022'
  },
  plugins: [
    viteSingleFile(),
    {
      name: 'rename-entry-to-index',
      async closeBundle() {
        await rename(
          resolve(rootDir, 'dist/app.html'),
          resolve(rootDir, 'dist/index.html')
        );
      }
    }
  ],
  test: {
    environment: 'node',
    include: ['src/**/*.test.ts', 'test/**/*.test.ts']
  }
} as never);
