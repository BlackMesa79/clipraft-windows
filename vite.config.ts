import { defineConfig } from 'vite';
import vue from '@vitejs/plugin-vue';
export default defineConfig({
  plugins: [vue()],
  clearScreen: false,
  server: {
    host: '127.0.0.1',
    port: 1420,
    strictPort: true,
    // Rust 构建和本地工具不属于前端热更新范围，避免监听被占用的二进制文件。
    watch: { ignored: ['**/src-tauri/**', '**/.tools/**', '**/.test-data/**'] },
  },
  envPrefix: ['VITE_', 'TAURI_ENV_'],
  build: { target: 'chrome105' },
});
