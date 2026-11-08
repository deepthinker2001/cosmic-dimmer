#!/bin/sh
# Rebuild the signed apt repository that GitHub Pages serves.
#
# Usage: scripts/publish-apt.sh <site-dir> <key-id> [new .deb files...]
#
# <site-dir> is a checkout of the gh-pages branch. New .deb files are added
# to its pool (earlier ones stay, so apt can still find them); the package
# index and the signed Release files are then generated again from the whole
# pool. Signing uses the key <key-id> from the keyring in $GNUPGHOME.
# Needs apt-ftparchive (apt-utils) and gpg.
set -eu

if [ $# -lt 2 ]; then
    echo "usage: $0 <site-dir> <key-id> [new .deb files...]" >&2
    exit 2
fi
site=$1
key=$2
shift 2

pool=pool/main/c/cosmic-ext-dimmer
dist=dists/stable
mkdir -p "$site/$pool" "$site/$dist/main/binary-amd64"
for deb in "$@"; do
    cp "$deb" "$site/$pool/"
done

cd "$site"
apt-ftparchive packages pool > "$dist/main/binary-amd64/Packages"
gzip -9 -n -k -f "$dist/main/binary-amd64/Packages"

# Written beside the dists tree first: apt-ftparchive would otherwise list
# the half-written Release file in itself.
apt-ftparchive \
    -o APT::FTPArchive::Release::Origin=deepthinker2001 \
    -o APT::FTPArchive::Release::Label="Dimmer for COSMIC" \
    -o APT::FTPArchive::Release::Suite=stable \
    -o APT::FTPArchive::Release::Codename=stable \
    -o APT::FTPArchive::Release::Architectures=amd64 \
    -o APT::FTPArchive::Release::Components=main \
    -o APT::FTPArchive::Release::Description="Dimmer for COSMIC: an unofficial per-display dimming applet" \
    release "$dist" > Release.new
mv Release.new "$dist/Release"

gpg --batch --yes --local-user "$key" --clearsign -o "$dist/InRelease" "$dist/Release"
gpg --batch --yes --local-user "$key" --armor --detach-sign -o "$dist/Release.gpg" "$dist/Release"
gpg --batch --yes --armor --export "$key" > key.gpg

# Serve the files as they are, without GitHub Pages' Jekyll processing.
touch .nojekyll
echo "Repository in $site rebuilt and signed with $key."
