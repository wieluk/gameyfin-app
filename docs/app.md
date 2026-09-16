# App settings

## Window and tray

**Settings, Interface**:

- **Closing the window keeps Gameyfin running**: the close button hides the window to the
  tray. Downloads run inside the app, so without this, closing the window stops them.
- **Start hidden in the tray**. Opening Gameyfin again brings the window to the front.
- **Start Gameyfin when I log in**: starts hidden, so background downloads keep going.
- **Bring Gameyfin back when a game closes** (on by default): raises the window once a game
  quits, where its save upload and any error are shown.
- **Appearance**: dark, light or match your system.

## Notifications

Three switches: downloads and installs, failures, and new versions of Gameyfin. Failures
are always shown; the others stay quiet while you are looking at the window.

Clicking a notification opens Gameyfin on the page it is about. Every notification is also
listed behind the bell in the title bar, even when its popup was switched off or held back.
Click one to go to its page, dismiss it, or **Dismiss all**. The list clears when Gameyfin
closes.

## Offline

If the server cannot be reached, a banner says so and the library shows what is on this
PC. Installed games stay playable. Everything refreshes when the server is back.

## Updates

Gameyfin checks GitHub once at startup (switch off in **Settings, About**). What happens next
depends on how you installed it:

| Install | Updates |
| --- | --- |
| Windows `.exe` | Gameyfin downloads and applies the update itself. |
| Flatpak | Updates come from the Flatpak repository; the button runs the same update. |
| `.deb`, `.rpm` | Your package manager owns the files, so Gameyfin only tells you a new version exists. |

## Account

**Settings, Account** shows the server, who you are signed in as and the connection status.
It also holds **This device's name**, shown beside every save this PC uploads, and
**Sign out**.

## Troubleshooting

**Settings, Diagnostics**:

- **Log files**: one per day. Attach the newest when reporting a problem.
- **Log detail**: set it to Debug while reproducing a problem.
- **App data**: settings, session and local records.
- **Artwork cache**: cleans itself, or press **Clear** if covers look wrong.
- **Reset settings**: puts every setting back to its default. Your server, sign-in, games
  folders, save location and per-game options stay.

Passwords for archives and WebDAV are stored in the settings file, readable only by your
user account, but not encrypted.
