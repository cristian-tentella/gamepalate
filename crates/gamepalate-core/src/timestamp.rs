#[derive(Debug, Clone, PartialEq, Eq, Copy, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Timestamp(pub i64);

impl Timestamp {
    pub fn ms_since(self, earlier: Timestamp) -> i64 {
        self.0 - earlier.0
    }
}
