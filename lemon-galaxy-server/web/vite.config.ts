import { fileURLToPath, URL } from 'node:url'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import Components from 'unplugin-vue-components/vite'
import { AntdvNextResolver } from '@antdv-next/auto-import-resolver'

// 开发时把 /api 转给本机的 LemonGalaxy 服务器（默认 https://127.0.0.1:7443，自签证书）。
// 产物输出到 dist/，由服务器用 rust-embed 打进可执行文件，所有页面走 history 路由。
const target = process.env.GALAXY_API ?? 'https://127.0.0.1:7443'

export default defineConfig({
  base: '/',
  resolve: { alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) } },
  plugins: [vue(), Components({ resolvers: [AntdvNextResolver()], dts: 'src/components.d.ts' })],
  server: {
    port: 5190,
    proxy: {
      '/api': { target, secure: false, changeOrigin: false, ws: true },
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    target: 'es2022',
    sourcemap: false,
    chunkSizeWarningLimit: 1800,
    assetsInlineLimit: 0,
  },
})
