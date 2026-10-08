use mirajazz::{error::MirajazzError, types::DeviceInput};

use crate::mappings::{ENCODER_COUNT, INPUT_KEY_COUNT};

// Device input codes for the Mirabox N1, determined from hardware:
//   0x01..=0x0f  15 LCD keys, row-major (top-left -> bottom-right). state 1=down, 0=up
//   0x32 / 0x33  knob rotation, one event per detent (left / right), state always 0
//   0x23         knob press (state 1/0). NOTE: also appears as part of the boot handshake
//                with state=2, which we ignore
//   0x1e         extra button A (state 1/0)
//   0x1f         extra button B (state 1/0)
//   0xcc / 0xaa  boot handshake noise, ignored
//
const KEY_BUTTON_A: usize = 15;
const KEY_BUTTON_B: usize = 16;
const ENC_KNOB: usize = 0;

pub fn process_input(input: u8, state: u8) -> Result<DeviceInput, MirajazzError> {
    log::debug!("Processing input: code=0x{:02x} state={}", input, state);

    match input {
        0x01..=0x0f => read_button_press((input - 1) as usize, state),
        0x32 | 0x33 => read_encoder_twist(input),
        0x23 => read_encoder_press(ENC_KNOB, state),     // knob press
        0x1e => read_button_press(KEY_BUTTON_A, state), // button A
        0x1f => read_button_press(KEY_BUTTON_B, state), // button B
        // Boot handshake (0xcc/0xaa) and anything unexpected: non-fatal, just ignore.
        _ => Err(MirajazzError::BadData),
    }
}

/// Keypad keys and the two screenless touch points share the input state vector.
fn read_button_press(key: usize, state: u8) -> Result<DeviceInput, MirajazzError> {
    if key >= INPUT_KEY_COUNT || state > 1 {
        return Err(MirajazzError::BadData);
    }

    let mut states = vec![false; INPUT_KEY_COUNT];
    states[key] = state != 0;

    Ok(DeviceInput::ButtonStateChange(states))
}

/// Knob rotation: 0x32 = left (-1), 0x33 = right (+1). One detent per event.
fn read_encoder_twist(input: u8) -> Result<DeviceInput, MirajazzError> {
    let mut values = vec![0i8; ENCODER_COUNT];

    values[ENC_KNOB] = match input {
        0x32 => -1,
        0x33 => 1,
        _ => return Err(MirajazzError::BadData),
    };

    Ok(DeviceInput::EncoderTwist(values))
}

/// Press or release of the sole knob (encoder 0).
fn read_encoder_press(encoder: usize, state: u8) -> Result<DeviceInput, MirajazzError> {
    // Real presses report state 0/1. The boot handshake emits 0x23 with state=2, ignore it.
    if state > 1 {
        return Err(MirajazzError::BadData);
    }

    let mut states = vec![false; ENCODER_COUNT];
    states[encoder] = state != 0;

    Ok(DeviceInput::EncoderStateChange(states))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auxiliary_buttons_decode_as_keypad_press_and_release() {
        for (code, position) in [(0x1e, 15), (0x1f, 16)] {
            let DeviceInput::ButtonStateChange(pressed) = process_input(code, 1).unwrap() else {
                panic!("Auxiliary button must produce a keypad state");
            };
            let mut expected = vec![false; 17];
            expected[position] = true;
            assert_eq!(pressed, expected);

            let DeviceInput::ButtonStateChange(released) = process_input(code, 0).unwrap() else {
                panic!("Auxiliary release must produce a keypad state");
            };
            assert_eq!(released, vec![false; 17]);
        }
    }

    #[test]
    fn knob_press_release_and_twist_use_only_encoder_zero() {
        for (state, expected) in [(1, true), (0, false)] {
            let DeviceInput::EncoderStateChange(states) = process_input(0x23, state).unwrap() else {
                panic!("Knob press must produce an encoder state");
            };
            assert_eq!(states, vec![expected]);
        }
        for (code, expected) in [(0x32, -1), (0x33, 1)] {
            let DeviceInput::EncoderTwist(ticks) = process_input(code, 0).unwrap() else {
                panic!("Knob twist must produce encoder ticks");
            };
            assert_eq!(ticks, vec![expected]);
        }
        assert!(matches!(process_input(0x23, 2), Err(MirajazzError::BadData)));
    }

    #[test]
    fn lcd_keys_keep_row_major_positions_in_seventeen_button_state() {
        for code in 0x01..=0x0f {
            let DeviceInput::ButtonStateChange(states) = process_input(code, 1).unwrap() else {
                panic!("LCD key must produce a keypad state");
            };
            let mut expected = vec![false; 17];
            expected[(code - 1) as usize] = true;
            assert_eq!(states, expected);
        }
    }
}
