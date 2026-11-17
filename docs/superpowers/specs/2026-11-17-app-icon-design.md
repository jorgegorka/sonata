# Sonata app icon: design

Date: 2026-11-17 · Size: small change

## Intent

Replace the current indigo "S" with an icon that matches the name and what the app does: Master
conducts, and the team of bots follows its lead. Musical, but it should read as "conductor and
ensemble" rather than as a generic music app.

## The design (chosen in the visual companion)

- **Mark:** a conductor's baton, nearly level (rotated about -28°), with a cork-style pear grip,
  a small ring, and a tapered shaft that bends slightly with the swing. Behind the tip, a dotted
  wave shows the path the baton just drew, with three bots following along it in single file.
  The largest and closest to the tip is amber `#fbbf24`, then green `#34d399`, then the smallest
  and farthest, pink `#fb7185`.
- **Baton:** white. **Trail:** white dots at about 32% opacity.
- **Background ("Midnight"):** vertical gradient `#33307f` → `#12102e`, with a soft white
  radial spotlight (about 16% at the center) toward the upper right.
- **Finish:** a soft gradient, a light shadow under the baton and bots, and the standard macOS
  drop shadow under the body.
- **Layout:** Apple's macOS grid, a 1024 canvas with an 824×824 body inset 100px, using the same
  continuous-corner curve as `app/scripts/volume-icon.swift`.
- **Menu bar:** a one-color version of the same mark, black only, as a macOS template image.
- **Reference:** `.superpowers/brainstorm/70353-1794930040/icon/reference-midnight.svg` (with
  `reference-mono.svg` and the `generator/` scripts that produced them; this folder is
  gitignored and local only).

## Goals

- New icon everywhere the app shows one: Dock/Finder (`icon.icns`), the generated Tauri icon set,
  the menu bar, and the DMG volume icon.
- Right size in the Dock next to other Mac apps (the current icon fills the canvas and looks
  oversized).
- Clear at 16px and in the menu bar.

## Non-goals

- A layered Liquid Glass icon (Icon Composer `.icon`). This is a possible follow-up.
- Changing the app's interface accent color (`--accent` stays blue).
- A committed icon generator script.

## Anti-goals

- Anything that reads as a speedometer needle or gauge (round hub, marks in an arc around it), a
  paintbrush, or a fishing rod.
- A DMG icon that is padded twice.

## Approaches considered

1. **SVG source plus generated files (picked).** The SVG is the single source like today, it fixes
   the Dock size, and one script goes away.
2. SVG plus a committed generator: easy to tweak later, but about 200 lines of geometry code and
   an extra rendering tool for an icon that rarely changes.
3. A native layered Liquid Glass icon: looks most at home on current macOS, but Tauri bundler
   support is uncertain and it may need custom build steps. Deferred.

## How it gets built

1. Replace `app/app-icon.svg` with the design above as clean, static paths: the real squircle
   path, gradient, spotlight and shadows. Render `app/app-icon.png` (1024×1024) from it.
2. Regenerate `app/src-tauri/icons/` with `npm run tauri icon app-icon.png` (from `app/`).
3. Replace `ICON_ROWS` in `app/src-tauri/src/tray.rs` with a 36×36 rasterization of the
   one-color mark. The existing tray tests must still pass (36×36, black only, transparent
   border, solid and anti-aliased pixels).
4. DMG: copy `src-tauri/icons/icon.icns` to `.VolumeIcon.icns` in `app/scripts/make-dmg.sh` and
   delete `app/scripts/volume-icon.swift`, which only existed to pad a full-canvas image.

## Calls I made (check these)

- Menu bar: baton plus the three dots, without the dotted trail. At 18pt the trail is noise.
  Keep it only if it reads cleanly at 36px.
- `tauri icon` also rewrites the iOS, Android and Windows files under `src-tauri/icons/`. They
  get committed as regenerated, as they are today, even though the app ships for macOS only.
- Watch for banding in the dark gradient at 1024px. If it shows, add a little noise or dithering.

## Left to the builder

- Exact path coordinates, as long as the result matches the reference.
- How the 36×36 tray values are produced (a one-off script, not committed).

## Done when

- `cargo test` for the Tauri crate passes (including the tray tests), and `npm test` passes.
- A bundled app shows the new icon in the Dock and Finder at the same size as neighboring
  apps. The menu-bar icon reads in light and dark menu bars. A DMG built with `npm run dmg:unsigned`
  shows the new volume icon, padded once.
