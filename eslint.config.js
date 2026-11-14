import js from '@eslint/js';
import ts from 'typescript-eslint';
import vue from 'eslint-plugin-vue';
import globals from 'globals';

export default [
  { ignores: ['dist/**', 'node_modules/**', 'src-tauri/**', '.tools/**', '.test-data/**'] },
  js.configs.recommended,
  ...ts.configs.recommended,
  ...vue.configs['flat/essential'],
  {
    files: ['src/**/*.{ts,vue}', 'vite.config.ts'],
    languageOptions: { globals: globals.browser, parserOptions: { parser: ts.parser } },
    rules: {
      complexity: ['warn', 10],
      'max-depth': ['warn', 3],
      'max-params': ['warn', 4],
      'max-lines-per-function': ['warn', { max: 50, skipBlankLines: true, skipComments: true }],
      'no-else-return': 'error',
      'no-nested-ternary': 'error',
      'no-var': 'error',
      'prefer-const': 'error',
    },
  },
  {
    files: ['scripts/**/*.mjs', 'eslint.config.js'],
    languageOptions: { globals: { ...globals.node, ...globals.browser } },
  },
];
