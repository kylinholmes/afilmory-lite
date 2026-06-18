# Afilmory 前端（apps/web）UI 风格速查

写新页面照着抄。对象：`afilmory-main/apps/web`（React 19 + Vite + **Tailwind v4**）。共享组件 `@afilmory/ui`（`packages/ui`），工具 `@afilmory/utils`。
权威来源：`apps/web/AGENTS.md`、`.cursor/rules/color.mdc`、`apps/web/src/styles/tailwind.css`。

> ⚠️ `be/apps/dashboard` 用的是另一套（Pastel）调色板——别把它的约定带进 apps/web。

## 1. 颜色：Apple UIKit 语义 class（**禁裸 hex**）

Token 全来自 `tailwindcss-uikit-colors/v4/macos.css`（无 tailwind.config.js，CSS-first）。暗色靠 `data-theme="dark"`，accent = iOS 蓝 `#007aff`，单图页会按图片取色注入 `--color-accent`。

| 类别 | class | 说明 |
|---|---|---|
| 文本 | `text-text` / `-secondary` / `-tertiary` / `-quaternary` / `-quinary`（+ `-vibrant-*`） | 主→次级文本 |
| 填充 | `bg-fill` / `-secondary` … `-quinary`（+ `-vibrant-*`） | 控件/卡片背景 |
| 材质(玻璃) | `bg-material-ultra-thin`/`-thin`/`-medium`/`-thick`/`-ultra-thick`/`-opaque` | 玻璃面板（薄→厚） |
| 强调 | `bg-accent` `text-accent` `border-accent`（带不透明度 `border-accent/20` `bg-accent/5` `text-accent/80`） | 品牌色 |
| 界面 | `bg-popover` `bg-menu` `bg-sidebar` `bg-tooltip` `bg-selection-focused` `bg-background` | 特定区域 |
| 系统色 | `text-red` `bg-blue` …（red/orange/yellow/green/mint/teal/cyan/blue/indigo/purple/pink/brown/gray） | 语义系统色 |
| 控件 | `bg-control-enabled` / `-disabled` | — |

规则：用语义 class + 正确前缀（`text-`/`bg-`/`border-`），自动明暗适配；只有复杂渐变/阴影才内联 `color-mix()`。深浅差异用 `dark:` 前缀。

## 2. Glassmorphic Depth（浮层招牌）

5 原则：多层透明叠深度 · 品牌色仅 5–20% 用于边框/微光/背景 · 重 blur(`backdrop-blur-2xl`，更强 `backdrop-blur-3xl`/`-[120px]`) · 柔和多重阴影 · spring 动画。

**标准玻璃层组合**（Tooltip/HoverCard/DropdownMenu 几乎逐字）：
```tsx
className="backdrop-blur-2xl rounded-2xl border border-accent/20 relative overflow-hidden text-text"
style={{
  backgroundImage: 'linear-gradient(to bottom right, color-mix(in srgb, var(--color-background) 98%, transparent), color-mix(in srgb, var(--color-background) 95%, transparent))',
  boxShadow: '0 8px 32px color-mix(in srgb, var(--color-accent) 8%, transparent), 0 4px 16px color-mix(in srgb, var(--color-accent) 6%, transparent), 0 2px 8px rgba(0,0,0,0.1)',
}}
```
内部叠一层 inner glow（绝对定位）：
```tsx
<div className="pointer-events-none absolute inset-0 rounded-2xl"
  style={{ background: 'linear-gradient(to bottom right, color-mix(in srgb, var(--color-accent) 5%, transparent), transparent, color-mix(in srgb, var(--color-accent) 5%, transparent))' }} />
```
- **不透明实色面板**用 material：`bg-material-thick border-fill-tertiary rounded-2xl border shadow-2xl backdrop-blur-[120px]`。
- **覆盖在图片上的控件**用 black 玻璃：`bg-black/20 text-white backdrop-blur-md border border-white/10 rounded-full`。

## 3. 排版 / 圆角 / 间距 / 尺寸

- 字体：`font-sans`(Geist,默认) / `font-serif` / `font-mono`；标题 `text-lg font-semibold tracking-tight`，正文/控件 `text-sm`，次要 `text-xs`，字重 `font-medium`/`font-semibold`。
- 圆角：`rounded`(0.5rem,输入/按钮) `rounded-lg`(Dialog) `rounded-xl`(菜单) `rounded-2xl`(玻璃面板) `rounded-full`(圆按钮/点)；连续曲率 `shape-squircle`。
- 间距：面板 `p-4`/`p-5`；菜单项 `px-2.5 py-1`；元素 `gap-2`/`gap-1.5`/`gap-4`。
- 尺寸：图标按钮 `size-7`/`size-8`/`size-10`；图标 `size-3.5`/`size-4`/`size-5`；按钮高 `h-8/h-10/h-11`(sm/md/lg)；状态点 `size-1.5`+`rounded-full`；细线 `h-[0.5px]`。

## 4. 交互

- **hover 用 `data-highlighted`（Radix），不挂 JS**：`className="data-highlighted:text-accent"` + `style={{ '--highlight-bg': 'linear-gradient(...accent 8%...)' }}`。
- **动画 = motion + Spring**：`import { m } from 'motion/react'`（LazyMotion 用 `m.*` 不是 `motion.*`）；`import { Spring } from '@afilmory/utils'` → `transition={Spring.presets.smooth}`（默认）/`.snappy`/`.bouncy`；手势 `whileHover={{scale:1.1}}` `whileTap={{scale:0.95}}`；进出场 `initial/animate/exit`（玻璃常用 `{opacity:0,scale:0.95,y:4}`）。已全局 `reducedMotion="user"`。
- **focus**：全局清了 outline，组件自加 `focus:ring-2 focus:ring-accent/40`（或用 `@afilmory/utils` 的 `focusRing`/`focusInput` 预设）。
- **class 合并**：统一 `clsxm`（= twMerge(clsx)）`import { clsxm } from '@afilmory/utils'`。

## 5. 关键组件（优先复用 `@afilmory/ui`，别自己造）

| 组件 | 用途 / 关键 props |
|---|---|
| `Button` | 主按钮。`variant`(primary/secondary/light/ghost/text/destructive) `size`(xs–xl) `isLoading` `asChild`。primary=`bg-accent text-text shape-squircle` |
| `GlassButton` | 浮在图片上的圆形玻璃 FAB |
| `Dialog`/`DialogContent` | 模态框（Radix）。`from`(top/bottom/left/right) `dismissOnOutsideClick`；命令式弹窗用已挂载的 `ModalContainer` |
| `DropdownMenu` / `Select` | 下拉（玻璃 + `data-highlighted`） |
| `Tooltip` / `HoverCard` | 提示气泡 / 悬浮卡（玻璃标准组合） |
| `Input` / `Textarea` | 输入框。`error?` |
| `Checkbox` / `Switch` | 复选/开关（Radix + path 动画） |
| `SegmentGroup` / `SegmentItem` | 分段控制（`m.span layout layoutId` 滑块 + Spring） |
| `ScrollArea` | 自定义滚动条 |
| `Collapsible` | 折叠容器（高度动画） |
| `LinearDivider` / `LinearBorderContainer` | 渐变细线 / 四边渐变边框容器 |
| `EllipsisWithTooltip` | 省略号+tooltip |

## 6. 加页面（pages 薄壳 + modules 真逻辑）

- **File-based routing**：页面放 `apps/web/src/pages/**/*.tsx`，`vite-plugin-route-builder` 自动生成路由，**无需手动登记**。
- 文件约定：导出 **`export const Component = () => {...}`**（不是 default）；`(group)`=route group(不产生 URL 段，可放 `layout.tsx` 含 `<Outlet/>`)；`[param]`=动态段(用 `useParams()`)；`index.tsx`=默认。
- **pages = 薄壳**（URL 同步 + lazy + Suspense/ErrorBoundary）；**真实 UI/逻辑放 `apps/web/src/modules/<domain>/`**。
- 状态：**Jotai**(`jotaiStore`, atoms 在 `src/atoms`) + **TanStack Query**；避免 prop drilling，store 下沉到 feature。
- i18n：`const { t } = useTranslation()`；文案先改 `locales/app/en.json`（flat key、`.` 分隔、复数 `_one`/`_other`、key 不能既是叶子又是父路径）。
- 主题：无 Provider，靠 `data-theme` + CSS 变量；accent 由 `(main)/layout.sync.tsx` 注入。

## 7. 图标

1. **lucide-react**（功能图标，主选）：`import { Pencil } from 'lucide-react'` → `<Pencil className="size-4" />`
2. **iconify CSS 类**（业务图标，`<i>` 标签）：`<i className="i-mingcute-search-line text-base" />`（集合 `@iconify-json/mingcute`，`-line`/`-fill`）
3. **品牌/社交**：`<i className="i-simple-icons-github" />`
4. **自定义 SVG**：`apps/web/src/icons/index.tsx`（`width="1em"` 继承字号）

装饰图标加 `aria-hidden="true"`；组件 API 常传 `icon: string`（即 `i-mingcute-*` 类名）。

## 8. 新页面最小骨架

`pages/<path>/index.tsx`（薄壳）：
```tsx
import { lazy, Suspense } from 'react'
import { ErrorBoundary } from 'react-error-boundary'

const YourFeature = lazy(() => import('~/modules/<domain>/YourFeature').then((m) => ({ default: m.YourFeature })))

export const Component = () => (   // 必须叫 Component；路由自动生成
  <Suspense fallback={<div className="flex h-full items-center justify-center text-text-secondary">Loading…</div>}>
    <ErrorBoundary fallback={<div className="text-red">Error</div>}>
      <YourFeature />
    </ErrorBoundary>
  </Suspense>
)
```

`modules/<domain>/YourFeature.tsx`（套玻璃 + 语义色 + motion）：
```tsx
import { Button } from '@afilmory/ui'
import { Spring, clsxm } from '@afilmory/utils'
import { m } from 'motion/react'
import { useTranslation } from 'react-i18next'

export const YourFeature = () => {
  const { t } = useTranslation()
  return (
    <div className="flex h-full flex-col gap-4 p-5 text-text">
      <m.div
        initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} transition={Spring.presets.smooth}
        className={clsxm('relative overflow-hidden rounded-2xl border border-accent/20 p-4 backdrop-blur-2xl')}
        style={{
          backgroundImage: 'linear-gradient(to bottom right, color-mix(in srgb, var(--color-background) 98%, transparent), color-mix(in srgb, var(--color-background) 95%, transparent))',
          boxShadow: '0 8px 32px color-mix(in srgb, var(--color-accent) 8%, transparent), 0 2px 8px rgba(0,0,0,0.1)',
        }}
      >
        <div className="pointer-events-none absolute inset-0 rounded-2xl"
          style={{ background: 'linear-gradient(to bottom right, color-mix(in srgb, var(--color-accent) 5%, transparent), transparent, color-mix(in srgb, var(--color-accent) 5%, transparent))' }} />
        <div className="relative">
          <h1 className="text-lg font-semibold tracking-tight">{t('your.title')}</h1>
          <p className="mt-1.5 text-sm text-text-secondary">{t('your.subtitle')}</p>
          <div className="mt-4 flex items-center gap-2">
            <Button variant="primary" size="sm">
              <i className="i-mingcute-check-line mr-1.5 text-base" />
              {t('your.action')}
            </Button>
          </div>
        </div>
      </m.div>
    </div>
  )
}
```

**清单**：颜色只用语义 class；玻璃层=`backdrop-blur-2xl`+`border-accent/20`+渐变背景+多重 accent 阴影+inner glow；实色面板用 `bg-material-thick`；动画 `m.*`+`Spring.presets.*`；hover `data-highlighted`/`whileHover`；合 class 用 `clsxm`；基础控件复用 `@afilmory/ui`；文案走 `useTranslation()` + `locales/app/en.json`。
