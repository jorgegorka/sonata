# Midnight palette for the whole app: design

Date: 2026-11-17 · Size: small change · Companion: `2026-11-17-app-icon-design.md`

## Intent

The new icon's Midnight palette (navy, a periwinkle and indigo accent, and the bots' amber, green
and pink) becomes the app's palette, so the app and its icon feel like one product.

## Decisions (made by the user)

- Follow the system appearance, as today: dark mode becomes Midnight, and light mode gets a matching light theme.
- Variant **A, "Deep Midnight"**: dark surfaces come from the icon's navy.

## Tokens

`app/src/styles.css`. The existing tokens get new values, and the new tokens replace hardcoded colors.

| Token | Light | Dark |
| --- | --- | --- |
| `--bg` | `#ffffff` | `#15132f` |
| `--panel` | `#f4f3fb` | `#1d1a42` |
| `--text` | `#1b1840` | `#ecebff` |
| `--muted` | `#6b6890` | `#a3a1c9` |
| `--border` | `#e2e0f0` | `#2f2c5e` |
| `--accent` | `#4b44d6` | `#7a73ff` |
| `--accent-text` | `#ffffff` | `#ffffff` |
| `--mine` | `#eeedfc` | `#28245c` |
| `--error` | `#be123c` | `#fb7185` |
| `--needs-bg` | `#fff7ed` | `#2a2238` |
| `--needs-border` | `#fcd9a8` | `#6e5222` |
| `--shadow` | `rgba(27, 24, 64, 0.14)` | `rgba(5, 4, 20, 0.6)` |
| `--ok` (new: idle, ready ✓) | `#059669` | `#34d399` |
| `--busy` (new: working, starting, limited, running) | `#c96b05` | `#fbbf24` |
| `--alert` (new: needs attention, blocked) | `#e11d48` | `#fb7185` |
| `--waits` (new: waiting/queued ring) | `#c96b05` | `#f59e0b` |
| `--console-bg` (new: terminal, task output) | `#0d0b24` | `#0d0b24` |
| `--console-text` (new) | `#ecebff` | `#ecebff` |

Hardcoded colors to replace with these tokens in `styles.css`:

- `.status-dot.idle`
- `.status-dot.working, .starting, .limited`
- `.status-dot.needs_attention`
- `.check-row.ready/.unchecked strong::before`
- `.waiting-mark`
- `.state-dot.running`, `.waits` and `.blocked`
- `.terminal-host` background
- `.task-tail` background and text
- The `.task-card` box-shadow tint: use `rgba(27, 24, 64, 0.06)` in light mode, or make it a token. This one is the builder's call.

## Also in scope

- `app/src/pickbox/pickbox.css` (the element-picker popup) gets the same values for the tokens it has: `--bg` uses the panel color as today, which means `#ffffff` light and `#1d1a42` dark; plus `--text`, `--muted`, `--border`, `--accent`, `--accent-text` and `--error`.
- The xterm terminal in `app/src/components/TerminalView.tsx` draws its own background (default black). Pass `theme: { background: "#0d0b24", foreground: "#ecebff" }` so it matches `--console-bg` and `--console-text`. Keep the values in one exported constant so they don't drift.

## Not in scope

- The 10 preset bot colors in `BotForm.tsx`, and existing bots' colors.
- The custom-color swatch gradient.
- The icon, which has its own plan.
- A theme switcher.

## Known trade-off (accepted)

- White text on the dark-mode accent (`#7a73ff`) is 3.7:1, the same as today's blue (3.68:1).
- A single accent color can't pass 4.5:1 both as a button background and as link text on `#15132f`.

## Done when

- `npm test` and `npm run build` pass in `app/`.
- No hex or rgb colors remain in `styles.css` outside the token blocks, except `.avatar` (`#fff`, overridden inline) and the custom swatch gradient.
- The Tasks board, chat, settings and the workflow canvas look right in light and dark (checked by rendering the real stylesheet).
