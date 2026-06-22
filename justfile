# epub-voicevox — EPUB リーダー + VOICEVOX

set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

voicevox_image := "voicevox/voicevox_engine:cpu-latest"
voicevox_name := "bk-voicevox"
voicevox_port := "50021"
voicevox_url := "http://127.0.0.1:50021"

# podman があれば podman、なければ docker
container := `if command -v podman >/dev/null 2>&1; then echo podman; else echo docker; fi`

default:
    @just --list

# VOICEVOX ENGINE コンテナを起動（既に動いていれば何もしない）
up voicevox-up:
    #!/usr/bin/env bash
    if {{container}} ps --format '{{{{.Names}}}}' 2>/dev/null | grep -qx '{{voicevox_name}}'; then
        echo "{{voicevox_name}} は既に起動しています ({{voicevox_url}})"
    else
        echo "{{container}} で VOICEVOX を起動します..."
        {{container}} run -d --rm \
            --name {{voicevox_name}} \
            -p "127.0.0.1:{{voicevox_port}}:50021" \
            {{voicevox_image}}
        echo "VOICEVOX: {{voicevox_url}}"
    fi

# VOICEVOX ENGINE コンテナを停止
down voicevox-down:
    #!/usr/bin/env bash
    if {{container}} ps --format '{{{{.Names}}}}' 2>/dev/null | grep -qx '{{voicevox_name}}'; then
        {{container}} stop {{voicevox_name}}
        echo "{{voicevox_name}} を停止しました"
    else
        echo "{{voicevox_name}} は起動していません"
    fi

# コンテナの状態を表示
status voicevox-status:
    {{container}} ps --filter "name={{voicevox_name}}" --format "table {{{{.Names}}}}\t{{{{.Status}}}}\t{{{{.Ports}}}}"

# コンテナログを追跡
logs voicevox-logs:
    {{container}} logs -f {{voicevox_name}}

# VOICEVOX の話者一覧を確認
speakers:
    curl -fsS "{{voicevox_url}}/speakers" | head -c 2000
    echo

# Rust ビルド
build:
    cargo build

# テスト
test:
    cargo test

# EPUB リーダーを起動（例: just read book.epub）
read book:
    cargo run -- "{{book}}" --voicevox-url {{voicevox_url}}

# コンテナ起動 → リーダー起動
start book:
    just up
    just read "{{book}}"
