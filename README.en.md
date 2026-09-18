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
- An existing `steamcmd.exe` for updates and installation
- For source builds: Rust via `rustup`, Visual Studio C++ Build Tools and the Windows SDK

The Rust toolchain is pinned in [rust-toolchain.toml](https://github.com/cetusk/GameServerManager/blob/main/rust-toolchain.toml). Linux supports the mock GUI and tests; real server management requires Windows.

## Build and run

### Release build

Obtain the [repository](https://github.com/cetusk/GameServerManager) and run this command in PowerShell from its root directory:

```powershell
cargo build --release --locked -p manager-gui -p gsm-ctrlc-helper --all-features --target x86_64-pc-windows-msvc --target-dir target
```

The following files are generated, relative to the repository root:

| File | Output path |
|---|---|
| Management application | `target\x86_64-pc-windows-msvc\release\manager-gui.exe` |
| Graceful-shutdown helper | `target\x86_64-pc-windows-msvc\release\gsm-ctrlc-helper.exe` |

### Run from a folder of your choice

Copy **both executables into the same folder**, for example `D:\Apps\GameServerManager`. Keep the graceful-shutdown helper alongside the application. Images are embedded in the executable, so the `assets` folder is not required at runtime. Rust is not required to run the built application.

Open PowerShell in that folder and specify an **absolute path** for management data:

```powershell
.\manager-gui.exe --backend local --data-dir 'D:\GameServerManagerData'
```

`--data-dir` stores registrations, app preferences and operation records. Game installation and world save paths are configured separately in the GUI. Use the same management data path on subsequent launches to retain your setup.

For a launch script, copy `tools\start-packaged.ps1` from the source repository into the executable folder as `start-manager.ps1`, then run `.\start-manager.ps1`. Its default management data location is the `data` subfolder beside the executables.

### Development build from source

Run this command from the repository root:

```powershell
.\run-manager.ps1
```

This builds and launches the GUI and graceful-shutdown helper in the development profile. Management data is stored in `.manager-data/`. See the [development guide](docs/development.md) for details.

## First-time setup

1. Select a game and open **Server settings**.
2. For a new installation, select **Create new configuration** and enter a new configuration file path, an empty installation folder, the SteamCMD path and other required fields. For an existing server, register a supported ServerMaintainer configuration (`Profile/*.ini` for ARK; `config.toml` for other games).
3. Review and create/register the configuration. Registration takes effect automatically.
4. For a new installation, use **Controls → Update / install**, then reload the manager and start the server.

Creation does not overwrite existing files. Some initial setup must be performed in the game, such as claiming a Satisfactory server and creating its first session. See the [server settings guide](docs/server-settings.md).

Change the interface language using **Language → 日本語 / English** in the bottom-left corner. Remembering the previous game and tab is disabled by default.

## Data handling

Backups and configuration changes require a stopped server. Changing a path does not move existing data. Closing the GUI does not automatically stop servers.

Game configurations and backups may contain passwords. Remove credentials, personal information and personal paths before attaching files or logs to an issue.

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
