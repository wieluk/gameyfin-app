# Library, downloads and installing

## Library

The **Library** page shows every game on your server.

- **Search** by title, and pick a single library if your server has several.
- **Filters**: genre, developer, publisher, theme, feature, perspective, keyword, platform
  and rating. Tags on a game's page are clickable and filter by that value.
- **Sort** by title, last played, playtime or size.
- **Cover size**: small, medium or large.

Click a game for its details: description, playtime, release, developer, photos and
trailers.

## Downloads

Press **Download** on a game. With more than one games folder, you choose where it goes and
see the free space on each.

- Zip and tar downloads are unpacked while they arrive, so a game needs only its own size in
  free space. 7z, rar and other archives are unpacked as soon as the download finishes. An
  interrupted download starts over.
- A setup program or disc image is saved as it is and waits for **Install**.
- **Speed limit**: set one on the Downloads page. It is off by default.
- **Cancel** stops a download; **Delete download** removes a finished one.
- Progress shows on the taskbar or dock icon, and a notification tells you when a game is
  ready.

**Settings, Library, Automation**:

- **Extract automatically** (on by default): unpack archives as soon as they download.
  - **Delete the archive after extracting** (on by default).
  - **Archive password**: tried on encrypted archives.
- **Install automatically when a download finishes**: Inno Setup and NSIS installers run
  silently. Other installers, or more than one setup program, wait for you.
  - **Delete the download after installing**: the starting choice in the install dialog, off
    when there is more than one setup program.
  - **Inno Setup options** (default `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART`) and
    **NSIS options** (default `/S`). Empty restores the default.
  - **Never offer these executables**.

## Installing

Gameyfin looks at what was downloaded and handles it:

| Download | What happens |
| --- | --- |
| **Archive** (zip, 7z, rar, tar) | Unpacked into `Gameyfin/Installations`. Encrypted archives use the password from **Settings, Library, Automation**. On Linux, rar needs `unar` installed. |
| **Setup program** | Run for you, on Linux inside the game's prefix. Common installer types are told where to install, so the game ends up where Gameyfin can find it. The row shows how much it has written. |
| **Game files** | Moved into place as they are. |

**Setup options** are flags passed to the setup program and remembered for the game.
A silent flag such as `/VERYSILENT` lets it install without asking questions.

### Choosing the executable

Gameyfin picks the file to launch and asks when it is not sure. You can change it any time
in **Installed** with **Choose an executable**, and pick any file in the game's folder.

Crash handlers, redistributables and uninstallers are never offered. Edit that list in
**Settings, Library, Automation, Never offer these executables**.

## Installed games

The **Installed** page lists what is on this PC, with the install path, the running game and
per-game options: launch options, Proton settings, the compatibility prefix, shortcuts and
setup options. See
[Playing games](playing.md).

**Uninstall** looks for the game's own uninstaller and runs it, or lets you choose one.
Without one, only the game's files are removed. Uninstalling does not delete your stored saves.

## Games folders

**Settings, Library, Games folders** lists every folder Gameyfin uses, with free space.
Add one on another drive, pick the default, or stop using one. A missing folder, such as an
unplugged drive, is flagged and its games return once it is back.

**Rescan folders** finds games already in a games folder, for example after reinstalling
Gameyfin.
