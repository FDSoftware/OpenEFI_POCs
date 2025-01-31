#![feature(type_alias_impl_trait)]
#![feature(exclusive_range_pattern)]
#![feature(proc_macro_hygiene)]
#![feature(int_roundings)]
#![feature(is_some_and)]
#![feature(stdsimd)]
#![feature(async_closure)]

#![no_main]
#![no_std]

#![allow(stable_features)]
#![allow(unused_mut)]

use rtic;
use rtic_monotonics::systick::*;
use rtic_sync::{channel::*, make_channel};

use w25q::series25::FlashInfo;
use arrayvec::ArrayVec;

use usb_device::{bus::UsbBusAllocator, device::UsbDevice};
use usbd_serial::SerialPort;
use usbd_webusb::{url_scheme, WebUsb};

use stm32f4xx_hal::{
    adc::{
        config::{AdcConfig, Dma, SampleTime, Scan, Sequence},
        Adc, Temperature,
    },
    dma::{config::DmaConfig, PeripheralToMemory, Stream0, StreamsTuple, Transfer},
    crc32,
    crc32::Crc32,
    gpio::{Edge, Input},
    otg_fs,
    otg_fs::{USB, UsbBusType},
    pac::{ADC1, TIM13, TIM2, TIM3, TIM5,DMA2,ADC2},
    prelude::*,
    spi::*,
    timer::{self, Event},
    watchdog::IndependentWatchdog,
};

use panic_halt as _;

#[rtic::app(device = stm32f4xx_hal::pac, peripherals = true, dispatchers = [TIM4, TIM7, TIM8_CC])]
mod app {
    use my_module::blink2;

    use crate::app::{
        engine::{
            cpwm::VRStatus,
            efi_cfg::{EngineConfig, get_default_efi_cfg},
            engine_status::{EngineStatus, get_default_engine_status},
            pmic::{PMIC, PmicT},
            sensors::{get_sensor_raw, SensorTypes, SensorValues},
        },
        gpio::{
            ADCMapping,
            AuxIoMapping,
            IgnitionGpioMapping,
            init_gpio,
            InjectionGpioMapping,
            LedGpioMapping,
            RelayMapping,
            StepperMapping,
        },
        injection::{calculate_time_isr, injection_setup},
        ignition::{ignition_checks, ignition_trigger},
        logging::host,
        memory::tables::{SpiT, Tables},
        webserial::{
            finish_message,
            handle_engine::engine_cdc_callback,
            handle_pmic::pmic_cdc_callback,
            handle_realtime_data::realtime_data_cdc_callback,
            handle_tables::table_cdc_callback,
            send_message,
            SerialMessage,
            SerialStatus,
        },
        tasks::{engine::ckp_checks/* , engine::motor_checks,  ignition::ignition_schedule */},
    };
    use crate::app::engine::sensors;
    use crate::app::tasks::engine::ckp_trigger;

    use super::*;

    mod my_module;
    // pub mod debug;
    pub mod engine;
    pub mod gpio;
    pub mod logging;
    pub mod memory;
    pub mod util;
    pub mod tasks;



    type DMATransfer =
    Transfer<Stream0<DMA2>, 0, Adc<ADC1>, PeripheralToMemory, &'static mut [u16; 6]>;

    #[shared]
    struct Shared {
        // Timers:
        timer: timer::CounterUs<TIM2>,
        timer3: timer::CounterUs<TIM3>,
        timer4: timer::CounterUs<TIM5>,
        timer13: timer::DelayUs<TIM13>,

        // Core I/O
        led: LedGpioMapping,
        inj_pins: InjectionGpioMapping,
        ign_pins: IgnitionGpioMapping,
        aux_pins: AuxIoMapping,
        relay_pins: RelayMapping,
        stepper_pins: StepperMapping,
        adc_transfer: DMATransfer,

        // core:
        spi_lock: bool,
        crc: Crc32,

        // EFI Related:
        efi_cfg: EngineConfig,
        efi_status: EngineStatus,
        flash: memory::tables::FlashT,
        flash_info: FlashInfo,
        tables: Tables,
        sensors: SensorValues,
        pmic: PmicT,

        // CKP/SYNC
        ckp: VRStatus,
        ignition_running: bool,
    }

    #[local]
    struct Local {
        // core
        watchdog: IndependentWatchdog,
        adc_buffer: Option<&'static mut [u16; 6]>,

        // EFI Related:
        ckp: stm32f4xx_hal::gpio::PC6<Input>,
        adc: Adc<ADC2>,
        analog_pins: ADCMapping,

        // cdc_sender: Sender<'static, u32, 8>,

        // TODO: Remove
        state: bool,
        state2: bool,

        // ignition,
        ign_channel_1: bool,
    }

    const CDC_BUFF_CAPACITY: usize = 30;

    #[init()]
    fn init(mut cx: init::Context) -> (Shared, Local) {
        // Setup clocks
        //let mut flash = cx.device.FLASH.constrain();
        let mut device = cx.device;
        let mut rcc = device.RCC.constrain();
        cx.core.DWT.enable_cycle_counter();

        // Initialize the systick interrupt & obtain the token to prove that we did
        let systick_mono_token = rtic_monotonics::create_systick_token!();
        Systick::start(cx.core.SYST, 120_000_000, systick_mono_token); // default STM32F407 clock-rate is 36MHz

        let _clocks = rcc.cfgr.use_hse(25.MHz()).sysclk(120.MHz()).require_pll48clk().freeze();

        // TODO: Disable this in release builds
        debug!("Hello v1 :)");

        let gpio_a = device.GPIOA.split();
        let gpio_b = device.GPIOB.split();
        let gpio_c = device.GPIOC.split();
        let gpio_d = device.GPIOD.split();
        let gpio_e = device.GPIOE.split();

        let mut gpio_config = init_gpio(gpio_a, gpio_b, gpio_c, gpio_d, gpio_e);

        // ADC
        let dma = StreamsTuple::new(device.DMA2);

        let config = DmaConfig::default()
            .transfer_complete_interrupt(true)
            .memory_increment(true)
            .double_buffer(false);

        let adc_config = AdcConfig::default()
            .dma(Dma::Continuous)
            .scan(Scan::Enabled);

        let mut adc1 = Adc::adc1(device.ADC1, true, adc_config);

        // aca van todos los canales a revisar con DMA
        adc1.configure_channel(&gpio_config.adc_dma.tps, Sequence::One, SampleTime::Cycles_480);
        adc1.configure_channel(&gpio_config.adc_dma.clt, Sequence::Two, SampleTime::Cycles_480);
        adc1.configure_channel(&gpio_config.adc_dma.iat, Sequence::Three, SampleTime::Cycles_480);
        adc1.configure_channel(&gpio_config.adc_dma.map, Sequence::Four, SampleTime::Cycles_480);
        adc1.configure_channel(&gpio_config.adc_dma.o2, Sequence::Five, SampleTime::Cycles_480);
        adc1.configure_channel(&gpio_config.adc_dma.vbatt, Sequence::Six, SampleTime::Cycles_480);

        let adc_first_buffer = cortex_m::singleton!(: [u16; 6] = [0; 6]).unwrap();
        let adc_second_buffer = Some(cortex_m::singleton!(: [u16; 6] = [0; 6]).unwrap());
        // Give the first buffer to the DMA. The second buffer is held in an Option in `local.buffer` until the transfer is complete
        let adc_transfer = Transfer::init_peripheral_to_memory(dma.0, adc1, adc_first_buffer, None, config);

        let mut adc = Adc::adc2(device.ADC2, true, AdcConfig::default());
        adc.enable();
        //adc.calibrate();

        // configure CKP/CMP Pin for Interrupts
        let mut ckp = gpio_config.ckp;
        let mut syscfg = device.SYSCFG.constrain();
        debug!("init gpio");
        ckp.make_interrupt_source(&mut syscfg);
        ckp.trigger_on_edge(&mut device.EXTI, Edge::Falling);

        // configure the timers

        // timer Tiempo inyeccion
        let mut timer: timer::CounterUs<TIM2> = device.TIM2.counter_us(&_clocks);

        // timer tiempo de ignicion
        let mut timer3: timer::CounterUs<TIM3> = device.TIM3.counter_us(&_clocks);

        // timer CPWM
        let mut timer4: timer::CounterUs<TIM5> = device.TIM5.counter_us(&_clocks);

        // timer uso generico
        let mut timer13: timer::DelayUs<TIM13> = device.TIM13.delay_us(&_clocks);

        //  TIM5, TIM7, TIM4
        debug!("init timers");
        // timer.start((150).millis()).unwrap();

        // Set up to generate interrupt when timer expires
        timer.listen(Event::Update);
        timer3.listen(Event::Update);
        timer13.listen(Event::Update);
        // timer4.start((70).minutes()).unwrap();
        timer4.start(1_000_000_u32.micros()).unwrap();

        let mut efi_cfg = get_default_efi_cfg();
        let mut _efi_status = get_default_engine_status();

        // SPI:
        let mode = Mode {
            polarity: Polarity::IdleLow,
            phase: Phase::CaptureOnFirstTransition,
        };

        let spi2 = Spi::new(
            device.SPI2,
            (gpio_config.spi_sck, gpio_config.spi_miso, gpio_config.spi_mosi),
            mode,
            (30).MHz(),
            &_clocks,
        );

        let spi_bus = shared_bus_rtic::new!(spi2, SpiT);
        let spi_pmic = spi_bus.acquire();

        // CRC32:
        let mut crc = crc32::Crc32::new(device.CRC);
        host::debug!("init flash");
        let mut flash = w25q::series25::Flash::init(spi_bus.acquire(), gpio_config.memory_cs).unwrap();

        // let id = flash.read_jedec_id().unwrap();

        let flash_info = flash.get_device_info().unwrap();

        // EFI Setup:
        let mut table = Tables {
            tps_rpm_ve: None,
            tps_rpm_afr: None,
            injector_delay: None,
            load_tps_deg: None,
            // get not implemented
            vbat_correction: None,
            wue: None,
            ase_taper: None,
            ase_intensity: None,
        };

        efi_cfg.read(&mut flash, &flash_info, &mut crc);

        let mut sensors = SensorValues::new();

        let mut spi_lock = false;
        let mut pmic = PMIC::init(spi_pmic, gpio_config.pmic.pmic1_cs).unwrap();

        // ckp.enable_interrupt(&mut device.EXTI);

        let mut ckp_status = VRStatus::new();


        // DEMO
        // Schedule the blinking task
        blink::spawn().ok();
        blink2::spawn().ok();


        let mut watchdog = IndependentWatchdog::new(device.IWDG);
        // se puede desactivar en debug
        // watchdog.start(100.millis());
        // watch_dog_update::spawn().unwrap();

        (Shared {
            // Timers:
            // delay,
            timer,
            timer3,
            timer4,
            timer13,

            // GPIO:
            led: gpio_config.led,
            inj_pins: gpio_config.injection,
            ign_pins: gpio_config.ignition,
            aux_pins: gpio_config.aux,
            relay_pins: gpio_config.relay,
            stepper_pins: gpio_config.stepper,
            sensors,

            // CORE:
            crc,
            flash,
            flash_info,
            spi_lock,
            adc_transfer,

            // EFI Related
            efi_cfg,
            efi_status: _efi_status,
            tables: table,
            pmic,

            //CKP/SYNC
            ckp: ckp_status,
            ignition_running: false,
        }, Local {
            watchdog,

            adc_buffer: adc_second_buffer,

            adc,
            ckp,
            analog_pins: gpio_config.adc,

            state: false,
            state2: false,
            // ignition,
            ign_channel_1: false,
        })
    }
    #[idle()]
    fn idle(cx: idle::Context) -> ! {
        // Locals in idle have lifetime 'static
        //let _x: &'static mut u32 = cx.local.x;
        loop {
            cortex_m::asm::nop();
            ckp_checks::spawn().unwrap();
            //Systick::delay(250.millis()).await;
        }
    }

    #[task(shared = [adc_transfer] ,priority = 3)]
    async fn polling_adc(mut cx: polling_adc::Context) {
        loop {
            cx.shared.adc_transfer.lock(|transfer| {
                transfer.start(|adc| {
                    adc.start_conversion();
                });
            });
            Systick::delay(250.millis()).await;
        }
    }

    #[task(binds = DMA2_STREAM0, shared = [adc_transfer,sensors], local = [adc_buffer])]
    fn sensors_adc_dma(mut cx: sensors_adc_dma::Context) {

        let (buffer, sample_to_millivolts) = cx.shared.adc_transfer.lock(|transfer| {
            // When the DMA completes it will return the buffer we gave it last time - we now store that as `buffer`
            // We still have our other buffer waiting in `local.buffer`, so `take` that and give it to the `transfer`
            let (buffer, _) = transfer
                .next_transfer(cx.local.adc_buffer.take().unwrap())
                .unwrap();

            let sample_to_millivolts = transfer.peripheral().make_sample_to_millivolts();
            (buffer, sample_to_millivolts)
        });

        // Pull the ADC data out of the buffer that the DMA transfer gave us

        let raw_tps = buffer[0];
        let raw_clt = buffer[1];
        let raw_iat = buffer[2];
        let raw_map = buffer[3];
        let raw_o2 = buffer[4];
        let raw_vbatt = buffer[5];

        // Now that we're finished with this buffer, put it back in `local.buffer` so it's ready for the next transfer
        // If we don't do this before the next transfer, we'll get a panic
        *cx.local.adc_buffer = Some(buffer);

        cx.shared.sensors.lock(|s| { s.update(sample_to_millivolts(raw_tps),SensorTypes::TPS) });
        cx.shared.sensors.lock(|s| { s.update(sample_to_millivolts(raw_clt),SensorTypes::CooltanTemp) });
        cx.shared.sensors.lock(|s| { s.update(sample_to_millivolts(raw_iat),SensorTypes::AirTemp) });
        cx.shared.sensors.lock(|s| { s.update(sample_to_millivolts(raw_map),SensorTypes::MAP) });
        cx.shared.sensors.lock(|s| { s.update(sample_to_millivolts(raw_o2),SensorTypes::ExternalLambda) });
        cx.shared.sensors.lock(|s| { s.update(sample_to_millivolts(raw_vbatt),SensorTypes::BatteryVoltage) });

    }

    #[task(local = [watchdog],priority = 3)]
    async fn watch_dog_update(mut ctx: watch_dog_update::Context) {
        loop {
            ctx.local.watchdog.feed();
            Systick::delay(50.millis()).await;
        }
    }

    #[task(local = [state], shared = [led],priority = 1)]
    async fn blink(mut cx: blink::Context) {
        loop {
            if *cx.local.state {
                cx.shared.led.lock(|l| { l.led_0.set_high() });
                *cx.local.state = false;
            } else {
                cx.shared.led.lock(|l| { l.led_0.set_low() });
                *cx.local.state = true;
            }
            Systick::delay(100.millis()).await;
        }
    }

    //TODO: reciclar para encendido
    #[task(binds = TIM2, local = [], shared = [timer, led])]
    fn timer2_exp(mut ctx: timer2_exp::Context) {
        ctx.shared.timer.lock(|tim| tim.clear_interrupt(Event::Update));

        ctx.shared.led.lock(|l| l.led_0.toggle());
    }

    #[task(binds = TIM3, local = [], shared = [timer3, led])]
    fn timer3_exp(mut ctx: timer3_exp::Context) {
        ctx.shared.timer3.lock(|tim| {
            tim.clear_interrupt(Event::Update);
            tim.cancel().unwrap();
        });

        ctx.shared.led.lock(|l| l.led_check.set_high());
    }

    // Externally defined tasks
    extern "Rust" {

        // from: https://github.com/noisymime/speeduino/blob/master/speeduino/decoders.ino#L453
        #[task(binds = EXTI9_5, local = [ckp], shared = [led, efi_status, flash_info, efi_cfg, timer, timer3, timer4, ckp, ign_pins], priority = 5)]
        fn ckp_trigger(ctx: ckp_trigger::Context);
        #[task(shared = [efi_cfg, ckp, timer4, efi_status, ignition_running],priority = 3)]
        async fn ckp_checks(ctx: ckp_checks::Context);

        //
        // #[task(
        // shared = [led, efi_status, efi_cfg, timer3, timer4, ckp, ign_pins],
        // priority = 4
        // )]
        // async fn ignition_schedule(ctx: ignition_schedule::Context);
        //
    }

    // Externally defined tasks
    extern "Rust" {
        #[task(local = [state2], shared = [led],priority = 1)]
        async fn blink2(cx: blink2::Context);
    }
}
