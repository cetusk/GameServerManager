<p align="center">
  <img src="assets/logo-dark-trimmed.png" alt="GameServerManager" width="360">
</p>

# GameServerManager: Unified Game Server Management Tool

[日本語](README.md) | **English**

Manage game servers on Windows from a single desktop application. Select the games you use, then start, stop, configure and back up servers on the same PC.

We plan to continue adding server management support for more games.

## Features

- Supports ARK: Survival Ascended, Valheim, Windrose, Satisfactory and Conan Exiles
- Server startup and graceful shutdown; updates and installation through SteamCMD
- World and configuration backups and restoration while the server is stopped
- Create or register configuration files and edit game-specific settings in the GUI
- Server logs with full-log copying, operation status and task history
- Three dark themes and Japanese / English interface languages

Currently, Valheim startup, joining, graceful shutdown and backup creation have been verified on a real server. Other games, restoration and the full new-server setup flow have not yet been verified on real Windows servers. See [support status and limitations](docs/compatibility.md).

## Requirements

- Windows x64, managing servers on the same PC
- Hardware meeting each game's dedicated server requirements
- SteamCMD for updates and installation (download it from App settings or select an existing `steamcmd.exe`)
- For source builds: Rust via `rustup`, Visual Studio C++ Build Tools and the Windows SDK

The Rust toolchain is pinned in [rust-toolchain.toml](https://github.com/cetusk/GameServerManager/blob/main/rust-toolchain.toml). Linux supports the mock GUI and tests; real server management requires Windows.

## Build and run

### Release build

Obtain the [repository](https://github.com/cetusk/GameServerManager) and run this command in PowerShell from its root directory:

```powershell
.\build-release.bat
```

The script builds both the GUI and graceful-shutdown helper in release mode and prints their output paths on success. The following files are generated, relative to the repository root:

| File | Output path |
|---|---|
| Management application | `target\x86_64-pc-windows-msvc\release\manager-gui.exe` |
| Graceful-shutdown helper | `target\x86_64-pc-windows-msvc\release\gsm-ctrlc-helper.exe` |

### Run from a folder of your choice

Copy **both executables into the same folder**, for example `D:\Apps\GameServerManager`. Keep the graceful-shutdown helper alongside the application. Images are embedded in the executable, so the `assets` folder is not required at runtime. Rust is not required to run the built application.

Double-click `manager-gui.exe`. On first launch, use **Browse…** or enter an absolute folder path, then select **Open this directory**. Subsequent direct launches automatically use that location.

View or change it under **App settings → Data location**. This directory stores registrations, app preferences and operation records; game installations, worlds and backups have separate paths. Changing the directory reopens the manager with the selected folder's data. Original data is never moved or deleted. Stop all servers and finish or recover outstanding operations before switching.

The selected location is remembered in `%LOCALAPPDATA%\GameServerManager\data-location.json`. If the directory is unavailable, for example because a drive is disconnected, the app displays the directory chooser with an explanation.

An explicit command-line path is still supported. It overrides the remembered location without replacing that preference:

```powershell
.\manager-gui.exe --backend local --data-dir 'D:\GameServerManagerData'
```

Development launch scripts and `tools\start-packaged.ps1` specify their own directories, which take precedence over the saved GUI selection. For normal use, open the EXE directly.

### Development build from source

Run this command from the repository root:

```powershell
.\run-manager.ps1
```

This builds and launches the GUI and graceful-shutdown helper in the development profile. Management data is stored in `.manager-data/`. See the [development guide](docs/development.md) for details.

## First-time setup

1. Choose a management data folder on first launch, then select a game and open **Server settings**.
2. For a new installation, select **Create new configuration** and enter a new configuration file path, an empty installation folder, the SteamCMD path and other required fields. For an existing server, register a supported ServerMaintainer configuration (`Profile/*.ini` for ARK; `config.toml` for other games).
3. Review and create/register the configuration. Registration takes effect automatically.
4. For a new installation, use **Controls → Update / install**, then reload the manager and start the server.

Creation does not overwrite existing files. Some initial setup must be performed in the game, such as claiming a Satisfactory server and creating its first session. See the [server settings guide](docs/server-settings.md).

Change the interface language using **Language → 日本語 / English** in the bottom-left corner. Remembering the previous game and tab is disabled by default.

### Set up SteamCMD

SteamCMD is a separate tool from the Steam client; installing Steam does not install SteamCMD. One SteamCMD installation can serve multiple games, with a separate server installation folder for each game.

1. Open **App settings → SteamCMD**.
2. The suggested path is `C:\steamcmd\steamcmd.exe`. Use **Choose install folder** to select another writable location if needed.
3. For a new installation, choose an empty folder and select **Download and configure**. The app downloads the ZIP from Valve, extracts it and saves the shared path. Existing files are never overwritten.
4. If SteamCMD is already installed, use **Choose existing EXE → Save this path**.

New server configurations start with the saved shared path, or the suggested path if none has been saved. **Each server uses its own SteamCMD field; changing the shared default does not update existing server configurations.** Use **Use default path** beside a server's field to copy in the current default.

This downloads the SteamCMD bootstrap executable. SteamCMD initializes and updates itself when you first use **Update / install** for a game server. Use of the same SteamCMD installation is serialized. See the [SteamCMD guide](docs/steamcmd.md) for details and path references.

The manager verifies SteamCMD initialization and self-update before updating the game server. If an update fails, check the operation history and the SteamCMD update log in **Logs**. **Copy full log** includes the update log as well.

## Data handling

### Management data and game server configuration

`.manager-data/` is the management data directory used by `run-manager.ps1`. When opening the EXE directly, the folder selected during first-run setup or under **App settings → Data location** serves the same purpose. **It stores more than GUI preferences: it also contains server registrations and records needed to manage servers safely.**

| Contents | Location |
|---|---|
| Theme, language, notifications and navigation preferences | `local-preferences.json` in the management data directory |
| Shared SteamCMD path used to prefill new server configurations | `local-steamcmd.json` in the management data directory |
| Selected games and server metadata, including server and world names | `local-app.json` in the management data directory |
| Registered configuration file paths, identifiers and hashes for detecting changes | `local-registrations.json` in the management data directory |
| Started process identities, unfinished operations, and settings/restore recovery records | Under `instances/` in the management data directory |
| Actual settings, such as ports, passwords, SteamCMD paths and world parameters | The `config.toml` or ARK profile specified when creating/registering a configuration, and game-specific INI/JSON files |
| Game installations, worlds and backups | Their respective locations specified in Server settings |

Saving in **Server settings** or **World settings** updates the corresponding configuration files. Registration records do not duplicate configuration contents or passwords as normal settings. However, **while saving configuration changes, `instances/<server ID>/settings-undo.json` temporarily stores the full contents before and after the change, which may include passwords.** This record is removed after a successful save or recovery; interrupted operations can leave it in place for recovery.

Copying only the management data directory does not back up the actual game configuration or worlds. Deleting it as though it contained only GUI preferences also removes registrations and recovery records. To transfer appearance, notifications and navigation preferences, use export/import under **App settings → Transfer**. The shared SteamCMD path is machine-specific and is excluded from this export.

Backups and configuration changes require a stopped server. Changing a path does not move existing data. Closing the GUI does not automatically stop servers.

Game configurations, backups and recovery records in the management data directory may contain passwords. Remove credentials, personal information and personal paths before attaching files or logs to an issue.

## Documentation

The detailed guides are currently in Japanese.

- [User guide](docs/local-manager.md)
- [Interface and app preferences](docs/native-ui.md)
- [Server settings and new configuration creation](docs/server-settings.md)
- [Support status and limitations](docs/compatibility.md)
- [Development, builds and tests](docs/development.md)
- [Changelog](CHANGELOG.md)

## License and attribution

The project source code is published under the [MIT License](LICENSE). Existing copyright notices, dependency licenses and game artwork rights are documented in [third-party notices](THIRD_PARTY_NOTICES.md). Game names and artwork belong to their respective owners. This is an unofficial application.
