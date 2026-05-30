use pinocchio::Address;

#[repr(C)]
pub struct PoolState {
    pub authority: Address,
    pub mint_a: Address,
    pub mint_b: Address,
    pub reserve_a: Address,
    pub reserve_b: Address,
    pub lp_mint: Address,
    pub amp_factor: u64,
    pub trade_fee_bps: u16,
    pub admin_fee_bps: u16,
    pub paused: u8,
    pub cb_window_sec: u64,
    pub cb_threshold: u64,
    pub cb_last_value: u64,
    pub cb_last_ts: i64,
    pub bump: u8,
}

impl PoolState {
    pub const LEN: usize = 238;

    pub const OFFSET_AUTHORITY: usize = 0;
    pub const OFFSET_MINT_A: usize = 32;
    pub const OFFSET_MINT_B: usize = 64;
    pub const OFFSET_RESERVE_A: usize = 96;
    pub const OFFSET_RESERVE_B: usize = 128;
    pub const OFFSET_LP_MINT: usize = 160;
    pub const OFFSET_AMP: usize = 192;
    pub const OFFSET_TRADE_FEE: usize = 200;
    pub const OFFSET_ADMIN_FEE: usize = 202;
    pub const OFFSET_PAUSED: usize = 204;
    pub const OFFSET_CB_WINDOW: usize = 205;
    pub const OFFSET_CB_THRESH: usize = 213;
    pub const OFFSET_CB_LAST_VAL: usize = 221;
    pub const OFFSET_CB_LAST_TS: usize = 229;
    pub const OFFSET_BUMP: usize = 237;
}

pub const ERR_PAUSED: u64 = 7000;
pub const ERR_CB_TRIGGERED: u64 = 7001;
pub const ERR_UNAUTHORIZED: u64 = 7002;
pub const ERR_INSUFFICIENT: u64 = 7003;
pub const ERR_INVALID_ACCOUNT: u64 = 7004;
pub const ERR_MATH: u64 = 7005;
