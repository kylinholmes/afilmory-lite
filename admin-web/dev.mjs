// admin-web dev 预览：Tailwind --watch 实时编译 + 静态 server。
// 跑法：bun dev.mjs（或 bun run dev / npm run dev），然后开 http://localhost:5174/?demo=1
import { createServer } from 'node:http'
import { readFile } from 'node:fs/promises'
import { spawn, spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { dirname, join, extname } from 'node:path'

const root = dirname(fileURLToPath(import.meta.url))
const PORT = Number(process.env.PORT) || 5174

// 0) 先生成 Geist @font-face（styles.css @import 它）
spawnSync(process.execPath, [join(root, 'gen-font.mjs')], { cwd: root, stdio: 'inherit' })

// 1) Tailwind --watch（用当前运行时 = bun，避免 node18 跑 tailwind v4 的兼容问题）
const twBin = join(root, 'node_modules', '.bin', 'tailwindcss')
const tw = spawn(process.execPath, [twBin, '-i', './src/styles.css', '-o', './dist/styles.css', '--watch'], {
  cwd: root,
  stdio: 'inherit',
})

// 2) 静态 server：/ 与 /admin → 页面模板；/styles.css → 编译产物
const TYPES = { '.css': 'text/css; charset=utf-8', '.html': 'text/html; charset=utf-8' }
const server = createServer(async (req, res) => {
  let p = req.url.split('?')[0]
  if (p === '/' || p === '/admin') p = '/src/admin.html'
  else if (p === '/styles.css') p = '/dist/styles.css'
  try {
    const buf = await readFile(join(root, p))
    res.setHeader('content-type', TYPES[extname(p)] || 'application/octet-stream')
    res.setHeader('cache-control', 'no-cache')
    res.end(buf)
  } catch {
    res.statusCode = 404
    res.end('not found')
  }
})
server.listen(PORT, () => {
  console.log(`\n  ▸ admin dev 预览:  http://localhost:${PORT}/?demo=1`)
  console.log(`    改 src/admin.html 或 src/styles.css → 自动重编译，刷新浏览器即见（Ctrl-C 退出）\n`)
})

const bye = () => {
  tw.kill()
  server.close()
  process.exit(0)
}
process.on('SIGINT', bye)
process.on('SIGTERM', bye)
