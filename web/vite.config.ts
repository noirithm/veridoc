import { defineConfig } from 'vite';

export default defineConfig({
  build: { target: 'es2022' },
  // Workers are built as ES modules so they can load the WebAssembly glue code.
  worker: { format: 'es' },
});