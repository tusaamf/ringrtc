#!/bin/bash
set -e
docker compose down --rmi all
docker rmi ringrtc:2.60.1