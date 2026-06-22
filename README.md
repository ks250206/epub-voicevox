# epub-voicevox

Rust 製の TUI EPUB リーダー。ターミナル上で EPUB を読み、VOICEVOX による音声読み上げと読み上げ位置のハイライトをサポートする。

## 機能

- **EPUB 閲覧** — 章・目次ナビゲーション、スクロール
- **リッチ表示** — 見出し、表（`│` 整形）、画像（ターミナルセルに合わせてスケール）
- **インライン装飾** — `strong` / `em` / `code` など XHTML タグを Span スタイルに変換
- **音声読み上げ** — VOICEVOX ENGINE 連携（チャンクのバックグラウンド合成・プリロード）
- **読み上げハイライト** — 非再生時は `j`/`k` で開始行（黄背景）を移動、再生中はチャンク行を強調
- **読み上げ調整** — 話者切替、速度変更（0.5〜2.0）

## 必要環境

- Rust（`cargo`）
- 音声読み上げを使う場合:
  - [VOICEVOX ENGINE](https://github.com/VOICEVOX/voicevox_engine)
  - `podman` または `docker`（コンテナ起動用、任意）
- `just`（タスクランナー、任意）

## クイックスタート

`just` を使う場合（推奨）:

```bash
# タスク一覧
just

# ビルド（debug）→ VOICEVOX 起動 → リーダー起動
just build
just up
just read path/to/book.epub

# 上をまとめて（コンテナ起動 + リーダー）
just start path/to/book.epub
```

リリースビルドで直接実行する場合:

```bash
cargo build --release
just up
./target/release/bk path/to/book.epub
```

`cargo run` で起動する場合:

```bash
just up
cargo run -- path/to/book.epub
```

VOICEVOX を手動で起動する場合（`just up` と同等）:

```bash
podman run --rm -p 127.0.0.1:50021:50021 voicevox/voicevox_engine:cpu-latest
```

## CLI オプション

`bk`（`just read` / `cargo run` 経由でも同じ）:

```bash
bk path/to/book.epub \
  --voicevox-url http://127.0.0.1:50021 \
  --speaker 1 \
  --speech-speed 1.0
```

`just read` は `--voicevox-url` を justfile の設定（既定: `http://127.0.0.1:50021`）で付与する。話者・速度は起動後に `]` / `-` / `=` で変更可能。

| オプション | 説明 | デフォルト |
|-----------|------|-----------|
| `path` | EPUB ファイルパス | — |
| `--voicevox-url` | VOICEVOX ENGINE の URL | `http://127.0.0.1:50021` |
| `--speaker` | 話者 ID（スタイル ID） | `1` |
| `--speech-speed` | 読み上げ速度（speedScale） | `1.0` |

## キーバインド

### 読書モード

| キー | 動作 |
|------|------|
| `j` / `↓` | 下へ（非再生時: 読み上げ開始行、再生中: スクロール） |
| `k` / `↑` | 上へ（非再生時: 読み上げ開始行、再生中: スクロール） |
| `Ctrl+d` | 半画面下 |
| `Ctrl+u` | 半画面上 |
| `PageDown` / `PageUp` | ページ送り |
| `n` / `→` | 次の章 |
| `p` / `←` | 前の章 |
| `g` / `G` | 章の先頭 / 末尾 |
| `t` | 目次を開く |
| `v` | 画面表示テキストを読み上げ |
| `r` | 現在位置から章末まで読み上げ |
| `s` | 読み上げ停止 |
| `[` / `]` | 話者を前 / 次 |
| `-` / `=` | 速度を下げる / 上げる |
| `q` / `Ctrl+c` | 終了 |

### 目次モード

| キー | 動作 |
|------|------|
| `↑` / `↓` / `j` / `k` | 選択移動 |
| `Enter` | 章へ移動 |
| `t` / `Esc` | 目次を閉じる |

## just タスク

`podman` があれば自動で使用し、なければ `docker` にフォールバックする。コンテナ名は `bk-voicevox`。

| コマンド | 別名 | 説明 |
|---------|------|------|
| `just` | — | タスク一覧（`just --list` と同じ） |
| `just up` | `voicevox-up` | VOICEVOX コンテナを起動（既に動いていればスキップ） |
| `just down` | `voicevox-down` | VOICEVOX コンテナを停止 |
| `just status` | `voicevox-status` | コンテナ状態を表示 |
| `just logs` | `voicevox-logs` | コンテナログを追跡（`-f`） |
| `just speakers` | — | 話者 API の疎通確認（先頭 2000 バイト） |
| `just build` | — | `cargo build`（**debug** ビルド） |
| `just test` | — | `cargo test` |
| `just read <path>` | — | `cargo run -- <path> --voicevox-url …` でリーダー起動 |
| `just start <path>` | — | `just up` のあと `just read <path>` |

パスに空白や記号があるときは引用する（例: `just read "My Book.epub"`）。

docker を明示的に使う場合:

```bash
just --set container docker up
```

リリースビルド用の just タスクはない。`cargo build --release` 後は `./target/release/bk` を直接実行する。

## アーキテクチャ

```
src/
├── main.rs    # CLI・イベントループ
├── book.rs    # EPUB ロード、ContentBlock、RichText
├── html.rs    # XHTML → 構造化ブロック（インライン装飾付き）
├── layout.rs  # レイアウト、SpeechPlan、ハイライト用メタデータ
├── ui.rs      # ratatui 描画
└── voice.rs   # VOICEVOX 合成・再生・プリロード
```

EPUB の XHTML はパース時に `ContentBlock`（段落・見出し・表・画像）へ変換し、レイアウト段階で折り返し行と読み上げチャンクの対応（`SpeechPlan`）を構築する。再生中はチャンク index から表示行を逆引きしてハイライトする。

## 開発

```bash
just test
cargo clippy -- -D warnings
cargo fmt
```

リリースビルドの確認:

```bash
cargo build --release
cargo build --profile dist   # より小さいバイナリ（任意）
```

## ライセンス

MIT License — Copyright (c) 2026 ks25026（[LICENSE](LICENSE)）
