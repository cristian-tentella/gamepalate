use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Region([u8; 2]);

impl Region {
    pub const ITALY: Region = Region(*b"IT");

    pub fn new(code: &str) -> Option<Region> {
        match code.as_bytes() {
            &[first, second] if first.is_ascii_uppercase() && second.is_ascii_uppercase() => {
                Some(Region([first, second]))
            }
            _ => None,
        }
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).expect("a region is two ASCII letters")
    }
}

impl fmt::Display for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_uppercase_letters_are_a_region() {
        let region = Region::new("US").unwrap();

        assert_eq!(region.as_str(), "US");
        assert_eq!(region.to_string(), "US");
        assert_eq!(Region::new("IT"), Some(Region::ITALY));
    }

    #[test]
    fn anything_else_is_not_a_region() {
        for code in ["it", "It", "ITA", "I", "I1", "", "ÌT"] {
            assert_eq!(Region::new(code), None, "{code:?}");
        }
    }
}
