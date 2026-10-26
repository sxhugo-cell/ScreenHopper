#!/bin/sh
# Load uinput now and apply the access rule, so it works without a reboot.
modprobe uinput 2>/dev/null || true
udevadm control --reload-rules 2>/dev/null || true
udevadm trigger --name-match=uinput 2>/dev/null || true
exit 0
