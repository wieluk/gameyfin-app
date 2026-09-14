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
| **Proton (umu)** | Runs the game in Valve's Steam Runtime. UMU-Proton is the default and downloads on first launch. |
| **GE-Proton** | A community Proton build. Worth trying when cutscenes stay black or a game will not start. |
| **Wine** | The fallback for games that misbehave in Proton. Wine-Staging WoW64 is recommended; the 32-bit library variant exists for programs that need it. |
| **DXVK, vkd3d-proton** | Translate Direct3D to Vulkan. Without them games fall back to OpenGL, and DirectX 12 games do not start. |
| **Game fixes** | umu's list of per-title workarounds, matched by Steam AppID when your server knows it, by title otherwise. Updated daily. |

All of these are managed in **Settings, Compatibility**, where you can also pick older
versions if a new one breaks something.

### Prefixes

Every Windows game gets its own prefix, a small Windows environment in
`Gameyfin/Prefixes`. Its games folder is mapped to a drive letter so installers put files
where Gameyfin expects.

In **Settings, Compatibility, Compatibility prefixes** each prefix has **Wine settings**,
**Registry**, **Browse C:**, **Winetricks** and **Delete**. Compatibility settings exist on Linux only. A deleted prefix is rebuilt on next launch, but anything
the game kept inside it is gone, including saves that were not backed up.

**Winetricks** installs components such as `vcrun2022` or `d3dcompiler_47` into that
prefix, which is what many ProtonDB fixes ask for. Type the verbs and press **Run**. On
Proton nothing else is needed. A game on Wine needs winetricks installed on this PC.

### Per-game options

Open a game's options in **Installed**:

- **Launch options**: flags passed to the game, such as `-windowed`.
- **Environment variables**: one `KEY=value` per line, the place for fixes copied from
  ProtonDB. `WINEDLLOVERRIDES=dxgi=builtin` turns DXVK off for this game only.
- **Runtime**: force Proton or Wine for this game. Changing it rebuilds the prefix.
- **Proton build**: pick a specific build, including ones Steam installed.
- **Wayland** and **WOW64**, for games on Proton. Wayland draws the game without XWayland.
  WOW64 runs 32-bit games without 32-bit system libraries. Both need Proton 10 or
  GE-Proton. A variable typed in the box above wins over either switch.

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
