# epub-voicevox

Rust 製の TUI EPUB リーダー（crates.io パッケージ名: `epub-voicevox`、コマンド名: `bk`）。ターミナル上で EPUB を読み、VOICEVOX による音声読み上げと読み上げ位置のハイライトをサポートする。

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

```bash
# ビルド（リポジトリ内）
cargo build --release

# crates.io から（公開後）
cargo install epub-voicevox
```

# VOICEVOX をコンテナで起動（just 使用）
just up

# リーダー起動
cargo run -- book.epub

# まとめて
just start book.epub
```

VOICEVOX を手動で起動する場合:

```bash
podman run --rm -p 127.0.0.1:50021:50021 voicevox/voicevox_engine:cpu-latest
```

## CLI オプション

```bash
cargo run -- book.epub \
  --voicevox-url http://127.0.0.1:50021 \
  --speaker 1 \
  --speech-speed 1.0
```

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

`podman` があれば自動で使用し、なければ `docker` にフォールバックする。

| コマンド | 説明 |
|---------|------|
| `just up` | VOICEVOX コンテナを起動 |
| `just down` | VOICEVOX コンテナを停止 |
| `just status` | コンテナ状態を表示 |
| `just logs` | コンテナログを追跡 |
| `just speakers` | 話者 API の疎通確認 |
| `just read book.epub` | リーダーを起動 |
| `just start book.epub` | コンテナ起動 → リーダー起動 |
| `just build` | `cargo build` |
| `just test` | `cargo test` |

docker を明示的に使う場合:

```bash
just --set container docker up
```

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
cargo test
cargo clippy
cargo fmt
```

## ライセンス

MIT License — Copyright (c) 2026 ks25026（[LICENSE](LICENSE)）
