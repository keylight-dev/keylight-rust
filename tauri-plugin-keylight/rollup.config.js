import { readFileSync } from 'node:fs';
import typescript from '@rollup/plugin-typescript';

const pkg = JSON.parse(readFileSync(new URL('./package.json', import.meta.url), 'utf8'));

export default {
  input: 'guest-js/index.ts',
  output: [
    { file: pkg.exports['.'].import, format: 'esm' },
    { file: pkg.exports['.'].require, format: 'cjs' },
  ],
  plugins: [
    typescript({
      declaration: true,
      declarationDir: './dist-js',
      rootDir: 'guest-js',
    }),
  ],
  external: [/^@tauri-apps\/api/],
};
