# @mediaservo/ui — TOKENS（可覆盖变量清单 = 对外公共 API）

> v0.2.0（talk-app T1 / 裁决 L4）。真源：`tokens.css`（默认主题）+ `themes/{fluent,material}.css`（design-multi-style B3b，语义覆盖式）。
> **稳定性承诺**：本清单中的每个变量都是对外接口——名字与语义不随意变更；取值可随主题精修调整（视觉级），删名/改义需升主版本并在变更说明中列出迁移。

## 使用契约（4 条）

1. **组件只消费语义层**（`--ms-color-*` / `--ms-font-*` / `--ms-space-*` / `--ms-radius-*` / `--ms-shadow-*` / `--ms-motion-*` / `--ms-control-*` …），**禁止**裸 hex、禁止直接用 `--ms-palette-*`（色板层仅供主题取值引用）。
2. **主题 = 覆盖语义变量**（先例：`themes/fluent.css`、`themes/material.css` 用 `html.theme-*` 类作用域；默认主题的暗色用 `:root[data-theme="dark"]`，见 `tokens.css`）。
3. **消费方可任意容器作用域覆盖**（运行时换肤，无需重编译）：
   ```css
   .my-brand-scope { --ms-color-primary: #7c3aed; --ms-radius-md: 4px; }
   ```
4. **新增变量须有真实消费者**（沿 B8 纪律），并同笔登记进本清单。

## 变量清单

### 色板层（palette — 主题取值用，组件禁直接引用）
| 前缀 | 变量 |
|---|---|
| `--ms-palette-blue{,-hi,-ink,-tint}` | 原型电蓝系（accent 族） |
| `--ms-palette-green{,-ink,-tint}` / `--ms-palette-amber{,-ink,-tint}` / `--ms-palette-red{,-ink,-tint}` | 状态色系 |
| `--ms-palette-cyan` / `--ms-palette-violet` | info / meeting 点缀 |
| `--ms-palette-video-{bg,ink,ok,warn,danger}` | 视频画面内叠层专用 |

### 品牌 / 状态（light 值 ←→ dark 值）
| 变量 | light | dark |
|---|---|---|
| `--ms-color-primary` / `-hover` | blue / blue-hi | blue / blue-ink |
| `--ms-color-primary-soft` / `-soft-border` | `#eef1ff` / `#d8deff` | blue-tint / `#37415a` |
| `--ms-color-danger{,-soft}` | `#e5484d` / `#fdecec` | red / red-tint |
| `--ms-color-success{,-soft}` | `#1f9d55` / `#e6f7ee` | green / green-tint |
| `--ms-color-warning{,-soft}` | `#c97a00` / `#fdf3e2` | amber / amber-tint |

### 文字 / 背景 / 边框
| 变量 | light | dark |
|---|---|---|
| `--ms-color-text{,-secondary,-tertiary,-muted,-faint,-inverse}` | `#141821/#4a5364/#646e82/#9aa3b2/#9aa3b2/#fff` | `#e7ecf5/#a7b2c5/#7e8ba1/#4e5869/#4e5869/#0b0e14` |
| `--ms-color-bg` / `-surface` / `-surface-subtle` | `#f2f4f8` / `#fff` / `#fff` | `#0b0e14` / `#131824` / `#1b2230` |
| `--ms-color-surface-hover` / `-surface-muted` | `#f1f4f9` / `#e9edf3` | `rgba(255,255,255,.05)` / `#080b11` |
| `--ms-color-border{,-subtle,-strong,-interactive}` | `#e4e8ef/#e4e8ef/#b9c1ce/primary` | `#242c3c/#242c3c/#37415a/primary` |

### 字体 / 间距 / 圆角 / 尺寸（主题无关）
`--ms-font-family`（含 PingFang SC/YaHei CJK 栈）、`--ms-font-family-mono`、`--ms-font-size-{xs,sm,base,md}`、`--ms-font-weight-{normal,medium,semibold}`、`--ms-font-leading-{tight,normal}`；`--ms-space-1..6`（0.25–1.5rem）；`--ms-radius-{sm 4,md 8,lg 10,xl 16,full}`。

### 布局常量（**待 B-shell 批重订**）
`--ms-size-header-h: 48px`、`--ms-size-sidebar-w: 200px` —— 现仍为 admin 现值；原型 `--top-h 54 / --nav-w 230` 由 B-shell 批（壳）落地时更新。

### 样式系统维度（design-multi-style B3a，主题共享）
`--ms-shadow-1..3`；`--ms-focus-ring-color`（light `#3651d4` / dark blue-ink）+ `-width 2px`；`--ms-motion-{duration-fast,duration-base,easing}`；`--ms-control-h-{sm 28,md 34}`；`--ms-icon-size-{sm 16,md 18}`；`--ms-state-layer-{hover,pressed}`（暗色下为白系叠加）；`--ms-scrim`；`--ms-layer-01..03`（= surface/subtle/muted 别名）；`--ms-transition-fast`。

## 显式挂账（未加，等真实消费者）
`*-ink`（tint 上文字色，原型有值待映射）、`--ms-color-info`（cyan 系）、暗色阴影分层（现共享 B3a 三档）。

## 校验
`pnpm -C packages/ui check`（PIT-258 守卫：www/ 下任何 `var(--ms-*)` 引用未定义即红；已接入 `lint` 车道）。
