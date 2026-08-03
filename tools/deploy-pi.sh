#!/bin/sh
set -eu

usage() {
    echo "Usage: $0 [user@]hostname" >&2
    echo "Build RustyTune for a 64-bit Pi with Docker and deploy it over SSH." >&2
    exit 2
}

[ "$#" -eq 1 ] || usage
ssh_target=$1

# Keep the target from being interpreted as an ssh option or remote shell text.
case "$ssh_target" in
    ""|-*|*[!A-Za-z0-9._@:%+-]*) usage ;;
esac

for command_name in docker ssh scp; do
    if ! command -v "$command_name" >/dev/null 2>&1; then
        echo "error: required command not found: $command_name" >&2
        exit 1
    fi
done

repo_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
artifact_dir=$(mktemp -d "${TMPDIR:-/tmp}/rustytune-pi.XXXXXX")
trap 'rm -rf "$artifact_dir"' EXIT HUP INT TERM
build_sha=$(git -C "$repo_dir" rev-parse --short HEAD 2>/dev/null || echo unknown)

echo "Building Linux ARM64 appliance binary at $build_sha in Docker..."
docker build \
    --platform linux/arm64 \
    --file "$repo_dir/appliance/Dockerfile" \
    --target artifact \
    --build-arg "BUILD_SHA=$build_sha" \
    --output "type=local,dest=$artifact_dir" \
    "$repo_dir"

artifact=$artifact_dir/rustytune
[ -f "$artifact" ] || {
    echo "error: Docker build did not produce $artifact" >&2
    exit 1
}
chmod 755 "$artifact"

remote_artifact=/tmp/rustytune.deploy
echo "Uploading binary to $ssh_target..."
scp "$artifact" "$ssh_target:$remote_artifact"

echo "Preflighting and installing binary (sudo may prompt on the Pi)..."
ssh -t "$ssh_target" '
    set -eu
    candidate=/tmp/rustytune.deploy
    installed=/usr/local/bin/rustytune
    backup=/usr/local/bin/rustytune.previous

    cleanup() { rm -f "$candidate"; }
    trap cleanup EXIT HUP INT TERM

    chmod 755 "$candidate"
    ldd "$candidate"
    "$candidate" --version

    sudo systemctl stop rustytune.service
    if [ -f "$installed" ]; then
        sudo cp -p "$installed" "$backup"
    fi
    sudo install -o root -g root -m 0755 "$candidate" "$installed"

    if sudo systemctl start rustytune.service &&
       sudo systemctl is-active --quiet rustytune.service; then
        sudo systemctl --no-pager --full status rustytune.service
    else
        echo "Deployment failed; restoring the previous binary..." >&2
        if [ -f "$backup" ]; then
            sudo install -o root -g root -m 0755 "$backup" "$installed"
            sudo systemctl restart rustytune.service
        fi
        exit 1
    fi
'

echo "RustyTune deployed successfully to $ssh_target."
