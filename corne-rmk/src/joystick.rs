use core::cell::RefCell;

use rmk::channel::KEYBOARD_REPORT_CHANNEL;
use rmk::event::Event;
use rmk::hid::Report;
use rmk::input_device::{InputProcessor, ProcessResult};
use rmk::keymap::KeyMap;
use usbd_hid::descriptor::MouseReport;

pub enum KeyboardSide {
    Left,
    Right,
}

pub struct JoystickProcessor<
    'a,
    const ROW: usize,
    const COL: usize,
    const NUM_LAYER: usize,
    const NUM_ENCODER: usize,
    const N: usize,
> {
    transform: [[f32; N]; N],
    bias: [i16; N],
    threshold: [f32; N],
    keymap: &'a RefCell<KeyMap<'a, ROW, COL, NUM_LAYER, NUM_ENCODER>>,
    record: [i16; N],
    side: KeyboardSide,
}

impl<
        'a,
        const ROW: usize,
        const COL: usize,
        const NUM_LAYER: usize,
        const NUM_ENCODER: usize,
        const N: usize,
    > JoystickProcessor<'a, ROW, COL, NUM_LAYER, NUM_ENCODER, N>
{
    pub fn new(
        transform: [[f32; N]; N],
        bias: [i16; N],
        threshold: [f32; N],
        keymap: &'a RefCell<KeyMap<'a, ROW, COL, NUM_LAYER, NUM_ENCODER>>,
        side: KeyboardSide,
    ) -> Self {
        Self {
            transform,
            bias,
            threshold,
            keymap,
            record: [0; N],
            side,
        }
    }
    async fn generate_report(&mut self) {
        for (rec, b) in self.record.iter_mut().zip(self.bias.iter()) {
            *rec = rec.saturating_add(*b);
        }
        let mut result = [0i8; N];
        for i in 0..N {
            let mut sum = 0.0f32;
            for j in 0..N {
                sum += self.transform[i][j] * self.record[j] as f32;
            }

            sum = if sum.abs() < self.threshold[i] {
                0.0
            } else {
                sum
            };
            // Cast back to i8 with saturation
            result[i] = sum.clamp(-128.0, 127.0) as i8;
        }

        // map to mouse
        let mouse_report = match self.side {
            KeyboardSide::Left => MouseReport {
                buttons: 0,
                x: result[0],
                y: result[1],
                wheel: 0,
                pan: 0,
            },
            KeyboardSide::Right => MouseReport {
                buttons: 0,
                x: 0,
                y: 0,
                // wheel: result[0],
                // pan: result[1],
                wheel: 0,
                pan: 0,
            },
        };
        self.send_report(Report::MouseReport(mouse_report)).await;
    }
}

impl<
        'a,
        const ROW: usize,
        const COL: usize,
        const NUM_LAYER: usize,
        const NUM_ENCODER: usize,
        const N: usize,
    > InputProcessor<'a, ROW, COL, NUM_LAYER, NUM_ENCODER>
    for JoystickProcessor<'a, ROW, COL, NUM_LAYER, NUM_ENCODER, N>
{
    async fn process(&mut self, event: Event) -> ProcessResult {
        embassy_time::Timer::after_millis(5).await;
        match event {
            Event::Joystick(event) => {
                for (rec, e) in self.record.iter_mut().zip(event.iter()) {
                    *rec = e.value;
                }
                // debug!("Joystick info: {:#?}", self.record);
                self.generate_report().await;
                ProcessResult::Stop
            }
            _ => ProcessResult::Continue(event),
        }
    }

    /// Send the processed report.
    async fn send_report(&self, report: Report) {
        KEYBOARD_REPORT_CHANNEL.send(report).await;
    }

    /// Get the current keymap
    fn get_keymap(&self) -> &RefCell<KeyMap<'a, ROW, COL, NUM_LAYER, NUM_ENCODER>> {
        self.keymap
    }
}
