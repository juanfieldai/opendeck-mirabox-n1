# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

- Encode key and full-LCD images once (quality 95, 4:4:4) instead of mirajazz's
  nearest-neighbour resize plus quality-90 re-encode; accept lossless PNG frames and skip
  resampling for native-size input.
- Key images are now 105×100, calibrated on the MSD NEO (`0b00:1004`) with edge-marker
  patterns: 105×100 fills the key exactly, so the previous 108×104 exceeded the visible area.
- Declare the N1's physical layout (A, B and knob on top, LCD strip below, then the keypad),
  key/strip pixel sizes and lossless frame delivery through an optional `layout` field in
  `registerDevice`. OpenDeck builds without `layout` support ignore it.
- Let plugins draw the full LCD directly at native resolution through a user-only Unix socket
  (`$XDG_RUNTIME_DIR/opendeck-mirabox-n1/strip.sock`; SVG, PNG or JPEG data URLs), sharp on
  stock OpenDeck. The drawing connection owns the strip; OpenDeck's Infobar frames return when
  it releases or disconnects.
- Keep the first and last ten LCD columns black on every strip image: OpenDeck and direct
  frames are fitted to a 430×85 drawable area between driver-added margins. The Full LCD
  calibration image now marks that area.

## [0.3.0] - 2026-10-08

- Juan Field AI fork of Sergey Ovechkin's N1 plugin; new plugin/package UUID
  `com.github.juanfieldai.opendeck-mirabox-n1`, retaining namespace `n1` and device IDs.
- Add MSD NEO `0b00:1004` discovery and Linux udev permissions.
- Register one encoder, two touch points, and one native Infobar through the public
  OpenAction JSON event API. Keep the 5×3 keypad and 108×104 key images unchanged.
- **Breaking:** button A/B are now Keypad 15/16 with press/release, and the knob is Encoder 0.
  Move old Encoder 0/1 auxiliary actions and Encoder 2 knob actions to the new positions.
  Replace old encoder/strip image assignments with one Full LCD action on Infobar 0;
  back up profiles and disable the upstream plugin before installing this fork.
- Infobar 0 exclusively writes one 450×85 JPEG at mirajazz index 15 / BAT wire slot 16.
  The user physically calibrated this canvas on `0b00:1004`, firmware `V3.MSD-NEO.02.011`;
  it is not proof of manufacturer-native resolution or of dimensions on other revisions.
- Ignore screenless Keypad 15/16 and encoder image requests. LCD clear draws/flushes a black
  JPEG instead of CLE slots; reject invalid positions and malformed data URLs without panic.
- Add a standard Full LCD Infobar action with a 450×85 SVG calibration state image and
  no title; retain native drag/drop and normal image-picker customization.
- Make the device available before registration so the initial OpenDeck image can be handled.
  Keep upstream mode 3 initialization, keep-alive, and resume reconnect behavior unchanged.
- Replace the three-tile strip probe and add image-consumer and input regression tests.

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
