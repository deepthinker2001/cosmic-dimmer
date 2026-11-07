#!/usr/bin/env python3
"""Show which /dev/i2c-* nodes a udev rules file would match, without installing it.

`udevadm test` only reads the installed rules directories, so this evaluates
the file's own lines against sysfs with udev's matching semantics:
SUBSYSTEM/KERNEL/ATTR test the device itself, and the SUBSYSTEMS/KERNELS/ATTRS
keys of one rule must all match on the same device, the device itself or one
of its parents. Values are shell globs (fnmatch), as in udev. Only the keys
this project's rule uses are supported; any other key is an error rather than
a silent pass.

    python3 -I packaging/udev/match-report.py packaging/udev/60-cosmic-dimmer-i2c.rules
"""

import fnmatch
import os
import re
import sys

SELF_KEYS = {"SUBSYSTEM", "KERNEL", "ATTR"}
PARENT_KEYS = {"SUBSYSTEMS", "KERNELS", "ATTRS"}
ASSIGN_KEYS = {"GROUP", "MODE"}
TOKEN = re.compile(r'\s*([A-Z]+)(?:\{([^}]*)\})?\s*(==|!=|=|\+=)\s*"([^"]*)"\s*,?')


def parse(path):
    rules = []
    with open(path, encoding="utf-8") as f:
        for n, line in enumerate(f, 1):
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            keys, pos = [], 0
            while pos < len(line):
                m = TOKEN.match(line, pos)
                if not m:
                    sys.exit(f"{path}:{n}: cannot parse at: {line[pos:]!r}")
                key, attr, op, value = m.groups()
                if key not in SELF_KEYS | PARENT_KEYS | ASSIGN_KEYS:
                    sys.exit(f"{path}:{n}: unsupported key {key}")
                if key not in ASSIGN_KEYS and op != "==":
                    sys.exit(f"{path}:{n}: unsupported operator {op} on {key}")
                keys.append((key, attr, value))
                pos = m.end()
            rules.append((n, keys))
    return rules


def subsystem(dev):
    link = os.path.join(dev, "subsystem")
    return os.path.basename(os.path.realpath(link)) if os.path.islink(link) else ""


def attr(dev, name):
    try:
        with open(os.path.join(dev, name), encoding="utf-8", errors="replace") as f:
            return f.read().rstrip("\n").rstrip()
    except OSError:
        return None


def key_matches(dev, key, name, value):
    if key in ("SUBSYSTEM", "SUBSYSTEMS"):
        actual = subsystem(dev)
    elif key in ("KERNEL", "KERNELS"):
        actual = os.path.basename(dev)
    else:
        actual = attr(dev, name)
    return actual is not None and fnmatch.fnmatchcase(actual, value)


def chain(dev):
    while dev.startswith("/sys/devices/") and dev != "/sys/devices":
        yield dev
        dev = os.path.dirname(dev)


def rule_matches(dev, keys):
    if not all(key_matches(dev, k, a, v) for k, a, v in keys if k in SELF_KEYS):
        return False
    parent = [(k[:-1] if k != "ATTRS" else "ATTR", a, v) for k, a, v in keys if k in PARENT_KEYS]
    return not parent or any(all(key_matches(d, k, a, v) for k, a, v in parent) for d in chain(dev))


def main():
    rules = parse(sys.argv[1])
    base = "/sys/class/i2c-dev"
    names = sorted(os.listdir(base), key=lambda s: int(s.split("-")[1]))
    ddc = {}
    for c in os.listdir("/sys/class/drm"):
        link = os.path.join("/sys/class/drm", c, "ddc")
        if os.path.exists(link):
            ddc[os.path.basename(os.path.realpath(link))] = c
    print("| Node | Adapter name | Parent | DRM `ddc` link from | Matches (rule line) |")
    print("| --- | --- | --- | --- | --- |")
    for n in names:
        dev = os.path.realpath(os.path.join(base, n))
        parent = os.path.dirname(os.path.realpath(os.path.join(dev, "device")))
        hit = [str(line) for line, keys in rules if rule_matches(dev, keys)]
        print(f"| /dev/{n} | {attr(dev, 'name')} | {os.path.basename(parent)} ({subsystem(parent)})"
              f" | {ddc.get(n, '')} | {'**yes** (' + ', '.join(hit) + ')' if hit else 'no'} |")


if __name__ == "__main__":
    main()
