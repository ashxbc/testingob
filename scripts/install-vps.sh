#!/usr/bin/env bash
# One-shot VPS bootstrap. Tested on Ubuntu 22.04 / Debian 12.
# Usage: sudo bash install-vps.sh your.domain.tld
set -euo pipefail

DOMAIN="${1:-}"
if [[ -z "$DOMAIN" ]]; then
  echo "usage: $0 <domain>"
  exit 1
fi

REPO_DIR="${REPO_DIR:-/opt/liquidity-src}"
INSTALL_DIR="/opt/liquidity"

apt-get update
apt-get install -y curl build-essential pkg-config libssl-dev redis-server nginx certbot python3-certbot-nginx git

# Rust toolchain (system-wide)
if ! command -v cargo >/dev/null; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
  source "$HOME/.cargo/env"
fi

# Node 20
if ! command -v node >/dev/null; then
  curl -fsSL https://deb.nodesource.com/setup_20.x | bash -
  apt-get install -y nodejs
fi

# User
id -u lv >/dev/null 2>&1 || useradd -r -s /usr/sbin/nologin lv
mkdir -p "$INSTALL_DIR"/{bin,web,data}
chown -R lv:lv "$INSTALL_DIR"

# Build
cd "$REPO_DIR"
cargo build --release
install -m 755 target/release/ingestor "$INSTALL_DIR/bin/"
install -m 755 target/release/analyzer "$INSTALL_DIR/bin/"
install -m 755 target/release/api "$INSTALL_DIR/bin/"
install -m 644 config.toml "$INSTALL_DIR/"

cd web
npm ci
npm run build
cp -r .next/standalone/. "$INSTALL_DIR/web/"
mkdir -p "$INSTALL_DIR/web/.next"
cp -r .next/static "$INSTALL_DIR/web/.next/"
[[ -d public ]] && cp -r public "$INSTALL_DIR/web/" || true

chown -R lv:lv "$INSTALL_DIR"

# systemd
cp "$REPO_DIR"/deploy/systemd/*.service /etc/systemd/system/
systemctl daemon-reload
systemctl enable --now redis-server lv-ingestor lv-analyzer lv-api lv-web

# nginx
sed "s/your.domain.tld/$DOMAIN/g" "$REPO_DIR/deploy/nginx.conf" > /etc/nginx/sites-available/liquidity
ln -sf /etc/nginx/sites-available/liquidity /etc/nginx/sites-enabled/liquidity
rm -f /etc/nginx/sites-enabled/default
nginx -t && systemctl reload nginx

# TLS
certbot --nginx -d "$DOMAIN" --non-interactive --agree-tos --register-unsafely-without-email || true

echo
echo "Done. Visit https://$DOMAIN"
echo "Logs: journalctl -u lv-ingestor -u lv-analyzer -u lv-api -u lv-web -f"
