//! Windows enumeration IDs used only by the independent parity harness.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Fid {
    Standard(u64),
    Extended(u128),
}
impl Fid {
    pub fn new(raw: u64) -> Self {
        Self::Standard(raw)
    }
    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self::Extended(u128::from_le_bytes(bytes))
    }
    pub fn is_standard(self) -> bool {
        matches!(self, Self::Standard(_))
    }
    pub fn as_u64(self) -> Option<u64> {
        match self {
            Self::Standard(raw) => Some(raw),
            Self::Extended(_) => None,
        }
    }
    pub fn record_number(self) -> Option<u64> {
        self.as_u64().map(|raw| raw & 0x0000_ffff_ffff_ffff)
    }
    pub fn as_u128(self) -> u128 {
        match self {
            Self::Standard(raw) => u128::from(raw),
            Self::Extended(raw) => raw,
        }
    }
}
impl std::fmt::Display for Fid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#x}", self.as_u128())
    }
}
