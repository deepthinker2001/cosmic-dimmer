# Changelog

## 0.1.0

First release.

- A standalone COSMIC panel applet that dims each display on its own:
  software dimming through a translucent overlay on any display, and hardware
  dimming over DDC/CI where a display supports it (not yet tested on a real
  DDC/CI monitor).
- Minimum brightness floors, so the screen never goes black, and
  `cosmic-dimmer --reset` to bring every display back to full brightness from
  a terminal.
- Auto dimming: learns the levels you set at different times of day and
  follows them, 1 % at a time, relative to sunrise and sunset where you are.
- Optional brightness shortcuts (Super + F1 / F2 by default, rebindable).
- Levels saved per display, keyed by the display's EDID identity.
- English, German, Spanish, French, Italian, Brazilian Portuguese, Dutch,
  Polish, Swedish, Japanese and Simplified Chinese.
