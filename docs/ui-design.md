# Sonata UI conventions

The main task dialog is the reference for Sonata's interface: compact typography,
quiet borders, clear title and description fields, and a small set of prominent
actions. Keep the purple/navy palette and the current navigation destinations.

## Shared components

`app/src/components/ui.tsx` owns the presentation primitives:

- `PageHeader` aligns a heading, optional description, and page actions.
- `EditorSection` groups settings into labeled rows that stack in narrow windows.
- `Button`, `Icon`, `StatusBadge`, and `PropertyButton` provide consistent controls.
- `Field` keeps a visible label, control, hint, and validation message together.
- `Dialog` supplies native modal focus containment, initial focus, Escape handling,
  and focus restoration. `DialogHeader` supplies its heading and close action.
- `ActionMenu` contains secondary conversation actions and supports keyboard
  navigation, Escape, and focus restoration.

Shared geometry and interaction styles live in `app/src/styles.css`. Use the
4px spacing scale, 6px control radius, 8px surface radius, and 14px dialog radius.
Keep colors in theme tokens; derive hover and selected surfaces from those tokens.
The light and dark token blocks must stay aligned with the picker window.
Omarchy colors remain user supplied; accent button text uses a 4.5:1 minimum
contrast rule independently of avatar initials.

## Choosing a layout

| Surface | Layout |
| --- | --- |
| Main task, recurring task, project | Modal over the relevant board or list, scrolling body, visible footer |
| Bot and skill editor | Centered page with a prominent name and sticky save/cancel footer |
| Workflow editor | Canvas and inspector, stacked below 900px |
| Management lists | Compact rows, aligned actions, wrapped controls at narrow widths |
| Settings | Section names beside controls on wide windows; stacked on small windows |
| Task details and chat | Side drawer, full width below 900px; independent transcript scrolling |

Task cards retain questions, answer choices, dependencies, progress, and actions.
Finished work and archived Done work remain distinct. Pending and Done visibility,
project ownership, shared skills, and resumable onboarding keep their existing
behavior.

## Interaction rules

Recurring task and project dialogs use `useFormGuard` to confirm dismissal of
dirty drafts, prevent duplicate saves, retain drafts after errors, and ignore late
callbacks after unmounting. Native directory picking is guarded separately.
The main task dialog retains its attachment, dependency, and submission guards.
Backdrop clicks do not discard drafts. Command/Ctrl+Enter submits editor dialogs.

Keep Stop, Terminal, and the browser toggle directly available in chat. Put session
management, folder access, and bot editing in the conversation action menu. Scroll
only the transcript when messages arrive, so the surrounding drawer stays in place.
Short windows use a smaller composer and scroll overflowing detail panels. Honor
reduced motion and preserve visible keyboard focus.

## Verification

Run `npm test -- --run` and `npm run build` from `app/`. Dialog regression coverage
includes focus, discard confirmation, validation, duplicate submissions, failed
saves, project directory rules, and late native-picker/save callbacks.

For visual checks, render the actual source components with fixture data at
760×480, 1100×720, and 1440×900. Check light, dark, and Omarchy themes, long
translated labels, modal scrolling, editor footers, and conversation controls.
These browser checks do not replace macOS WebKit or Linux WebKitGTK acceptance
checks for packaged desktop releases.
