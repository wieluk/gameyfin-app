# Save sync

Gameyfin backs up your saves after you play and restores the newest one before you start,
on every PC that syncs to the same place. You can also restore older versions and move
saves to a new location.

> **Using a Gameyfin server for saves** needs server support that is not merged into
> [Gameyfin](https://github.com/gameyfin/gameyfin) yet. Until it is, run `ghcr.io/wieluk/gameyfin:save-sync` as your server.
> A folder or WebDAV share works with any Gameyfin server.

## How it works

1. **Finding saves.** Gameyfin uses [Ludusavi](https://github.com/mtkennerly/ludusavi), which
   comes with the app. Ludusavi has a community database of where thousands of games keep
   their saves: files in Documents or AppData, folders beside the game, and on Windows,
   registry keys.
2. **Backing up.** When a game closes, Ludusavi copies its save files into
   `Gameyfin/Saves` in your games folder. Gameyfin packs them and uploads them as a new
   version, but only if something changed.
3. **Restoring.** Before a game starts, Gameyfin checks for a newer version from another PC,
   downloads it and lets Ludusavi put each file back where it belongs.

A small window shows each step. You can skip a restore while it downloads. A session that
started without the newer save is then not uploaded afterwards, so it cannot bury that save;
you choose which one to keep under **Saves**. If a restore fails, the game waits for you
instead of starting on an old save.

### Recognising a game

Gameyfin matches each game to the Ludusavi database by its Steam AppID if your server
knows it, then by title. It never guesses: if it is not certain, the game shows
**Not sure which game this is** and you press **Choose game** to pick the right entry.
A wrong match would restore one game's save over another's.

The database updates itself once a day. If a new game is not recognised, press **Update
game database** in **Settings, Saves**.

## Where saves are kept

**Settings, Saves, Where saves are kept**:

| Location | Use it when |
| --- | --- |
| **Gameyfin server** | Your server supports save sync (see the note above). Retention is set on the server. |
| **A folder** | Something else syncs that folder: Syncthing, rclone, Nextcloud or Dropbox. In the Flatpak it must be inside your home folder. |
| **WebDAV** | You have a Nextcloud, ownCloud or other WebDAV share and do not want to mount it. Use an app password: it is stored unencrypted. |

For a folder or WebDAV, **Versions to keep per game** (default 10) sets how many old
versions stay. Press **Test connection** after changing anything.

Other options:

- **Sync my saves**: the main switch.
- **Restore before a game starts** and **Back up after a game closes**: the two automatic steps.
- **This device's name**, in **Settings, Account**: shown beside every save this PC uploads.

## The Saves page

Each game shows one line saying where it stands:

| Status | Meaning | What to do |
| --- | --- | --- |
| Backed up | This PC and the store match. | Nothing. |
| Played, not uploaded yet | You played since the last upload. | **Back up**, or it happens next time the game closes. |
| A newer save from another PC | Another PC uploaded since. | **Restore**, or just start the game. |
| Both have unsaved progress | Both PCs played since they last synced. | **Resolve**, see [Conflicts](#conflicts). |
| Saved on (system), which does not map onto (system) | The save came from a system this PC cannot restore directly. | See [Windows, Proton and Linux](#windows-proton-and-linux). |
| Not sure which game this is | No certain database match. | **Choose game**, or **Set folders**. |
| Nothing saved yet, or the saves are not where the game keeps them | Recognised, but no files found. | Play first, or **Set folders**. |
| Not backed up yet | Never synced on this PC. | **Back up**. |

By default only installed games are listed. **Show all saves** adds games with stored saves
that are not installed here, and **Show all games** lists your whole library. A save can only
be restored once its game is installed.

Expand a game to:

- see every **stored version** with its date, device, system and size
- **restore** any version
- **lock** a version so cleanup never removes it
- **delete** versions
- **Open folder** where the saves are on this PC
- **Set folders** or **Choose game**

At the top, **Open saves folder** opens `Gameyfin/Saves`, and **Delete all saves** removes
every stored save for every game. Save files on this PC are not touched.

## First start on a PC

The first time you start a game on a PC and saves for it already exist in the store,
Gameyfin asks **Which save?** It always asks, because you may have played the game here
before using Gameyfin.

- **Restore and play**: pick a version (the newest one that works here is preselected).
- **Keep mine and play** (or **Play without a save**): keeps what this PC has. You are asked
  again only when a newer save arrives.
- **Not now**: the game does not start, and the question comes back next time.

If this PC already has save files, the dialog says when they last changed, so you can
compare.

## Conflicts

A conflict means two PCs both played since they last synced, so one save would overwrite
the other. Gameyfin shows both side by side:

- **Keep this PC's save**: uploads it. The other stays in the version history.
- **Keep the cloud save**: restores the other PC's save over this one.
- **Keep both**: uploads this PC's save and keeps the other in history. You can restore
  either later.
- **Decide later**

## Windows, Proton and Linux

Saves carry the system they were made on:

| Tag | Made by |
| --- | --- |
| **Windows** | a game on Windows |
| **Windows, on Proton** | a Windows game on Linux, through Proton or Wine |
| **Linux** | a native Linux build |

**Windows and Proton saves are interchangeable.** A Windows game on Linux keeps its saves in
Windows-style folders inside its prefix, and Gameyfin maps that prefix's user folder onto the
real user folder on Windows. So you can play on a Windows desktop, carry on on a Steam Deck,
and go back.

Your username and install folder do not matter either: both are stored as neutral
placeholders and filled in with this PC's own paths on restore.

**Windows or Proton to a native Linux build** does not map on its own, because the save
folders are completely different. The game says it was saved on another system:

- **Try anyway** uses Ludusavi's path translation. It is best effort and does not carry
  registry settings.
- Otherwise use **Set folders** to say where the saves belong.

**Registry saves.** A few games keep saves or settings in the Windows registry. On Windows,
Ludusavi backs those up. Inside a Proton prefix only files are captured.

## Steam

- **Games from Steam itself** are not synced automatically, since only games in your Gameyfin
  library are. If the same game is in your library, **Find saves on this PC** (below) can
  back up the progress you made in Steam.
- **Steam AppIDs** from your server make recognition more reliable.
- **A Gameyfin game added to Steam** as a shortcut still starts through Gameyfin, so its saves
  sync as usual. See [Shortcuts](playing.md#shortcuts).

## Find saves on this PC

**Saves, Find saves on this PC** scans the whole machine for anything the Ludusavi database
recognises: normal save locations, your Steam library (including Proton games' saves on
Linux) and every Gameyfin prefix. It can take a few minutes.

Each find is matched to a game in your library where possible. Tick what you want and press
**Back up**. A find with no match needs **Match to a game** first, since a stored save always
belongs to a library game. Nothing is backed up until you tick it.

Use this when you start using Gameyfin on a PC that already has progress.

## Set folders

For a game the database does not know, or where it looks in the wrong place, press
**Set folders** on its row.

- **Save folders**: where this game keeps saves on this PC. Adding one makes an unknown game
  backupable. For a known game, the folders are added to what the database already finds.
  For a Windows game on Linux, **Browse** starts inside its prefix, and a folder picked there
  still lands in the right place on a Windows PC.
- **Path corrections**: only for saves travelling between PCs that keep them in different
  places. The first box is the folder on this PC, the second is the name it is stored under,
  which must be the same on every PC, for example `/gameyfin/home/Example`.
- **Translate between Windows and Linux paths**: the same translation as **Try anyway**. Not
  needed for Windows games on Proton.

## Move saves

**Settings, Saves, Move saves** copies saves between the server, a folder and WebDAV, for
example when switching location.

- Nothing is deleted from the source, so running it again is safe and only copies what is
  missing.
- By default only each game's newest save is copied. Tick **Copy every version** for the full
  history.
- Both locations use their own settings, so set up the new one before copying.

## Ludusavi version

A copy of Ludusavi ships with the app. **Settings, Saves** can download a newer or an older
version. Removing a downloaded one falls back to the built-in copy. Your backups are not
affected either way.

## Troubleshooting

| Problem | Try |
| --- | --- |
| "Your server does not support save sync" | The server lacks save support. Use `ghcr.io/wieluk/gameyfin:save-sync`, or a folder or WebDAV. |
| "Save sync is turned off on your server" | An administrator switched it off on the server. |
| A game is not recognised | **Update game database**, then **Choose game**, then **Set folders**. |
| Backed up but nothing found | Play and save in the game first. Check **Open folder**, then **Set folders**. |
| Restored to the wrong place | Check **Choose game** picked the right entry. Fix paths with **Set folders**. |
| A save from Windows will not restore on a native Linux build | **Try anyway**, or **Set folders**. |
