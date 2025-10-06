#![no_std]
#![no_main]

mod vial;
#[macro_use]
mod macros;
mod joystick;
mod keymap;

use crate::keymap::{COL, COL_OFFSET, NUM_ENCODER, NUM_LAYER, ROW};
use defmt::{info, unwrap};
use embassy_executor::Spawner;
use embassy_nrf::gpio::{Input, Output};
use embassy_nrf::interrupt::{self, InterruptExt};
use embassy_nrf::mode::Async;
use embassy_nrf::peripherals::{RNG, SAADC, USBD};
use embassy_nrf::saadc::{self, AnyInput, Input as _, Saadc};
use embassy_nrf::usb::vbus_detect::HardwareVbusDetect;
use embassy_nrf::usb::Driver;
use embassy_nrf::{bind_interrupts, rng, usb, Peri};
use embassy_time::Duration;
use nrf_mpsl::Flash;
use nrf_sdc::mpsl::MultiprotocolServiceLayer;
use nrf_sdc::{self as sdc, mpsl};
use rand_chacha::ChaCha12Rng;
use rand_core::SeedableRng;
use rmk::ble::trouble::build_ble_stack;
use rmk::channel::{blocking_mutex::raw::NoopRawMutex, channel::Channel, EVENT_CHANNEL};
use rmk::config::macro_config::KeyboardMacrosConfig;
use rmk::config::{
    BehaviorConfig, BleBatteryConfig, KeyboardUsbConfig, RmkConfig, StorageConfig, VialConfig,
};
use rmk::debounce::default_debouncer::DefaultDebouncer;
use rmk::event::Event;
use rmk::futures::future::{join, join4};
use rmk::input_device::adc::{AnalogEventType, NrfAdc};
use rmk::input_device::battery::BatteryProcessor;
use rmk::input_device::joystick::JoystickProcessor;
use rmk::input_device::rotary_encoder::RotaryEncoder;
use rmk::input_device::Runnable;
use rmk::keyboard::Keyboard;
use rmk::light::LightController;
use rmk::split::ble::central::read_peripheral_addresses;
use rmk::split::central::{run_peripheral_manager, CentralMatrix};
use rmk::{
    initialize_encoder_keymap_and_storage, run_devices, run_processor_chain, run_rmk, HostResources,
};
use static_cell::StaticCell;
use vial::{VIAL_KEYBOARD_DEF, VIAL_KEYBOARD_ID};

use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    USBD => usb::InterruptHandler<USBD>;
    SAADC => saadc::InterruptHandler;
    RNG => rng::InterruptHandler<RNG>;
    EGU0_SWI0 => nrf_sdc::mpsl::LowPrioInterruptHandler;
    CLOCK_POWER => nrf_sdc::mpsl::ClockInterruptHandler, usb::vbus_detect::InterruptHandler;
    RADIO => nrf_sdc::mpsl::HighPrioInterruptHandler;
    TIMER0 => nrf_sdc::mpsl::HighPrioInterruptHandler;
    RTC0 => nrf_sdc::mpsl::HighPrioInterruptHandler;
});

#[embassy_executor::task]
async fn mpsl_task(mpsl: &'static MultiprotocolServiceLayer<'static>) -> ! {
    mpsl.run().await
}

/// How many outgoing L2CAP buffers per link
const L2CAP_TXQ: u8 = 4;

/// How many incoming L2CAP buffers per link
const L2CAP_RXQ: u8 = 4;

/// Size of L2CAP packets
const L2CAP_MTU: usize = 251;

fn build_sdc<'d, const N: usize>(
    p: nrf_sdc::Peripherals<'d>,
    rng: &'d mut rng::Rng<RNG, Async>,
    mpsl: &'d MultiprotocolServiceLayer,
    mem: &'d mut sdc::Mem<N>,
) -> Result<nrf_sdc::SoftdeviceController<'d>, nrf_sdc::Error> {
    sdc::Builder::new()?
        .support_scan()?
        .support_central()?
        .support_adv()?
        .support_peripheral()?
        .support_dle_peripheral()?
        .support_dle_central()?
        .support_phy_update_central()?
        .support_phy_update_peripheral()?
        .support_le_2m_phy()?
        .central_count(1)?
        .peripheral_count(1)?
        .buffer_cfg(L2CAP_MTU as u16, L2CAP_MTU as u16, L2CAP_TXQ, L2CAP_RXQ)?
        .build(p, rng, mpsl, mem)
}

fn ble_addr() -> [u8; 6] {
    let ficr = embassy_nrf::pac::FICR;
    let high = u64::from(ficr.deviceid(1).read());
    let addr = high << 32 | u64::from(ficr.deviceid(0).read());
    let addr = addr | 0x0000_c000_0000_0000;
    unwrap!(addr.to_le_bytes()[..6].try_into())
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("Hello RMK BLE!");
    // Initialize the peripherals and nrf-sdc controller
    let mut nrf_config = embassy_nrf::config::Config::default();
    nrf_config.dcdc.reg0_voltage = Some(embassy_nrf::config::Reg0Voltage::_3V3);
    nrf_config.dcdc.reg0 = true;
    nrf_config.dcdc.reg1 = true;
    let p = embassy_nrf::init(nrf_config);
    let mpsl_p =
        mpsl::Peripherals::new(p.RTC0, p.TIMER0, p.TEMP, p.PPI_CH19, p.PPI_CH30, p.PPI_CH31);
    let lfclk_cfg = mpsl::raw::mpsl_clock_lfclk_cfg_t {
        source: mpsl::raw::MPSL_CLOCK_LF_SRC_RC as u8,
        rc_ctiv: mpsl::raw::MPSL_RECOMMENDED_RC_CTIV as u8,
        rc_temp_ctiv: mpsl::raw::MPSL_RECOMMENDED_RC_TEMP_CTIV as u8,
        accuracy_ppm: mpsl::raw::MPSL_DEFAULT_CLOCK_ACCURACY_PPM as u16,
        skip_wait_lfclk_started: mpsl::raw::MPSL_DEFAULT_SKIP_WAIT_LFCLK_STARTED != 0,
    };
    static MPSL: StaticCell<MultiprotocolServiceLayer> = StaticCell::new();
    static SESSION_MEM: StaticCell<mpsl::SessionMem<1>> = StaticCell::new();
    let mpsl = MPSL.init(unwrap!(mpsl::MultiprotocolServiceLayer::with_timeslots(
        mpsl_p,
        Irqs,
        lfclk_cfg,
        SESSION_MEM.init(mpsl::SessionMem::new())
    )));
    spawner.must_spawn(mpsl_task(&*mpsl));
    let sdc_p = sdc::Peripherals::new(
        p.PPI_CH17, p.PPI_CH18, p.PPI_CH20, p.PPI_CH21, p.PPI_CH22, p.PPI_CH23, p.PPI_CH24,
        p.PPI_CH25, p.PPI_CH26, p.PPI_CH27, p.PPI_CH28, p.PPI_CH29,
    );
    let mut rng = rng::Rng::new(p.RNG, Irqs);
    let mut rng_gen = ChaCha12Rng::from_rng(&mut rng).unwrap();
    let mut sdc_mem = sdc::Mem::<8192>::new();
    let sdc = unwrap!(build_sdc(sdc_p, &mut rng, mpsl, &mut sdc_mem));
    let mut host_resources = HostResources::new();
    let stack = build_ble_stack(sdc, ble_addr(), &mut rng_gen, &mut host_resources).await;

    // Initialize usb driver
    let driver = Driver::new(p.USBD, Irqs, HardwareVbusDetect::new(Irqs));

    // Initialize flash
    let flash = Flash::take(mpsl, p.NVMC);

    // Initialize IO Pins
    let (input_pins, output_pins) = config_matrix_pins_nrf!(
        peripherals: p,
        input: [P0_22, P0_24, P1_00, P0_11],
        output: [P1_07, P1_02, P1_01, P1_15, P1_13, P1_11]
    );
    const INPUT_PIN_NUM: usize = 4;
    const OUTPUT_PIN_NUM: usize = 6;
    // Initialize the ADC.
    // We are only using one channel for detecting battery level
    let saadc_config = saadc::Config::default();
    let saadc = saadc::Saadc::new(
        p.SAADC,
        Irqs,
        saadc_config,
        [
            saadc::ChannelConfig::single_ended(saadc::VddhDiv5Input.degrade_saadc()),
            saadc::ChannelConfig::single_ended(p.P0_31.degrade_saadc()),
            saadc::ChannelConfig::single_ended(p.P0_29.degrade_saadc()),
        ],
    );
    interrupt::SAADC.set_priority(interrupt::Priority::P3);
    // Wait for ADC calibration.
    saadc.calibrate().await;

    // Keyboard config
    let keyboard_usb_config = KeyboardUsbConfig {
        vid: 0x4c4b,
        pid: 0x4643,
        manufacturer: "Luca",
        product_name: "Corne",
        serial_number: "vial:f64c2b3c:000001",
    };
    let vial_config = VialConfig::new(VIAL_KEYBOARD_ID, VIAL_KEYBOARD_DEF);
    let ble_battery_config = BleBatteryConfig::new(None, true, None, false);
    let storage_config = StorageConfig {
        start_addr: 0xA0000,
        num_sectors: 6,
        clear_storage: true,
        ..Default::default()
    };
    let rmk_config = RmkConfig {
        usb_config: keyboard_usb_config,
        vial_config,
        ble_battery_config,
        storage_config,
        ..Default::default()
    };

    // Initialize keyboard stuff
    // Initialize the storage and keymap
    let mut default_keymap = keymap::get_default_keymap();
    let mut behavior_config = BehaviorConfig {
        tri_layer: Some([1, 2, 3]),
        keyboard_macros: KeyboardMacrosConfig {
            macro_sequences: keymap::get_macro_sequences(),
        },
        ..BehaviorConfig::default()
    };
    let mut encoder_map = keymap::get_default_encoder_map();
    let (keymap, mut storage) = initialize_encoder_keymap_and_storage(
        &mut default_keymap,
        &mut encoder_map,
        flash,
        &storage_config,
        behavior_config,
    )
    .await;

    // Initialize the matrix and keyb oard
    let debouncer = DefaultDebouncer::<4, 7>::new();
    let mut matrix = CentralMatrix::<_, _, _, 0, 0, INPUT_PIN_NUM, OUTPUT_PIN_NUM>::new(
        input_pins,
        output_pins,
        debouncer,
    );
    let mut keyboard = Keyboard::new(&keymap);

    // Read peripheral address from s torage
    let peripheral_addrs =
        read_peripheral_addresses::<1, _, ROW, COL, NUM_LAYER, NUM_ENCODER>(&mut storage).await;

    // Initialize the processors
    let local_channel: Channel<NoopRawMutex, Event, 16> = Channel::new();
    let mut local_analog_devices = NrfAdc::new(
        saadc,
        [AnalogEventType::Joystick(2)],
        Duration::from_ticks(20),
        Some(Duration::from_ticks(300)),
    );
    let mut batt_proc = BatteryProcessor::new(2000, 2806, &keymap);
    // left is [-259XX, -295XX]
    // right is [-311XX, -295XX]
    // up is [-288XX, -320XX]
    // down is [-288XX, -269XX]
    // neutral is [-282XX, -293XX]
    // the last to digits are useless
    // the third last digit is unreliable
    // the leading two digigs are reliable
    // up - down =  [0, -5900]
    // left - right = [-5200, 0]
    // =====> bias = -[-5200, -5900]
    let speed = 4.0;
    let mut joystick_processor_left = joystick::JoystickProcessor::new(
        [[-0.001 * speed, 0.0], [0.0, 0.001 * speed]],
        [28600, 29555],
        [0.4, 0.15],
        &keymap,
        joystick::KeyboardSide::Left,
    );
    let mut joystick_processor_right = joystick::JoystickProcessor::new(
        [[0.001 * speed, 0.0], [0.0, 0.001 * speed]],
        [28600, 29555],
        [0.4, 0.35],
        &keymap,
        joystick::KeyboardSide::Right,
    );

    // Initialize the controllers
    let mut light_controller: LightController<Output> =
        LightController::new(rmk::config::LightConfig {
            capslock: None,
            scrolllock: None,
            numslock: None,
        });

    // Start
    join4(
        run_devices! (
            (matrix) => EVENT_CHANNEL,
            (local_analog_devices) => local_channel
        ),
        run_processor_chain! {
            local_channel => [joystick_processor_left, batt_proc],
            EVENT_CHANNEL => [joystick_processor_right],
        },
        keyboard.run(),
        join(
            run_peripheral_manager::<4, 7, 0, COL_OFFSET, _>(0, peripheral_addrs[0], &stack),
            run_rmk(
                &keymap,
                driver,
                &stack,
                &mut storage,
                &mut light_controller,
                rmk_config,
            ),
        ),
    )
    .await;
}
