# Requirements

The code and the docs cite these as R0 to R15.

- **R0.** Every change to the project shall be reviewed by a cybersecurity reviewer before it is merged.
- **R1.** The project shall provide hardware dimming over DDC/CI from a Pop!_OS COSMIC desktop for external displays that implement it, and for a display that does not answer DDC (as the Samsung QN55LS03H Frame TV was found not to) shall show no hardware slider and no notice about it, without blocking the UI; the Samsung network API provides wake and power only, as it has no brightness control.
- **R2.** The project shall provide alpha-channel (software) dimming on stock cosmic-comp through the applet's own layer-shell overlay surfaces, regardless of what the display supports; a cosmic-comp fork is a fallback only if the overlay proves unworkable.
- **R3.** The project shall ship a standalone COSMIC panel applet, separately installable without replacing any stock COSMIC component, that gives each monitor hardware and alpha sliders; integrating the same capability into the existing COSMIC brightness control on forks is a parked second track, resumed only if upstream policy allows submission.
- **R4.** The project shall run without root at runtime: I2C and network access shall be granted through udev rules and group membership, not setuid or sudo.
- **R5.** Forks of System76's COSMIC repositories shall be kept but parked: no fork change is installed over the Pop!_OS packages unless the standalone applet cannot meet a requirement without it, and nothing is submitted upstream without the maintainers' agreement.
- **R6.** The standalone applet shall satisfy the reasonable asks found in upstream COSMIC issues on external-monitor brightness (several monitors, non-blocking DDC, no root), so it serves those users as a separately installable tool even while upstream submission is blocked.
- **R7.** The COSMIC brightness control shall show a slider per external monitor.
- **R8.** DDC/CI shall go through the kernel's i2c-dev interface, accessible without root via udev rules and group membership.
- **R9.** Several external monitors shall be dimmable at once, each independently.
- **R10.** DDC/CI communication shall never block the user interface: slow or unresponsive displays shall be handled off the UI thread with timeouts.
- **R11.** Every monitor shall have its own software alpha-dimming slider; a hardware-brightness slider shall appear only for a monitor whose hardware dimming the applet has detected as available, and shall be absent (not merely disabled) otherwise; where both exist they shall be settable independently or locked together so one movement sets both.
- **R12.** The combined effect of hardware and alpha dimming shall never make a screen so dark that the user cannot see the panel to raise it again: each slider has a non-zero floor, the combined effective brightness is bounded below, and brightness-up keys always work regardless of slider state.
- **R13.** Code shall follow the coding standards of the upstream project it is destined for: COSMIC / Pop!_OS conventions (rustfmt, clippy clean, their commit and i18n practices) for the COSMIC forks, and the Linux kernel coding style and patch rules for any kernel-side work.
- **R14.** This project shall contain only COSMIC-destined work and its own tooling; contributions to any other upstream project (the Linux kernel, ddcci-driver-linux, NVIDIA, ddcutil or others) shall live in a separate project of their own.
- **R15.** All of the project's code, including the standalone applet and its crates, shall be human readable (clear names, short functions, comments that explain why rather than what) and shall follow COSMIC's coding standards: rustfmt defaults, clippy clean with the pedantic lints, the structure and idioms of libcosmic and cosmic-applets, and Fluent translations for user-visible text.
