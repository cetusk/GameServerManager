# 開発と検証

Rust 1.98.1 と Slint 1.18.0 を固定し、Cargo.lock を管理します。主対象は Windows x64 + Visual Studio C++ Build Tools。共通層とゲーム別 crate の依存方向を `tools/check-boundaries.py` で検査します。

## 起動モード

```powershell
.\run-dev.ps1                         # 模擬モード、.dev-data/default
.\run-manager.ps1                     # 実管理モード、.manager-data
.\run-manager.ps1 -DataDir 'D:\GSM\data'
```

`run-manager.ps1` は GUI と `gsm-ctrlc-helper.exe` を同時にビルドします。直接起動は `manager-gui.exe --data-dir <絶対パス> --backend local`。引数なしのexe起動はlocalモードで、保存先を記憶していなければ初回設定画面を開きます。`--backend mock` と `--smoke-test` は明示的な絶対パスの `--data-dir` が必須です。`--data-dir` 指定時のbackend省略は従来互換のmockです。明示パスは記憶済みの保存先より優先され、記憶自体は変更しません。

mock と local の選択保存は `mock-app.json` / `local-app.json` に分離します。`local-registrations.json` は設定の参照先・ハッシュ・ID の登録です。実管理ではインストール先のファイルロックと Windows ユーザー単位のロックも取ります。実ファイルは [実管理ガイド](local-manager.md) に記載した範囲でのみ操作します。

## 必須チェック

```powershell
cargo fmt --all -- --check
python tools/check-boundaries.py
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
foreach ($feature in @('arksa','valheim','windrose','satisfactory','conan')) {
    cargo check -p manager-gui --no-default-features --features $feature --locked
    if ($LASTEXITCODE -ne 0) { throw "Failed: $feature" }
}
cargo build -p manager-gui -p gsm-ctrlc-helper --all-features --locked
.\run-dev.ps1 -DataDir 'C:\GsmDev\mock-smoke' -SmokeTest
.\run-manager.ps1 -DataDir 'C:\GsmDev\local-empty-smoke' -SmokeTest
```

local の smoke test は空の専用 data-dir で実行します。登録を持つ data-dir では、起動時の設定読み込み・ロック・状態確認まで行いますが、smoke test 自体はサーバー操作を送信しません。

Linux では mock GUI と人工ファイルのテストができます。X11 の例:

```bash
env -u WAYLAND_DISPLAY xvfb-run -a -s '-screen 0 1360x1000x24' \
  cargo run --locked -p manager-gui -- --backend mock --data-dir /tmp/gsm-preview --smoke-test
```

ビルド出力を別の場所へ置く場合は `CARGO_TARGET_DIR` を指定できます。Windows向けの検証はWindows上で行います。Linuxでの型検査・模擬GUIの成功は、Windowsのリンクや実ゲーム動作を保証しません。

## リリースビルド

WindowsのPowerShellまたはコマンドプロンプトから実行します。

```powershell
.\build-release.bat
```

GUIと正常停止用ヘルパーを `target\x86_64-pc-windows-msvc\release\` に生成します。スクリプト自身のフォルダーを基準にビルドするため、別の作業フォルダーから実行しても生成先は変わりません。失敗時はエラーコードを返します。

## 配布用 ZIP

```powershell
.\tools\build-windows.ps1
```

`dist/GameServerManager-v0.1.0-windows-x64.zip` にGUI、helper、`start-manager.ps1`、日英README、操作ガイド、ライセンスと出典表示を含めます。既存の配布フォルダーにある利用者の data は ZIP に含めません。SQLite は同梱ビルド、Windows の HTTPS は Schannel を使用します。

配布スクリプトも `build-release.bat` を使用してビルドします。

[Windows CI](https://github.com/cetusk/GameServerManager/blob/main/.github/workflows/check.yml) は全体テスト・単独 feature・mock/local 空登録の GUI 起動・ZIP 作成を定義しています。CI 定義の追加と CI が実際に成功したことは区別してください。

## 実装状況

5 ゲームの設定登録と local adapter、実プロセス識別・再接続・正常停止、停止中の実バックアップ／復元と回復、更新、ログ、退避後の設定編集を実装しています。UI のモデルを差分更新し、監視・IO は画面スレッドの外で実行します。

5ゲームの基本設定をGUI内で編集し、変更内容を確認してバックアップ後に保存できます。Valheimはワールドパラメーターの編集にも対応します。詳細は [画面の説明](native-ui.md) と [設定ガイド](server-settings.md) を参照してください。現在の実機確認と未対応項目は [対応状況](compatibility.md) にまとめています。

## リポジトリに含めないデータ

実際のゲーム設定・管理データ・ログ・ワールド・バックアップ、原本アーカイブ、分析資料、デザイン検討用ファイル、ローカルのエージェント指示はコミットしません。テスト用設定は人工データに限定します。`.gitignore` は既に追跡されているファイルや過去の履歴を消すものではありません。

## 通常のGit作業

リポジトリのルートで編集・検証・コミット・pushします。公開用ソースを `dist/` へコピーする工程はありません。`dist/` は配布ZIPなどの生成物の置き場です。

`.git/` はGit自身が使う履歴・設定の保存先で、通常の `git add` やpushのファイル対象には入りません。`.gitignore` への追加は不要です。`.github/` はGitHub Actionsのテスト・ビルド設定であり、ソースと一緒に追跡します。

`.gitignore` は未追跡ファイルを対象から外す設定です。既にコミットしたファイルや過去の履歴から情報を削除する機能ではありません。

### 管理データ保存先の起動経路

引数なし/local起動の保存先記録はWindowsで`%LOCALAPPDATA%/GameServerManager/data-location.json`、LinuxのUI確認では`$XDG_CONFIG_HOME/GameServerManager/data-location.json`（未指定時は`$HOME/.config/`以下）です。mockはこの記録を読み書きしません。初回画面の確認は、テスト専用ユーザー設定ディレクトリを使用してください。

保存先の選択・書き込み確認・既存設定の検証・記憶の保存はワーカーで行います。切り替え時はサーバー停止・ジョブ完了をGUIで確認し、元のセッションを終了・flushしてロックを解放した後に選択画面を開きます。選択の適用時に元の保存先ロックを取得し直し、未登録分を含む`instances`内のプロセス／操作／回復記録も確認します。新しい保存先はロック・管理設定・登録・アプリ設定・書き込み可能性を検証してから記憶します。キャンセルは元の保存先を再表示します。データのコピー・移動は行いません。

### SteamCMDのダウンロード検証

通常のworkspaceテストは人工ZIPと一時フォルダーを使い、ネットワークへ接続しません。Valve配布ZIPの取得・展開を確認する場合は、以下の任意テストを実行します。テストは一時フォルダーだけを使用し、取得したexeを実行しません。

```powershell
cargo test -p gsm-infra --locked downloads_official_bootstrap -- --ignored
```

Windowsの実機では、アプリ設定での保存先変更・既存exeの選択・ダウンロード進捗・再起動後の共通パス保持・5ゲームの新規作成時の初期入力・個別パスでの更新を確認します。模擬GUIではダウンロードと設定保存を無効化しています。
