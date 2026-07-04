# Changelog

All notable changes to this project will be documented in this file.

## [0.2.1] - 2026-07-04

- Recognize the `0300:3007` N1 variant

## [0.2.0] - 2026-06-26

- Automatically recover the device after the host resumes from suspend: the N1 is fully
  reconnected (mode switch, fresh input reader and OpenDeck re-registration), so it no longer
  gets stuck in its default mode requiring a USB replug or an OpenDeck restart
- `just`: allow overriding the docker command (`just docker="sudo docker" package`) for the
  macOS cross-build

## [0.1.0] - 2026-06-08

Initial release.

- Support for the Mirabox N1 (`6603:1000`)
- 15 LCD keys (5×3), images at 108×104
- Knob (rotation + press) and two extra buttons exposed as encoders
- Per-encoder icons rendered on the screen-strip segments
- Automatic switch into the device's image mode and periodic keep-alive
