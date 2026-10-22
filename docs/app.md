# App settings

## Window and tray

**Settings, Interface**:

- **Closing the window keeps Gameyfin running**: the close button hides the window to the
  tray. Downloads run inside the app, so without this, closing the window stops them.
  After ten minutes in the tray the window is unloaded to free its memory, a few hundred
  MB. Downloads and save sync carry on; opening the window again takes a moment longer.
- **Start Gameyfin when I log in**: starts with your session, so background downloads keep
  going.
  - **Start hidden in the tray** (on by default): that start stays in the tray. Opening
    Gameyfin yourself, or the restart after an update, always shows the window.
- **Bring Gameyfin back when a game closes** (on by default): raises the window once a game
  quits, where its save upload and any error are shown.
- **Appearance**: dark, light or match your system.
  - **Draw with the graphics card** (Linux, on by default): smoother scrolling. If Gameyfin
    ever closes before its window draws, it turns this off by itself for the next start.
    Changing it takes a restart.

While a game runs and the Gameyfin window has been in the background for a minute, the window
is unloaded so the game gets its memory. Playtime, save sync and notifications carry on. It
comes back when the game closes (with **Bring Gameyfin back when a game closes** on), or from
the tray. Without a tray icon the window stays loaded.

## Notifications

**Settings, Interface** has a main switch, **Show notifications**, and under it a switch per
kind, all on by default. Turning the main switch off stops every popup and keeps your choices
below it for when you turn it back on.

| Switch | Tells you |
| --- | --- |
| Downloads and installs | A download is ready to install, or a game is ready to play. |
| Failures | A download, install or launch went wrong. |
| New versions of Gameyfin | A new release of the app. |
| New Proton versions | A newer build of a Proton you have downloaded. |
| Signed out | The server stopped accepting your sign-in. |
| Server connection | The server stopped answering for a few minutes, and when it is back. |
| Save sync | A save could not sync, or a save needs your choice. |
| New games | Games were added to the server, also while Gameyfin was closed. |
| Updated games | An installed game has new files on the server. |

Failures and sign-outs are always shown; the others stay quiet while you are looking at the
window.

While you are signed in, Gameyfin checks the server every two minutes and the library every
ten. A game counts as updated when its size on the server changes from the size you
downloaded, which any Gameyfin server reports. Renaming a game's file or folder on the server
makes it a new game instead.

Proton builds do not update by themselves, since a new build can break a game that ran.
Gameyfin checks twice a day for a newer UMU-Proton, GE-Proton or Proton-CachyOS, only for the
ones you have downloaded, and tells you once per release. Clicking it opens **Settings,
Compatibility**, where **Update to** fetches it.

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
