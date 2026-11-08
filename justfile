# Build, install and uninstall the dimmer applet beside stock COSMIC.
# Same conventions as cosmic-applets' justfile: `rootdir` stages into another
# root (packaging), `prefix` picks /usr or /usr/local.
#
#   just build-release
#   sudo just install          # or: just rootdir=/tmp/stage install
#   sudo just uninstall
#   just deb                   # .deb in target/debian (cargo-deb)
#
# Only installing needs root, to write system folders. The applet never runs
# as root and nothing is setuid (R4). Nothing here installs over, edits or
# removes a stock COSMIC file, and nothing touches modprobe.d or the
# initramfs: the optional NVIDIA step in docs/install.md is the user's to make.

name := 'cosmic-ext-dimmer'
appid := 'io.github.deepthinker2001.CosmicExtDimmer'
crate := 'crates' / name

rootdir := ''
prefix := '/usr'

base-dir := absolute_path(clean(rootdir / prefix))
bin-dst := base-dir / 'bin' / name
desktop-dst := base-dir / 'share' / 'applications' / appid + '.desktop'
icons-dst := base-dir / 'share' / 'icons'
doc-dst := base-dir / 'share' / 'doc' / name

# udev and modules-load.d don't look under /usr/local, so a source install
# goes to the administrator's folders in /etc whatever the prefix. (The .deb
# uses /usr/lib instead.)
udev-dst := absolute_path(clean(rootdir / 'etc' / 'udev' / 'rules.d')) / '60-' + name + '-i2c.rules'
modules-dst := absolute_path(clean(rootdir / 'etc' / 'modules-load.d')) / name + '.conf'

target-dir := env('CARGO_TARGET_DIR', justfile_directory() / 'target')
bin-src := target-dir / 'release' / name
desktop-src := crate / 'data' / appid + '.desktop'
icons-src := crate / 'data' / 'icons'

default: build-release

build-debug *args:
    cargo build --locked -p {{name}} {{args}}

build-release *args:
    cargo build --locked --release -p {{name}} {{args}}

# The checks CI runs.
check:
    ./scripts/check.sh

clean:
    cargo clean

# Build the .deb (needs cargo-deb: `cargo install cargo-deb`).
deb *args:
    cargo deb --locked -p {{name}} {{args}}

# Install the built applet, desktop entry and udev rule (after build-release).
install:
    #!/bin/sh
    set -eu
    if [ ! -x '{{bin-src}}' ]; then
        echo "{{bin-src}} not found: run 'just build-release' first (as yourself, not root)." >&2
        exit 1
    fi
    # Never replace a file a package owns (the .deb, or anything of COSMIC's).
    if [ -z '{{rootdir}}' ] && command -v dpkg-query >/dev/null 2>&1; then
        for f in '{{bin-dst}}' '{{desktop-dst}}'; do
            if owner=$(dpkg-query -S "$f" 2>/dev/null); then
                echo "Refusing to overwrite $f, which belongs to a package (${owner%%:*})." >&2
                echo "Remove that package first, or install with another prefix." >&2
                exit 1
            fi
        done
    fi
    install -Dm0755 '{{bin-src}}' '{{bin-dst}}'
    install -Dm0644 '{{desktop-src}}' '{{desktop-dst}}'
    install -Dm0644 packaging/udev/60-cosmic-ext-dimmer-i2c.rules '{{udev-dst}}'
    install -Dm0644 packaging/modules-load/cosmic-ext-dimmer.conf '{{modules-dst}}'
    install -Dm0644 README.md '{{doc-dst}}/README.md'
    install -Dm0644 docs/install.md '{{doc-dst}}/install.md'
    install -Dm0644 LICENSE '{{doc-dst}}/LICENSE'
    if [ -d '{{icons-src}}' ]; then
        (cd '{{icons-src}}' && find . -type f) | while read -r icon; do
            install -Dm0644 "{{icons-src}}/$icon" "{{icons-dst}}/$icon"
        done
        if [ -z '{{rootdir}}' ]; then
            gtk-update-icon-cache -q -t '{{icons-dst}}/hicolor' 2>/dev/null || true
        fi
    fi
    # On the real system (not a staging root): make the rule take effect now.
    # The shared i2c group is created if missing, but nobody is added to it.
    if [ -z '{{rootdir}}' ]; then
        getent group i2c >/dev/null || groupadd --system i2c
        modprobe i2c-dev >/dev/null 2>&1 || true
        if command -v udevadm >/dev/null 2>&1; then
            udevadm control --reload-rules || true
            udevadm trigger --subsystem-match=i2c-dev --action=change || true
        fi
        echo "Installed. For hardware dimming, join the i2c group and log in again:"
        echo "  sudo usermod -aG i2c \"\$USER\""
    fi

# Remove everything `install` placed. Settings in ~/.config stay (see README).
uninstall:
    #!/bin/sh
    set -eu
    if [ -z '{{rootdir}}' ] && command -v dpkg-query >/dev/null 2>&1; then
        if owner=$(dpkg-query -S '{{bin-dst}}' 2>/dev/null); then
            echo "{{bin-dst}} belongs to the package ${owner%%:*}: remove it with apt instead." >&2
            exit 1
        fi
    fi
    rm -f '{{bin-dst}}' '{{desktop-dst}}' '{{udev-dst}}' '{{modules-dst}}'
    rm -rf '{{doc-dst}}'
    if [ -d '{{icons-src}}' ]; then
        (cd '{{icons-src}}' && find . -type f) | while read -r icon; do
            rm -f "{{icons-dst}}/$icon"
        done
        if [ -z '{{rootdir}}' ]; then
            gtk-update-icon-cache -q -t '{{icons-dst}}/hicolor' 2>/dev/null || true
        fi
    fi
    if [ -z '{{rootdir}}' ] && command -v udevadm >/dev/null 2>&1; then
        udevadm control --reload-rules || true
    fi
