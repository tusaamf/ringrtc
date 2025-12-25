#!/bin/bash

# Dừng script ngay lập tức nếu có lệnh bị lỗi
set -e

docker compose up -d --build
docker exec -it ringrtc bash -c "
    set -e
    cargo build -p ringrtc --features prebuilt_webrtc
    make android
"


echo "=== HOÀN TẤT! ==="
echo "File AAR của bạn nằm tại thư mục: out/"

bin/build-aar -a arm64 -d --webrtc-only --archive-webrtc
bin/build-aar -a arm64 -d -r --webrtc-only --archive-webrtc

bin/build-aar -a arm64 -d --ringrtc-only
bin/build-aar -a arm64 -d -r --ringrtc-only

docker exec -it ringrtc bash -c "
    bin/build-aar -a arm64 -d --webrtc-only --archive-webrtc
    bin/build-aar -a arm64 -d --ringrtc-only
"

docker exec -it ringrtc bash -c "
    bin/build-aar -a arm64 -d --ringrtc-only
"