![Plugin Icon](assets/icon.png)

# OpenDeck Mirabox N1 Full LCD Plugin

An unofficial [Juan Field AI fork](https://github.com/juanfieldai/opendeck-mirabox-n1) of
[Sergey Ovechkin's Mirabox N1 plugin](https://github.com/pomeo/opendeck-mirabox-n1), based on
[opendeck-akp03](https://github.com/4ndv/opendeck-akp03) by Andrey Viktorov.

## OpenDeck version

Requires OpenDeck 2.14.0 or newer for the native Infobar UI.

## Supported devices

- Mirabox N1 (`6603:1000`)
- Mirabox N1 variant (`0300:3007`)
- MSD NEO (`0b00:1004`), full-LCD calibration confirmed on firmware `V3.MSD-NEO.02.011`

The device registers as 5 rows × 3 columns, one encoder, two touch points, and one Infobar
(type 0). Namespace `n1` and device IDs are unchanged. The 15 LCD keys remain Keypad positions
0–14. Screenless button A (`0x1e`) is Keypad 15, B (`0x1f`) is Keypad 16, and the knob is Encoder 0
(press/release `0x23`, rotation `0x32`/`0x33`). Both auxiliary buttons report press and release.

## Full LCD

Infobar 0 is the sole owner of the LCD: one upright, unmirrored 450×85 JPEG, sent at mirajazz
image index 15 (BAT wire slot 16). This canvas was physically calibrated by the user on the
MSD NEO firmware above; it is **not manufacturer-confirmed native pixel resolution** and does
not prove the same LCD dimensions on the other USB variants or firmware revisions.

Drag the **Full LCD** action into the native Infobar (the strip below A, B and the knob). Its default
450×85 SVG has an edge border and colored corner markers. Use OpenDeck's normal state-image
picker to replace it with your image; no title is overlaid. OpenDeck renders the state image
through its standard pipeline. This driver accepts that frame as PNG or JPEG, composites any
transparency over black, and encodes the LCD JPEG itself once (quality 95, 4:4:4 chroma).
Native 450×85 frames are not resampled; other sizes are resized with Lanczos3. There are no
three-tile aliases or encoder display slots. Images and clears for
Keypad 15/16 or Encoder 0 are ignored, so input actions cannot overwrite the LCD.
Clearing Infobar 0 writes and flushes a black 450×85 JPEG rather than a CLE slot command.

## Drawing the strip directly (sharp on stock OpenDeck)

OpenDeck rasterizes the Infobar itself at 248×58 before this driver receives it, and enlarging
that to 450×85 cannot make it sharp. Plugins can therefore draw the strip **directly** through
this driver, which renders the image at the native 450×85 and sends it straight to the device.
This works with unmodified OpenDeck.

The driver listens on a Unix socket (Linux/macOS) at
`$XDG_RUNTIME_DIR/opendeck-mirabox-n1/strip.sock`, in a directory only your user can open.
Requests are newline-delimited JSON; each gets one reply line, `{"ok":true}` or
`{"ok":false,"error":"…"}`:

```json
{"event":"drawStrip","device":"n1-<serial>","image":"data:image/svg+xml;base64,…"}
{"event":"releaseStrip","device":"n1-<serial>"}
```

- `image` is an SVG, PNG or JPEG data URL. SVG is rendered to fill 450×85 (system fonts are
  available); bitmaps of another size are resampled with Lanczos3.
- The connection that draws first owns that device's strip; `drawStrip` from another
  connection is rejected until the owner releases it or disconnects.
- While a client owns the strip, OpenDeck's Infobar frames are kept but not shown. Releasing or
  disconnecting shows OpenDeck's latest Infobar frame again, so plugins that only use
  `setImage` keep working and remain the fallback.
- The owner's last frame is redrawn when the device reconnects (unplug or resume).
- `device` is the OpenDeck device ID the plugin sees in `willAppear`.

Clients should keep sending the same image through OpenDeck's `setImage` for the editor
preview and as the fallback. [Muxboard](https://github.com/juanfieldai/muxboard) is a client.

## Layout and display geometry

The plugin also declares the N1's physical layout and display sizes in `registerDevice`
through an optional `layout` field: A, B and the knob on the top row, the LCD strip below them,
then the 5×3 keypad, with keys at 105×100, the strip at 450×85 and lossless PNG frames.
Stock OpenDeck 2.14.0 ignores the field: the editor keeps its default arrangement (keypad, then
knob, then A/strip/B) and keys arrive as 144×144 JPEG, which this driver downscales to 105×100
with Lanczos3. Only an OpenDeck build with generic `layout` support applies it.

## Breaking profile migration (0.3.0)

Back up your profiles before installing. Disable or remove the upstream N1 device plugin:
the fork UUID is `com.github.juanfieldai.opendeck-mirabox-n1`, but both plugins use namespace
`n1` and must not drive the same device at once. Existing LCD-key assignments (0–14) remain
unchanged. Move old encoder button A/B actions (formerly Encoder 0/1) to Keypad 15/16 touch
points, and move knob actions (formerly Encoder 2) to Encoder 0. Replace any old strip/encoder
image assignments with one Full LCD action on Infobar 0. Old profiles are not automatically
migrated and encoder/touch-point images no longer provide strip tiles.

## Notes on the N1

- The device boots into its built-in numpad layer. The plugin sends a mode-switch command on
  connect to put it into the "PC / stream-dock" mode where host images are displayed, and a
  periodic keep-alive so it doesn't drop off the USB bus.
- Key images are 105×100, displayed upright (no rotation/mirroring). This was calibrated on the
  MSD NEO above with one-pixel edge markers: 105×100 fit all four edges exactly, while larger
  images were cropped or ignored. Upstream used 108×104 for `6603:1000` and Mirabox's SDK states
  96×96; neither was confirmed on this unit. Key images use the same single JPEG encode as the
  full LCD (native frames are not resampled; others use Lanczos3).
- Mode 3 initialization, the two-second keep-alive, and full reconnect after host suspend
  retain the upstream behavior.
- Malformed image data URLs, MIME types other than JPEG/PNG, and out-of-range controller
  positions are rejected before a device draw; invalid image requests remain non-fatal.

## Platform support

- Linux: developed and tested here
- Mac / Windows: untested, best effort (build targets are wired up but unverified)

## Installation

1. Build the plugin (see below) or grab a release archive
2. In OpenDeck: Plugins -> Install from file
3. Linux: copy [udev rules](./40-opendeck-mirabox-n1.rules) into `/etc/udev/rules.d/` and run
   `sudo udevadm control --reload-rules`
4. Unplug and plug the device again, restart OpenDeck

## Building

### Prerequisites

- Rust 1.87+ with the `x86_64-unknown-linux-gnu` target
- For cross builds: `x86_64-pc-windows-gnu` target, mingw-w64 gcc, Docker, and [just](https://just.systems)

### Local debug build

```sh
cargo build
```

### Release package

```sh
just package
```

### Regression checks and hardware probes

`cargo test` covers the actual image consumer (including JPEG conversion, black LCD clear, and
single-pixel detail plus alpha flattening for native PNG LCD and key frames),
controller collision protection, bounds/data-URL rejection, auxiliary press/release states,
knob index 0, that the declared editor layout places every control exactly once, native SVG
strip rendering, strip-socket replies and failures, and that a directly owned strip hides
OpenDeck's LCD frames while keeping the latest one.
`cargo check --examples` checks the direct-device probe examples.
The `strip_probe` example now draws one calibrated full-LCD frame on `0b00:1004`, not three
segments. Hardware probes require closing OpenDeck first; they are not automated tests.

### Verified integration

On Linux with OpenDeck 2.14.0 and MSD NEO firmware `V3.MSD-NEO.02.011`:

- Thirteen Rust regression tests passed; all hardware examples compiled and the release binary built.
- The installed fork connected to `0b00:1004` and registered one encoder, two touch points, and one Infobar.
- The live editor showed one LCD action between the auxiliary controls, rather than three encoder-image tiles.
- Real OpenDeck image callbacks produced a `450x85` JPEG at BAT wire slot `16`; stock OpenDeck frontend frames were `248x58` before driver resizing.
- On stock OpenDeck 2.14.0, Muxboard drew the strip through the socket (`direct_strip_frame … output=450x85`); a second client's `drawStrip` was rejected as owned, and stopping Muxboard released the strip and redrew OpenDeck's retained `248x58` Infobar frame.
- The accompanying Muxboard native profile was migrated to this layout; its typecheck, tests, and native build passed.

The earlier direct-device ruler images were physically confirmed by the user. The live integration evidence above comes from the editor and driver logs, not framebuffer readback; no LCD screenshot/readback command is known.

## Acknowledgments

Built on top of [mirajazz](https://github.com/4ndv/mirajazz) and the
[opendeck-akp03](https://github.com/4ndv/opendeck-akp03) / opendeck-akp153 plugins by Andrey
Viktorov, which are in turn based on work by contributors of the
[elgato-streamdeck](https://github.com/streamduck-org/elgato-streamdeck) crate.
