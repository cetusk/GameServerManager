# Third-party notices

GameServerManagerのソースコードはルートの [MIT License](LICENSE) で公開します。第三者のライブラリやゲーム画像は、それぞれのライセンス・権利に従います。

## ServerMaintainer由来のコード

Valheim / Windrose / Conan Exiles / ARK SAの保存レイアウト・設定・起動処理は、既存のServerMaintainerの仕様とMITソースを参照して移行しています。Copyright (c) 2026 cetusk。著作権表示は各 `crates/games/<game>/LICENSE` に保持しています。

Satisfactoryは設定形式・API・パスの調査に基づくアダプターです。元プロジェクトのソースモジュールはコピーしていません。

## Slint

Slint 1.18.0は独立したライセンスを持ちます。このWindowsデスクトップアプリでは **Slint Royalty-free Desktop, Mobile, and Web Applications License 2.0** の条件を使用し、「アプリ設定 → アプリ情報」に標準の `AboutSlint` ウィジェットを表示します。

- [同梱ライセンス本文](licenses/Slint-Royalty-free-2.0.md)
- [Slintライセンス案内](https://slint.dev/license/)

本プロジェクトのMITライセンスはSlint自体のライセンスを置き換えません。

## ゲームアイコン

出典とハッシュは [ゲームアイコンの出典](apps/manager-gui/assets/game-icons/SOURCES.md) に記載しています。各ゲームの画像・名称・商標の権利はそれぞれの権利者に帰属します。ゲーム画像に本プロジェクトのMITライセンスを適用しません。

## その他の依存ライブラリ

正確なバージョンは [Cargo.lock](Cargo.lock) に記録しています。各ライブラリの著作権表示とライセンスはそのパッケージに従います。依存関係は `cargo metadata --locked --format-version 1` で確認できます。

Windows配布物には本ファイル、ルートLICENSE、Slintライセンス、移行元のMIT著作権表示、ゲームアイコンの出典を含めます。
