//! Per-key RGB LEDs (SK6812MINI-E / WS2812 compatible) of the Corne.
//!
//! Every half has its own chain of 27 LEDs on the Corne LED data pin
//! (Pro Micro `D3` = nice!nano `P0_06`): 6 underglow LEDs first, then the 21
//! per-key LEDs in a snake pattern. The LEDs are driven with the nRF PWM
//! peripheral (EasyDMA), so no bit-banging and no interference with BLE.
//!
//! Colors are stored for the full 4x12 keymap (same row/col as `keyboard.toml`).
//! Each half only renders its own 6 columns.
//!
//! Change colors at runtime from anywhere with [`set_key_color`],
//! [`set_underglow`], [`set_brightness`] or [`show_layer`].

use core::cell::RefCell;

use embassy_nrf::gpio::{Level, Output, OutputDrive};
use embassy_nrf::pwm::{
    Config, Instance, Prescaler, SequenceConfig, SequenceLoad, SequencePwm, SingleSequenceMode,
    SingleSequencer,
};
use embassy_nrf::Peri;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::signal::Signal;
use embassy_time::Timer;
use rmk::channel::{ControllerSub, CONTROLLER_CHANNEL};
use rmk::controller::{Controller, EventController};
use rmk::action::{Action, KeyAction};
use rmk::event::{ControllerEvent, KeyboardEvent};
use rmk::keycode::KeyCode;

use crate::joystick::KeyboardSide;
use crate::keymap::{COL, NUM_LAYER, ROW};

#[derive(Clone, Copy, PartialEq, Eq, Default, defmt::Format)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
    fn scale(self, brightness: u8) -> Self {
        let s = |c: u8| ((c as u16 * brightness as u16) / 255) as u8;
        Self::new(s(self.r), s(self.g), s(self.b))
    }
}

pub const OFF: Rgb = Rgb::new(0, 0, 0);
pub const WHITE: Rgb = Rgb::new(255, 255, 255);
pub const RED: Rgb = Rgb::new(255, 0, 0);
pub const GREEN: Rgb = Rgb::new(0, 255, 0);
pub const BLUE: Rgb = Rgb::new(0, 0, 255);
pub const CYAN: Rgb = Rgb::new(0, 255, 255);
pub const MAGENTA: Rgb = Rgb::new(255, 0, 255);
pub const YELLOW: Rgb = Rgb::new(255, 255, 0);
pub const ORANGE: Rgb = Rgb::new(255, 80, 0);
pub const PURPLE: Rgb = Rgb::new(120, 0, 255);

const W: Rgb = WHITE;
const R: Rgb = RED;
const G: Rgb = GREEN;
const B: Rgb = BLUE;
const C: Rgb = CYAN;
const M: Rgb = MAGENTA;
const Y: Rgb = YELLOW;
const O: Rgb = ORANGE;
const P: Rgb = PURPLE;
const X: Rgb = OFF;

/// Default per-key colors for every layer, laid out like the keymap in
/// `keyboard.toml` / `keymap.rs`. Row 3 only uses cols 3..=8 (thumb keys).
/// The active (highest) layer is shown automatically on the central half.
#[rustfmt::skip]
pub const LAYER_COLORS: [[[Rgb; COL]; ROW]; NUM_LAYER] = [
    // BaseLayer
    [
        [O, W, W, W, W, W,   W, W, W, W, W, R],
        [O, W, W, W, W, W,   W, W, W, W, W, W],
        [O, W, W, W, W, W,   W, W, W, W, W, O],
        [X, X, X, C, C, C,   C, C, C, X, X, X],
    ],
    // LowerLayer
    [
        [B, B, B, B, B, B,   B, B, B, B, B, R],
        [B, P, P, P, P, P,   G, G, G, G, P, P],
        [B, P, P, P, P, P,   P, P, P, P, P, P],
        [X, X, X, C, C, C,   C, C, C, X, X, X],
    ],
    // UpperLayer
    [
        [M, M, M, M, M, M,   M, M, M, M, M, R],
        [M, Y, Y, Y, Y, Y,   Y, Y, Y, Y, Y, Y],
        [M, Y, Y, Y, Y, Y,   Y, Y, Y, Y, Y, Y],
        [X, X, X, C, C, C,   C, C, C, X, X, X],
    ],
    // Tri-layer
    [
        [G, G, G, G, G, G,   G, G, G, G, G, R],
        [G, G, G, G, G, G,   G, G, G, G, G, G],
        [G, G, G, G, G, G,   G, G, G, G, G, G],
        [X, X, X, C, C, C,   C, C, C, X, X, X],
    ],
];

pub const DEFAULT_UNDERGLOW: Rgb = PURPLE;
/// Global brightness 0..=255. Keep this low on battery: 27 LEDs at full
/// white draw well over 1 A.
pub const DEFAULT_BRIGHTNESS: u8 = 40;

/// Number of LEDs per half.
pub const NUM_LEDS: usize = 27;
const NUM_UNDERGLOW: usize = 6;
const COLS_PER_HALF: usize = COL / 2;

/// LED index in the chain for (row, column counted from the outer edge).
/// Same for both halves, the right half is mirrored on the PCB.
#[rustfmt::skip]
const KEY_TO_LED: [[Option<u8>; COLS_PER_HALF]; ROW] = [
    [Some(24), Some(23), Some(18), Some(17), Some(10), Some(9)],
    [Some(25), Some(22), Some(19), Some(16), Some(11), Some(8)],
    [Some(26), Some(21), Some(20), Some(15), Some(12), Some(7)],
    [None,     None,     None,     Some(14), Some(13), Some(6)],
];

struct State {
    keys: [[Rgb; COL]; ROW],
    underglow: Rgb,
    brightness: u8,
}

static STATE: Mutex<CriticalSectionRawMutex, RefCell<State>> = Mutex::new(RefCell::new(State {
    keys: LAYER_COLORS[0],
    underglow: DEFAULT_UNDERGLOW,
    brightness: DEFAULT_BRIGHTNESS,
}));
static REDRAW: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Set the color of a single key. `row`/`col` are keymap coordinates (4x12).
pub fn set_key_color(row: usize, col: usize, color: Rgb) {
    if row < ROW && col < COL {
        STATE.lock(|s| s.borrow_mut().keys[row][col] = color);
        REDRAW.signal(());
    }
}

/// Replace all key colors at once.
pub fn set_all_keys(colors: [[Rgb; COL]; ROW]) {
    STATE.lock(|s| s.borrow_mut().keys = colors);
    REDRAW.signal(());
}

/// Show the predefined colors of `layer` from [`LAYER_COLORS`].
pub fn show_layer(layer: usize) {
    if let Some(colors) = LAYER_COLORS.get(layer) {
        set_all_keys(*colors);
    }
}

pub fn set_underglow(color: Rgb) {
    STATE.lock(|s| s.borrow_mut().underglow = color);
    REDRAW.signal(());
}

pub fn set_brightness(brightness: u8) {
    STATE.lock(|s| s.borrow_mut().brightness = brightness);
    REDRAW.signal(());
}

/// Brightness change per `BacklightUp` / `BacklightDown` key press.
pub const BRIGHTNESS_STEP: u8 = 16;

/// Change the brightness by `delta`, clamped to 0..=255 (0 = off).
pub fn change_brightness(delta: i16) {
    STATE.lock(|s| {
        let mut s = s.borrow_mut();
        s.brightness = (s.brightness as i16 + delta).clamp(0, 255) as u8;
    });
    REDRAW.signal(());
}

// WS2812 timing with a 16 MHz PWM clock: 20 ticks = 1.25 us per bit.
// The high bit inverts the polarity so the pulse starts high.
const T1H: u16 = 0x8000 | 13;
const T0H: u16 = 0x8000 | 7;
const RES: u16 = 0x8000;
const SEQ_LEN: usize = NUM_LEDS * 24 + 1;

pub struct RgbLeds<'d, T: Instance> {
    pwm: SequencePwm<'d, T>,
    side: KeyboardSide,
    // Must live in RAM for EasyDMA.
    words: [u16; SEQ_LEN],
    // Keeps the nice!nano external VCC (P0_13) switched on, which powers the LEDs.
    _ext_power: Output<'d>,
}

impl<'d, T: Instance> RgbLeds<'d, T> {
    pub fn new(
        pwm: Peri<'d, T>,
        data_pin: Peri<'d, impl embassy_nrf::gpio::Pin>,
        ext_power_pin: Peri<'d, impl embassy_nrf::gpio::Pin>,
        side: KeyboardSide,
    ) -> Self {
        let mut config = Config::default();
        config.sequence_load = SequenceLoad::Common;
        config.prescaler = Prescaler::Div1;
        config.max_duty = 20;
        let pwm = defmt::unwrap!(SequencePwm::new_1ch(pwm, data_pin, config));
        let ext_power = Output::new(ext_power_pin, Level::High, OutputDrive::Standard);
        Self {
            pwm,
            side,
            words: [RES; SEQ_LEN],
            _ext_power: ext_power,
        }
    }

    fn render(&mut self) {
        let mut leds = [OFF; NUM_LEDS];
        STATE.lock(|s| {
            let s = s.borrow();
            leds[..NUM_UNDERGLOW].fill(s.underglow.scale(s.brightness));
            for (row, led_row) in KEY_TO_LED.iter().enumerate() {
                for (outer_col, led) in led_row.iter().enumerate() {
                    let Some(led) = led else { continue };
                    let col = match self.side {
                        KeyboardSide::Left => outer_col,
                        KeyboardSide::Right => COL - 1 - outer_col,
                    };
                    leds[*led as usize] = s.keys[row][col].scale(s.brightness);
                }
            }
        });

        for (led, words) in leds.iter().zip(self.words.chunks_exact_mut(24)) {
            // WS2812 / SK6812 expect GRB, MSB first.
            let grb = (led.g as u32) << 16 | (led.r as u32) << 8 | led.b as u32;
            for (i, w) in words.iter_mut().enumerate() {
                *w = if grb & (1 << (23 - i)) != 0 { T1H } else { T0H };
            }
        }
        self.words[SEQ_LEN - 1] = RES;
    }

    async fn write(&mut self) {
        let mut seq_config = SequenceConfig::default();
        seq_config.end_delay = 799; // >50 us reset
        let seq = SingleSequencer::new(&mut self.pwm, &self.words, seq_config);
        defmt::unwrap!(seq.start(SingleSequenceMode::Times(1)));
        // 649 bits * 1.25 us + 50 us reset ≈ 0.9 ms. Dropping the sequencer stops the PWM.
        Timer::after_millis(2).await;
        drop(seq);
    }

    /// Drive the LEDs. Redraws whenever colors are changed.
    pub async fn run(&mut self) -> ! {
        loop {
            self.render();
            self.write().await;
            REDRAW.wait().await;
        }
    }
}

/// Switches the key colors to [`LAYER_COLORS`] of the active layer and handles
/// the `BacklightUp` / `BacklightDown` keycodes.
///
/// Only works on the central: RMK doesn't sync the layer to the peripheral,
/// so the peripheral keeps showing whatever was set via [`set_key_color`] etc.
pub struct LayerRgbController {
    sub: ControllerSub,
}

impl LayerRgbController {
    pub fn new() -> Self {
        Self {
            sub: defmt::unwrap!(CONTROLLER_CHANNEL.subscriber()),
        }
    }
}

impl Controller for LayerRgbController {
    type Event = ControllerEvent;

    async fn process_event(&mut self, event: Self::Event) {
        match event {
            ControllerEvent::Layer(layer) => show_layer(layer as usize),
            ControllerEvent::Key(event, KeyAction::Single(Action::Key(key))) if is_press(event) => {
                match key {
                    KeyCode::BacklightUp => change_brightness(BRIGHTNESS_STEP as i16),
                    KeyCode::BacklightDown => change_brightness(-(BRIGHTNESS_STEP as i16)),
                    _ => (),
                }
            }
            _ => (),
        }
    }

    async fn next_message(&mut self) -> Self::Event {
        self.sub.next_message_pure().await
    }
}

impl EventController for LayerRgbController {}

/// `KeyboardEvent::pressed` is private in RMK 0.7, so compare against all
/// possible pressed key events instead.
fn is_press(event: KeyboardEvent) -> bool {
    (0..ROW as u8).any(|row| (0..COL as u8).any(|col| event == KeyboardEvent::key(row, col, true)))
}
