// SPDX-FileCopyrightText: 2025-2026 Espressif Systems (Shanghai) CO LTD
// SPDX-FileCopyrightText: 2026 IOBEWI
// SPDX-License-Identifier: MIT

#![no_std]
#![no_main]

//! Opt-in RAM image for USB-OTG CDC qualification on P4 v3.2.

use core::{
    arch::{asm, global_asm},
    panic::PanicInfo,
    ptr,
    sync::atomic::{AtomicBool, Ordering},
};
use stub_core::{
    rx::{DoubleBuffer, Poll},
    tx::{PumpResult, TxQueue},
    usb_irq::{LineDecision, RxIrqCoordinator, RxIrqMask, RxIrqSignal, RxLineState, ServiceResult},
    usb_rx::RxPump,
};
use stub_rom::{p4_usb::P4RomCdcTx, CdcAcmDevice};

global_asm!(
    r#"
    .section .text.entry, "ax", @progbits
    .global esp_main
    .type esp_main, @function
esp_main:
    la sp, __stack_top
    la t0, __bss_start
    la t1, __bss_end
1:
    bgeu t0, t1, 2f
    sw zero, 0(t0)
    addi t0, t0, 4
    j 1b
2:
    call rust_entry
3:
    j 3b
"#
);

// Layout sentinels checked by elf2stub.py.
#[used]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".data.foundation_sentinel")]
pub static INITIALIZED_SENTINEL: u32 = 0x5354_5542;

#[used]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".bss.foundation_sentinel")]
pub static mut ZEROED_SENTINEL: [u32; 4] = [0; 4];

const USB_IRQ: i32 = 17;
const USB_IRQ_MASK: i32 = 1 << USB_IRQ;
const USB_OTG_DEVICE: u8 = 5;
const UART_DEVICE_NUMBER_OFFSET: usize = 24;
const EFUSE_MAC_SYS_2: *const u32 = 0x5012_D04C as *const u32;
const USB_PHY_CONTROL: *mut u32 = 0x500E_515C as *mut u32;
const USB_IRQ_MAP_CORE0: *mut u32 = 0x500D_6174 as *mut u32;
const USB_GAHBCFG: *mut u32 = 0x5000_0008 as *mut u32;
const EVT_LINE_STATE: i32 = -1;
const EVT_RX: i32 = -4;
const LINE_CTRL_RTS: u32 = 1 << 1;
const TX_TIMEOUT_US: u32 = 250_000;
const TX_RETRY_US: u32 = 100;

static SIGNAL: RxIrqSignal = RxIrqSignal::new();
static LAST_RTS: AtomicBool = AtomicBool::new(false);
static LINE_FAULT: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy)]
#[repr(u32)]
enum TxOutcome {
    Sent = 1,
    Timeout,
    FifoError,
    LinkInvalid,
}

#[repr(C)]
struct TxDiagnostics {
    outcome: TxOutcome,
    calls: u32,
    accepted: u32,
    backpressure: u32,
}

#[unsafe(no_mangle)]
static mut P4_USB_TX_DIAGNOSTICS: TxDiagnostics = TxDiagnostics {
    outcome: TxOutcome::Timeout,
    calls: 0,
    accepted: 0,
    backpressure: 0,
};

unsafe extern "C" {
    static mut uart_acm_dev: *mut CdcAcmDevice;
    fn esp_rom_get_uart_device() -> *const u8;
    fn esp_rom_get_cpu_ticks_per_us() -> u32;
    fn esp_rom_cdc_acm_line_ctrl_get(device: *mut CdcAcmDevice, ctrl: u32, val: *mut u32) -> i32;
    fn esp_rom_esprv_intc_int_set_priority(irq: i32, priority: i32);
    fn esp_rom_isr_attach(irq: i32, handler: unsafe extern "C" fn(*mut ()), arg: *mut ());
    fn esp_rom_isr_mask(mask: i32);
    fn esp_rom_isr_unmask(mask: i32);
    fn esp_rom_cdc_acm_irq_callback_set(
        device: *mut CdcAcmDevice,
        callback: unsafe extern "C" fn(*mut CdcAcmDevice, i32),
    );
    fn esp_rom_cdc_acm_irq_state_enable(device: *mut CdcAcmDevice);
    fn esp_rom_cdc_acm_irq_rx_enable(device: *mut CdcAcmDevice);
    fn esp_rom_cdc_acm_irq_rx_disable(device: *mut CdcAcmDevice);
    fn esp_rom_usb_dw_isr_handler(arg: *mut ());
}

struct CdcRxMask(*mut CdcAcmDevice);

impl RxIrqMask for CdcRxMask {
    fn mask_rx(&mut self) {
        // Foreground runs on hart 0 with MIE cleared around each ROM access.
        unsafe { esp_rom_cdc_acm_irq_rx_disable(self.0) }
    }

    fn unmask_rx(&mut self) {
        unsafe { esp_rom_cdc_acm_irq_rx_enable(self.0) }
    }
}

impl RxLineState for CdcRxMask {
    fn quiesce_cdc_callbacks(&mut self) {
        // Both callback producers run through the ROM ISR on hart 0.
        // The enclosing foreground critical section already excludes them.
        let mstatus: u32;
        unsafe { asm!("csrr {0}, mstatus", out(reg) mstatus, options(nostack)) };
        assert_eq!(mstatus & 8, 0);
    }

    fn inspect_line_state_quiesced(&mut self) -> LineDecision {
        let current = unsafe { ptr::read_volatile(ptr::addr_of!(uart_acm_dev)) };
        if !LINE_FAULT.load(Ordering::Relaxed) && current == self.0 && observe_rts(self.0) {
            LineDecision::Benign
        } else {
            LineDecision::ResyncRequired
        }
    }
}

fn read_rts(device: *mut CdcAcmDevice) -> Option<bool> {
    let mut value = 0;
    let status = unsafe { esp_rom_cdc_acm_line_ctrl_get(device, LINE_CTRL_RTS, &mut value) };
    (status == 0).then_some(value != 0)
}

fn observe_rts(device: *mut CdcAcmDevice) -> bool {
    // Called from the USB ISR or with its delivery excluded on hart 0.
    // Latch a falling edge even if RTS rises before foreground inspection.
    match read_rts(device) {
        Some(rts) => {
            if LAST_RTS.load(Ordering::Relaxed) && !rts {
                LINE_FAULT.store(true, Ordering::Relaxed);
            }
            LAST_RTS.store(rts, Ordering::Relaxed);
        }
        None => LINE_FAULT.store(true, Ordering::Relaxed),
    }
    !LINE_FAULT.load(Ordering::Relaxed)
}

unsafe extern "C" fn cdc_event(device: *mut CdcAcmDevice, status: i32) {
    match status {
        EVT_RX => {
            unsafe { esp_rom_cdc_acm_irq_rx_disable(device) };
            SIGNAL.notify_rx_masked();
        }
        EVT_LINE_STATE => {
            observe_rts(device);
            SIGNAL.notify_line_state_changed();
        }
        _ => {}
    }
}

fn without_local_interrupts<T>(f: impl FnOnce() -> T) -> T {
    let status: u32;
    // No nomem annotation: keep the ROM operations inside the exclusion.
    unsafe { asm!("csrrci {0}, mstatus, 8", out(reg) status, options(nostack)) };
    let result = f();
    if status & 8 != 0 {
        unsafe { asm!("csrsi mstatus, 8", options(nostack)) };
    }
    result
}

fn on_p4_v3_2_core0() -> bool {
    let hart: u32;
    unsafe { asm!("csrr {0}, mhartid", out(reg) hart, options(nostack)) };
    if hart != 0 {
        return false;
    }
    let efuse = unsafe { ptr::read_volatile(EFUSE_MAC_SYS_2) };
    let major = ((efuse >> 21) & 4) | ((efuse >> 4) & 3);
    major == 3 && efuse & 0xf == 2 && efuse & (1 << 6) == 0
}

fn attach() -> Option<(P4RomCdcTx, CdcRxMask)> {
    if !on_p4_v3_2_core0() {
        return None;
    }
    without_local_interrupts(|| {
        // Match Espressif's ROM-download channel test before reading the CDC slot.
        let uart = unsafe { esp_rom_get_uart_device() };
        if uart.is_null()
            || unsafe { ptr::read_volatile(uart.wrapping_add(UART_DEVICE_NUMBER_OFFSET)) }
                != USB_OTG_DEVICE
        {
            return None;
        }
        let device = unsafe { ptr::read_volatile(ptr::addr_of!(uart_acm_dev)) };
        if device.is_null() {
            return None;
        }
        let port = unsafe { P4RomCdcTx::from_active_rom_session() }?;
        LAST_RTS.store(read_rts(device)?, Ordering::Relaxed);
        // The P4 ROM driver and interrupt vector remain installed; route only USB OTG.
        unsafe {
            esp_rom_isr_mask(USB_IRQ_MASK);
            ptr::write_volatile(
                USB_PHY_CONTROL,
                ptr::read_volatile(USB_PHY_CONTROL) | (1 << 21),
            );
            ptr::write_volatile(USB_IRQ_MAP_CORE0, USB_IRQ as u32 + 16);
            esp_rom_esprv_intc_int_set_priority(USB_IRQ, 1);
            esp_rom_isr_attach(USB_IRQ, esp_rom_usb_dw_isr_handler, ptr::null_mut());
            esp_rom_cdc_acm_irq_rx_disable(device);
            esp_rom_cdc_acm_irq_callback_set(device, cdc_event);
            esp_rom_cdc_acm_irq_state_enable(device);
            ptr::write_volatile(USB_GAHBCFG, ptr::read_volatile(USB_GAHBCFG) | 1);
            esp_rom_isr_unmask(USB_IRQ_MASK);
        }
        Some((port, CdcRxMask(device)))
    })
}

fn cycle_count() -> u32 {
    let count;
    unsafe { asm!("csrr {0}, mcycle", out(reg) count, options(nostack)) };
    count
}

fn send(
    port: &mut P4RomCdcTx,
    coord: &mut RxIrqCoordinator,
    mask: &mut CdcRxMask,
    ticks_per_us: u32,
    frame: &[u8],
) -> bool {
    let mut queue = TxQueue::<64>::new();
    if queue.enqueue(frame) != frame.len() {
        return false;
    }
    let start = cycle_count();
    let budget = ticks_per_us * TX_TIMEOUT_US;
    let retry = ticks_per_us * TX_RETRY_US;
    let outcome = loop {
        if cycle_count().wrapping_sub(start) >= budget {
            break TxOutcome::Timeout;
        }
        let outcome = without_local_interrupts(|| {
            if SIGNAL.link_change_pending()
                && coord.resolve_line_change(&SIGNAL, mask) == LineDecision::ResyncRequired
            {
                return None;
            }
            Some(queue.pump_once(port))
        });
        match outcome {
            Some(Ok(PumpResult::Progress { remaining: 0, .. })) => break TxOutcome::Sent,
            Some(Ok(PumpResult::Progress { .. })) => {}
            Some(Ok(PumpResult::Backpressure { .. })) => {
                // Give DWC interrupts time to advance TX before trying again.
                let wait = cycle_count();
                while cycle_count().wrapping_sub(wait) < retry
                    && cycle_count().wrapping_sub(start) < budget
                {
                    core::hint::spin_loop();
                }
            }
            Some(_) => break TxOutcome::FifoError,
            None => break TxOutcome::LinkInvalid,
        }
    };
    let sent = matches!(outcome, TxOutcome::Sent);
    let counts = queue.counters();
    unsafe {
        ptr::write_volatile(
            ptr::addr_of_mut!(P4_USB_TX_DIAGNOSTICS),
            TxDiagnostics {
                outcome,
                calls: counts.fill_calls as u32,
                accepted: counts.accepted_bytes as u32,
                backpressure: counts.zero_accepts as u32,
            },
        );
    }
    sent
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_entry() -> ! {
    let Some((mut port, mut mask)) = attach() else {
        stop();
    };
    // Allow the ROM DWC handler to deliver the reply and manage the link.
    unsafe { asm!("csrsi mstatus, 8", options(nostack)) };
    let ticks_per_us = unsafe { esp_rom_get_cpu_ticks_per_us() };
    let first_cycle = cycle_count();
    if !(1..=1000).contains(&ticks_per_us) || cycle_count() == first_cycle {
        stop_transport();
    }
    let mut coord = RxIrqCoordinator::new();
    if !send(
        &mut port,
        &mut coord,
        &mut mask,
        ticks_per_us,
        b"\xc0OHAI\xc0",
    ) {
        stop_transport();
    }
    let mut pump = RxPump::new();
    let mut frames = DoubleBuffer::<64>::new();
    loop {
        let service = without_local_interrupts(|| {
            coord.service_once(&SIGNAL, &mut mask, &mut port, &mut pump, &mut frames)
        });
        match service {
            Ok(ServiceResult::LinkChange) => {
                let decision =
                    without_local_interrupts(|| coord.resolve_line_change(&SIGNAL, &mut mask));
                if decision == LineDecision::ResyncRequired {
                    stop_transport();
                }
                continue;
            }
            Err(_) => stop_transport(),
            _ => {}
        }
        match frames.poll() {
            Poll::Ready { index, .. } => {
                if frames.frame(index) == Some(b"PING")
                    && !send(
                        &mut port,
                        &mut coord,
                        &mut mask,
                        ticks_per_us,
                        b"\xc0PONG\xc0",
                    )
                {
                    stop_transport();
                }
                frames.release_selected();
            }
            Poll::Error { .. } => frames.release_selected(),
            Poll::Idle => {}
        }
    }
}

fn stop_transport() -> ! {
    // A failed probe never reuses the ROM device. Recovery is an external reset.
    unsafe { esp_rom_isr_mask(USB_IRQ_MASK) };
    stop()
}

fn stop() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    stop();
}
