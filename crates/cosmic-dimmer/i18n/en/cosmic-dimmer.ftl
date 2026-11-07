applet-name = Dimmer

looking = Looking for displays…
display-unnamed = Display { $number }
no-displays = No displays found.

level = { $value }%

software-unavailable = Software dimming unavailable: { $reason }
reason-not-found = display not found
reason-read-only = display does not accept changes
reason-unsupported = not supported in this build
reason-timed-out = no answer in time
reason-invalid-level = level out of range
reason-unavailable = display can't be reached right now
reason-superseded = replaced by a newer level

not-applied = Not applied: { $reason }
not-applied-at = Not applied: { $reason }. The display is at { $value }%.

lock = Move both together

auto-dimming = Auto dimming
location = Location for sunrise and sunset
location-time-zone = From time zone ({ $zone })
location-placeholder = Latitude, longitude
location-invalid = Not a location. Type latitude, longitude in degrees, like 39.74, -104.99.
location-unknown = Your time zone has no known location. Type latitude, longitude for auto dimming to work.

settings = Settings
floor-hardware = Lowest hardware level
floor-software = Minimum brightness
floor-effective = Lowest combined level
key-step = Change per key press

keys-unavailable = Brightness keys unavailable: { $error }
shortcuts-toggle = Brightness shortcuts
shortcut-raise = Increase brightness key
shortcut-lower = Decrease brightness key
shortcut-press = Press keys…
shortcut-hint = Press an F-key, or a combination with Super, Ctrl or Alt. Esc cancels. Keys COSMIC itself uses won't register.
shortcuts-taken = Not changed: { $binding } is already in use.
shortcuts-invalid = Not changed: that key combination can't be used as a shortcut.
shortcut-description-raise = Dimmer: increase brightness
shortcut-description-lower = Dimmer: decrease brightness
shortcuts-unreadable = Not changed: your custom shortcuts could not be read safely, so they were left as they are.
shortcuts-failed = Not changed: { $error }

usage =
    Usage: cosmic-dimmer [--reset | --help | --version]
    Without options it runs as a COSMIC panel applet.
    --reset  Set every display back to 100% (hardware and software dimming) and
             save that, for when a screen is too dark to use the applet.
reset-applet-done = The running applet is setting every display to 100% and removing its overlays.
reset-applet-not-running = The applet is not running, so no software dimming is on screen.
reset-applet-no-answer = The applet did not answer ({ $error }). If the screen is still dimmed, end the applet with: pkill -f 'cosmic-dimmer$'
reset-saved = Saved levels set to 100% for { $count ->
        [one] { $count } display
       *[other] { $count } displays
    }.
reset-saved-failed = Saved levels were not reset: { $error }
reset-saved-no-home = Saved levels were not reset: HOME is not set, so the settings folder can't be found.
reset-hardware = Hardware brightness set to 100% on { $count ->
        [one] { $count } display
       *[other] { $count } displays
    }.
reset-hardware-partial = Hardware brightness set to 100% on { $count ->
        [one] { $count } display
       *[other] { $count } displays
    }; { $failed } did not take it.
reset-hardware-skipped = Hardware brightness not changed: { $reason }.
reset-failed = Reset failed: { $error }
