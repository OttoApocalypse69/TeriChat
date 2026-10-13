# Native notification smoke (issue #43), Linux

Real Tauri debug build of this branch (`custom-protocol`, bundled `dist`),
real `terichat-server` on `127.0.0.1:3001` with a throwaway Postgres 16, under
Xvfb with a session D-Bus and `dunst` as the freedesktop notification server.
Accounts and messages are synthetic (`notify_a` receives, `notify_b` sends
through the HTTP API with `smoke-driver.mjs`). Input was driven with `xdotool`;
focus moved to a second X window (`xclock`) to put the app in the background.

`linux-notify-bus.log` is `dbus-monitor` filtered to
`org.freedesktop.Notifications.Notify` for the whole run.

| # | App state when the message arrived | Notify calls | Result |
|---|---|---|---|
| 1 | Focused, DM open | 0 | Silent, message rendered |
| 2 | Unfocused | 1 (`Smoke Bee` / text) | Toast shown on screen by dunst |
| 3 | Unfocused | 1 | Toast shown |
| 4 | Unfocused, DM muted in the header | 0 | Silent; message and unread badge still arrive |
| 5 | Unfocused, DM unmuted | 1 | Toast shown |

Total: 5 messages, 3 `Notify` calls, matching the table (exactly one per
eligible message, none when focused or muted).

## Finding: toast click does not focus the window on desktop

Clicking the toast closed it but focus stayed on the other window. The
`Notify` calls carry an empty actions array, and
`tauri-plugin-notification` 2.4.0 desktop `show()` fires and forgets
(`let _ = notification.show()`); `onAction` is a mobile plugin listener. So
in the Tauri shell the "click focuses the app" criterion is not met on
desktop, and the same plugin path is used on Windows. Browsers are
unaffected (`Notification.onclick` focuses). Fixing it needs a native
notify command that waits for the activation (notify-rust `wait_for_action`
on Linux, WinRT `on_activated` on Windows); left as a follow-up.

## Not covered

Windows and macOS toasts (same plugin, different OS backend), signed or
installed builds. Screenshots stay local per the acceptance publication rules.
