# Playing games

## Launching

Press **Play**. Gameyfin watches the whole game while it runs, including games that start
through a launcher that closes at once, and records your playtime. If a game quits within a
few seconds, Gameyfin reads its output and tells you why in plain words where it can.

With save sync on, a newer save from another PC is restored before the game starts and your
save is backed up after it closes. See [Save sync](saves.md).

## Windows games on Linux

Gameyfin runs Windows games the way Steam does. You do not need Steam installed.

| Piece | What it does |
| --- | --- |
| **Proton (umu)** | Runs the game in Valve's Steam Runtime. UMU-Proton downloads on the first launch. |
| **GE-Proton** | Optional, with extra media codecs. Download it in **Settings, Compatibility**, then pick it in a game's options. |
| **Wine** | Gameyfin's own fallback, downloaded when needed: for a 32-bit program on a system without 32-bit libraries, or when Proton cannot start. |
| **Game fixes** | umu's list of per-title workarounds, matched by Steam AppID when your server knows it, by title otherwise. Updated daily. |

Gameyfin downloads all of these itself, so nothing needs installing on your system. A newer
Proton build replaces the old one.

### Prefixes

Every Windows game gets its own prefix, a small Windows environment in
`Gameyfin/Prefixes`. Its games folder is mapped to a drive letter so installers put files
where Gameyfin expects.

Once a Windows game has run, its options in **Installed** have **Compatibility prefix**,
with **Wine settings**, **Registry**, **Browse C:**, **Winetricks** and **Delete prefix**.
Compatibility settings exist on Linux only. A deleted prefix is rebuilt on next launch, but
anything the game kept inside it is gone, including saves that were not backed up. Prefixes
of games that are no longer installed are listed under **Settings, Compatibility, Leftover
prefixes**.

### Winetricks

Winetricks installs Windows components a game expects but does not bring along, such as
the Visual C++ runtimes, DirectX extras or .NET. Most games need none, and Proton already
applies fixes for the games it knows.

When a game fails to start because a component is missing, Gameyfin names it in the error
and ticks it in the game's **Compatibility prefix**. Press **Install** there. You can also
tick components yourself, or type the verbs a ProtonDB report mentions. On Proton nothing
else is needed. A game on Wine needs winetricks installed on this PC.

### Per-game options

Open a game's options in **Installed**:

- **Launch options**: flags passed to the game, such as `-windowed`.
- **Environment variables**: one `KEY=value` per line, the place for fixes copied from
  ProtonDB. `WINEDLLOVERRIDES=dxgi=builtin` turns DXVK off for this game only.
- **Proton build**: GE-Proton instead of UMU-Proton, once GE-Proton is downloaded.
- **Wayland** and **WOW64**. Wayland draws the game without XWayland. WOW64 runs 32-bit games
  without 32-bit system libraries, but can break anti-cheat. A variable typed in the box above
  wins over either switch.

These fields are not a shell. Quoting works, nothing else is interpreted.

### Installer memory limit

Repack installers can hang when they see a lot of RAM. Gameyfin caps them at half your
memory (at least 4 GB) by default. Change it in **Settings, Compatibility**.

## Shortcuts

In a game's options in **Installed**, add it to:

- **Applications menu**
- **Desktop**
- **Steam**, as a non-Steam game, for every Steam account on this PC. Restart Steam to see it.
  Handy for Big Picture and the Steam Deck.

Shortcuts start the game through Gameyfin, so the prefix, playtime tracking and save sync
still apply.

## Controllers

Connected controllers can drive the whole app. Press **Start** for the button map.

**Settings, Interface, Controller**:

- **Read connected controllers**: turn controller input on or off.
- **Switch to the large layout when a controller connects**: bigger text and covers for
  the sofa. Switch back from the controller overlay at any time.
