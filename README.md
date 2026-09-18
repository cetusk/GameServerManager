<p align="center">
  <img src="assets/logo-dark-trimmed.png" alt="GameServerManager" width="360">
</p>

# GameServerManager: Unified Game Server Management Tool

**日本語** | [English](README.en.md)

Windows上のゲームサーバーを、ひとつのGUIから管理するアプリです。使用するゲームを選び、同じPC内にあるサーバーの起動・停止・設定・バックアップを操作できます。

今後も継続的に、ほかのゲームのサーバー管理機能を追加していく予定です。

## 主な機能

- ARK: Survival Ascended / Valheim / Windrose / Satisfactory / Conan Exiles に対応
- サーバーの起動・正常停止、SteamCMDによる更新・インストール
- 停止中のワールドと設定のバックアップ・復元
- 設定ファイルの新規作成・登録、ゲームごとの設定をGUIで編集
- サーバーログの表示・全文コピー、処理状況と作業履歴の表示
- ダークテーマ3種類、日本語・英語の切り替え

現状，Valheimでは実機での起動・参加・正常停止・バックアップ作成を確認しています。ほかのゲームの実機操作や復元、新規作成からの一連の動作は未確認です。[対応状況と制約](docs/compatibility.md)をご確認ください。

## 動作環境

- Windows x64（同じPC内のサーバー管理）
- 各ゲームの専用サーバーの動作要件を満たすPC
- 更新・インストールには、用意済みの `steamcmd.exe`
- ソースからのビルドには、Rust（`rustup`）と Visual Studio C++ Build Tools / Windows SDK

Rustのバージョンは [rust-toolchain.toml](https://github.com/cetusk/GameServerManager/blob/main/rust-toolchain.toml) で固定しています。Linuxでは模擬GUIとテストを実行できますが、実サーバー管理はWindows専用です。

## ビルドと起動

### リリースビルド

[リポジトリ](https://github.com/cetusk/GameServerManager)を取得し、ルートフォルダーのPowerShellで実行します。

```powershell
.\build-release.bat
```

GUIと正常停止用ヘルパーをまとめてリリースビルドし、完了時に生成先を表示します。生成先はリポジトリのルートを基準に、次のとおりです。

| ファイル | 生成パス |
|---|---|
| 管理アプリ | `target\x86_64-pc-windows-msvc\release\manager-gui.exe` |
| 正常停止用ヘルパー | `target\x86_64-pc-windows-msvc\release\gsm-ctrlc-helper.exe` |

### 任意のフォルダーに配置して起動

生成した**2つのexeを同じフォルダー**へコピーします。配置先は任意です（例: `D:\Apps\GameServerManager`）。正常停止用ヘルパーもアプリと一緒に配置してください。画像はexeに埋め込まれているため、実行時に`assets`フォルダーをコピーする必要はありません。ビルド済みアプリの実行にRustは不要です。

配置先のPowerShellで、管理データの保存先を**絶対パス**で指定して起動します。

```powershell
.\manager-gui.exe --backend local --data-dir 'D:\GameServerManagerData'
```

`--data-dir`は登録情報・アプリ設定・処理記録の保存先です。ゲーム本体やワールドの保存先はGUIで別途設定します。次回以降も同じ管理データを使う場合は、同じパスを指定してください。

現在、管理データの保存先をアプリ内で変更する機能はありません。「アプリ設定 → アプリ情報」で現在の保存先を確認できます。

起動用スクリプトを使う場合は、ソースの`tools\start-packaged.ps1`をexeと同じフォルダーへ`start-manager.ps1`という名前でコピーし、`.\start-manager.ps1`を実行できます。既定の管理データ保存先は、そのフォルダー内の`data`です。

### ソースから開発用ビルドで起動

リポジトリのルートで実行します。

```powershell
.\run-manager.ps1
```

GUIと正常停止用ヘルパーを開発用ビルドで起動します。管理データは `.manager-data/` に保存されます。詳細は[開発ガイド](docs/development.md)を参照してください。

## 最初の設定

1. 管理するゲームを選び、「サーバー設定」を開きます。
2. 新しく始める場合は「新規作成」を選び、設定ファイルの保存先・空の設置先・SteamCMDなどを入力します。既存環境は「設定ファイルの登録」から、対応するServerMaintainerの設定を選びます（ARK: `Profile/*.ini`、ほかのゲーム: `config.toml`）。
3. 内容を確認して作成・登録します。登録内容は自動で反映されます。
4. 新規環境は「サーバー操作 → 更新・インストール」を実行し、管理画面を再読み込みして起動します。

新規作成は既存ファイルを上書きしません。Satisfactoryのclaim・初回セッション作成など、ゲーム内で行う初期設定もあります。詳細は[サーバー設定ガイド](docs/server-settings.md)を参照してください。

言語は画面左下の **Language → 日本語 / English** で切り替えられます。画面を記憶する設定は初期値オフです。

## データの取り扱い

バックアップと設定変更はサーバー停止中に行います。パスの変更では既存データを自動移動しません。GUIを終了してもサーバーは自動停止しません。

ゲーム設定やバックアップにはパスワードなどが含まれます。ログや設定をIssueへ添付する際は、認証情報・個人情報・個人のパスを取り除いてください。

## ドキュメント

- [操作ガイド](docs/local-manager.md)
- [画面・アプリ設定](docs/native-ui.md)
- [サーバー設定・新規作成](docs/server-settings.md)
- [対応状況と制約](docs/compatibility.md)
- [開発・ビルド・テスト](docs/development.md)
- [変更履歴](CHANGELOG.md)

## ライセンス・出典

本プロジェクトのソースコードは [MIT License](LICENSE) で公開します。既存の著作権表示、依存ライブラリのライセンス、ゲーム画像の権利は[第三者の権利表記](THIRD_PARTY_NOTICES.md)を参照してください。ゲーム名・画像の権利は各権利者に帰属します。本アプリは各ゲームの公式ツールではありません。
