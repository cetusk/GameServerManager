# 開発と検証

Rust 1.98.1 と Slint 1.18.0 を固定し、Cargo.lock を管理します。主対象は Windows x64 + Visual Studio C++ Build Tools。共通層とゲーム別 crate の依存方向を `tools/check-boundaries.py` で検査します。

## 起動モード

```powershell
.\run-dev.ps1                         # 模擬モード、.dev-data/default
.\run-manager.ps1                     # 実管理モード、.manager-data
.\run-manager.ps1 -DataDir 'D:\GSM\data'
```

`run-manager.ps1` は GUI と `gsm-ctrlc-helper.exe` を同時にビルドします。直接起動は `manager-gui.exe --data-dir <絶対パス> --backend local`。`--data-dir` を省略・相対指定するとエラーです。backend 省略時は従来互換の mock とし、実管理は明示的に選びます。

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

## 配布用 ZIP

```powershell
.\tools\build-windows.ps1
```

`dist/GameServerManager-v0.1.0-windows-x64.zip` にGUI、helper、`start-manager.ps1`、日英README、操作ガイド、ライセンスと出典表示を含めます。既存の配布フォルダーにある利用者の data は ZIP に含めません。SQLite は同梱ビルド、Windows の HTTPS は Schannel を使用します。

[Windows CI](https://github.com/cetusk/GameServerManager/blob/main/.github/workflows/check.yml) は全体テスト・単独 feature・mock/local 空登録の GUI 起動・ZIP 作成を定義しています。CI 定義の追加と CI が実際に成功したことは区別してください。

## 実装状況

5 ゲームの設定登録と local adapter、実プロセス識別・再接続・正常停止、停止中の実バックアップ／復元と回復、更新、ログ、退避後の設定編集を実装しています。UI のモデルを差分更新し、監視・IO は画面スレッドの外で実行します。

5ゲームの基本設定をGUI内で編集し、変更内容を確認してバックアップ後に保存できます。Valheimはワールドパラメーターの編集にも対応します。詳細は [画面の説明](native-ui.md) と [設定ガイド](server-settings.md) を参照してください。現在の実機確認と未対応項目は [対応状況](compatibility.md) にまとめています。

## リポジトリに含めないデータ

実際のゲーム設定・管理データ・ログ・ワールド・バックアップ、原本アーカイブ、分析資料、デザイン検討用ファイル、ローカルのエージェント指示はコミットしません。テスト用設定は人工データに限定します。`.gitignore` は既に追跡されているファイルや過去の履歴を消すものではありません。
