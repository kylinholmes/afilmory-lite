// 把编译好的 Tailwind CSS 内联进页面模板，产出自包含的 ../src/server/admin_page.html（Rust include_str! 用）。
import { readFileSync, writeFileSync } from 'node:fs'

const css = readFileSync(new URL('./dist/styles.css', import.meta.url), 'utf8')
const html = readFileSync(new URL('./src/admin.html', import.meta.url), 'utf8')
// 生产：把 dev 用的 <link> 换成内联 <style>，产出零外部依赖的单文件
const out = html.replace('<link rel="stylesheet" href="/styles.css" />', () => `<style>${css}</style>`)
const dest = new URL('../src/server/admin_page.html', import.meta.url)
writeFileSync(dest, out)
console.log(`wrote ${dest.pathname} (${(out.length / 1024).toFixed(1)} KB)`)
