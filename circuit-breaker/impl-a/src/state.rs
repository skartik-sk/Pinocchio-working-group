use pinocchio::Address;

#[repr(C)]
pub struct WindowState {
    pub last_value: u64,
    pub last_timestamp: i64,
}

#[repr(C)]
pub struct CircuitBreakerConfig {
    pub window_seconds: u64,
    pub threshold_type: u8,
    pub threshold: u64,
}

#[repr(C)]
pub struct CircuitBreaker {
    pub authority: Address,
    pub paused: u8,
    pub config: CircuitBreakerConfig,
    pub window: WindowState,
    pub bump: u8,
}

impl CircuitBreaker {
    pub const LEN: usize = 67;

    pub const OFFSET_AUTHORITY: usize = 0;
    pub const OFFSET_PAUSED: usize = 32;
    pub const OFFSET_WINDOW_SEC: usize = 33;
    pub const OFFSET_THRESHOLD_TYPE: usize = 41;
    pub const OFFSET_THRESHOLD: usize = 42;
    pub const OFFSET_LAST_VALUE: usize = 50;
    pub const OFFSET_LAST_TS: usize = 58;
    pub const OFFSET_BUMP: usize = 66;

    pub fn is_paused(&self) -> bool {
        self.paused != 0
    }
}

#[repr(C)]
pub struct Escrow {
    pub maker: Address,
    pub mint_a: Address,
    pub mint_b: Address,
    pub amount: u64,
    pub expiry: i64,
    pub bump: u8,
}

impl Escrow {
    pub const LEN: usize = 113;

    pub const OFFSET_MAKER: usize = 0;
    pub const OFFSET_MINT_A: usize = 32;
    pub const OFFSET_MINT_B: usize = 64;
    pub const OFFSET_AMOUNT: usize = 96;
    pub const OFFSET_EXPIRY: usize = 104;
    pub const OFFSET_BUMP: usize = 112;
}

pub const ERR_UNAUTHORIZED: u64 = 6000;
pub const ERR_PAUSED: u64 = 6001;
pub const ERR_CB_TRIGGERED: u64 = 6002;
pub const ERR_INVALID_ACCOUNT: u64 = 6003;
pub const ERR_EXPIRED: u64 = 6004;
