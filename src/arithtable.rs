// src/arithmetic_tables.rs

pub use crate::arithmetic_coder::State;
use crate::encode::zp::table::DEFAULT_ZP_TABLE;
use lazy_static::lazy_static;

// Macro to create State structs more concisely
macro_rules! s {
    ($qe:expr, $nmps:expr, $nlps:expr, $switch:expr) => {
        State {
            qe: $qe,
            nmps: $nmps,
            nlps: $nlps,
            switch: $switch != 0,
        }
    };
}

pub const MQ_BASE: [State; 47] = [
    s!(0x5601, 1, 1, 1),
    s!(0x3401, 2, 6, 0),
    s!(0x1801, 3, 9, 0),
    s!(0x0AC1, 4, 12, 0),
    s!(0x0521, 5, 29, 0),
    s!(0x0221, 38, 33, 0),
    s!(0x5601, 7, 6, 1),
    s!(0x5401, 8, 14, 0),
    s!(0x4801, 9, 14, 0),
    s!(0x3801, 10, 14, 0),
    s!(0x3001, 11, 17, 0),
    s!(0x2401, 12, 18, 0),
    s!(0x1C01, 13, 20, 0),
    s!(0x1601, 29, 21, 0),
    s!(0x5601, 15, 14, 1),
    s!(0x5401, 16, 14, 0),
    s!(0x5101, 17, 15, 0),
    s!(0x4801, 18, 16, 0),
    s!(0x3801, 19, 17, 0),
    s!(0x3401, 20, 18, 0),
    s!(0x3001, 21, 19, 0),
    s!(0x2801, 22, 19, 0),
    s!(0x2401, 23, 20, 0),
    s!(0x2201, 24, 21, 0),
    s!(0x1C01, 25, 22, 0),
    s!(0x1801, 26, 23, 0),
    s!(0x1601, 27, 24, 0),
    s!(0x1401, 28, 25, 0),
    s!(0x1201, 29, 26, 0),
    s!(0x1101, 30, 27, 0),
    s!(0x0AC1, 31, 28, 0),
    s!(0x09C1, 32, 29, 0),
    s!(0x08A1, 33, 30, 0),
    s!(0x0521, 34, 31, 0),
    s!(0x0441, 35, 32, 0),
    s!(0x02A1, 36, 33, 0),
    s!(0x0221, 37, 34, 0),
    s!(0x0141, 38, 35, 0),
    s!(0x0111, 39, 36, 0),
    s!(0x0085, 40, 37, 0),
    s!(0x0049, 41, 38, 0),
    s!(0x0025, 42, 39, 0),
    s!(0x0015, 43, 40, 0),
    s!(0x0009, 44, 41, 0),
    s!(0x0005, 45, 42, 0),
    s!(0x0001, 45, 43, 0),
    s!(0x5601, 46, 46, 0), // dummy “all done” state
];

// Export the MQ_BASE table as MQ_STATE_TABLE for JB2 encoding
pub const MQ_STATE_TABLE: &[State] = &MQ_BASE;

pub const ZP_STATE_TABLE: [State; 256] = {
    let mut table = [State {
        qe: 0,
        nmps: 0,
        nlps: 0,
        switch: false,
    }; 256];
    let mut i = 0;
    while i < 256 {
        table[i] = State {
            qe: DEFAULT_ZP_TABLE[i].p,
            nmps: DEFAULT_ZP_TABLE[i].up as usize,
            nlps: DEFAULT_ZP_TABLE[i].dn as usize,
            switch: false, // ZP does not flip MPS
        };
        i += 1;
    }
    table
};

lazy_static! {
    pub static ref ZP_STATE_TABLE_PATCHED: [State; 256] = {
        let mut table = ZP_STATE_TABLE;
        for j in 0..256 {
            let p = table[j].qe;
            let m = DEFAULT_ZP_TABLE[j].m;
            let a = 0x10000 - p as u32;
            let a_norm = if a >= 0x8000 { a << 1 } else { a };
            if m > 0 && (a + p as u32) >= 0x8000 && a_norm >= m as u32 {
                let x = table[j].nlps;
                let y = table[x as usize].nlps;
                table[j].nlps = y;
            }
        }
        table
    };
}
