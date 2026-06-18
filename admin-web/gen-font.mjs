// 把 Geist 变体字体（latin）base64 内联成 @font-face，供 styles.css @import，保证页面自包含、离线可用。
// 中文字符 Geist 不含 → 自动回退系统 CJK（与上游 Afilmory 行为一致）。
import { readFileSync, writeFileSync } from 'node:fs'

const woff2 = readFileSync(
  new URL('./node_modules/@fontsource-variable/geist/files/geist-latin-wght-normal.woff2', import.meta.url),
)
const b64 = woff2.toString('base64')
const css = `@font-face{font-family:'Geist';font-style:normal;font-display:swap;font-weight:100 900;src:url(data:font/woff2;base64,${b64}) format('woff2')}\n`
writeFileSync(new URL('./src/geist-font.css', import.meta.url), css)
console.log(`geist-font.css generated (${(css.length / 1024).toFixed(0)} KB)`)
