use core::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeEncoding {
    UnixBytes,
    WindowsWtf16LittleEndian,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeString {
    encoding: NativeEncoding,
    units: Vec<u8>,
}

impl NativeString {
    pub fn from_unix_bytes(units: Vec<u8>) -> Self {
        Self {
            encoding: NativeEncoding::UnixBytes,
            units,
        }
    }

    pub fn from_windows_wtf16(units: &[u16]) -> Self {
        let mut bytes = Vec::with_capacity(units.len() * 2);
        for unit in units {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        Self {
            encoding: NativeEncoding::WindowsWtf16LittleEndian,
            units: bytes,
        }
    }

    pub fn from_windows_wtf16_bytes(units: Vec<u8>) -> Result<Self, NativeStringError> {
        if units.len() % 2 != 0 {
            return Err(NativeStringError::OddWindowsByteLength(units.len()));
        }
        Ok(Self {
            encoding: NativeEncoding::WindowsWtf16LittleEndian,
            units,
        })
    }

    pub const fn encoding(&self) -> NativeEncoding {
        self.encoding
    }

    pub fn units(&self) -> &[u8] {
        &self.units
    }

    pub fn to_string_lossy(&self) -> String {
        match self.encoding {
            NativeEncoding::UnixBytes => String::from_utf8_lossy(&self.units).into_owned(),
            NativeEncoding::WindowsWtf16LittleEndian => {
                let units = self
                    .units
                    .chunks_exact(2)
                    .map(|pair| u16::from_le_bytes([pair[0], pair[1]]));
                std::char::decode_utf16(units)
                    .map(|result| match result {
                        Ok(character) => character,
                        Err(_) => char::REPLACEMENT_CHARACTER,
                    })
                    .collect()
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativePath(NativeString);

impl NativePath {
    pub fn from_unix_bytes(units: Vec<u8>) -> Self {
        Self(NativeString::from_unix_bytes(units))
    }

    pub fn from_windows_wtf16(units: &[u16]) -> Self {
        Self(NativeString::from_windows_wtf16(units))
    }

    pub fn as_native_string(&self) -> &NativeString {
        &self.0
    }

    pub fn to_string_lossy(&self) -> String {
        self.0.to_string_lossy()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NativeStringError {
    OddWindowsByteLength(usize),
}

impl fmt::Display for NativeStringError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OddWindowsByteLength(length) => write!(
                formatter,
                "Windows WTF-16 byte representation must have even length; got {length}"
            ),
        }
    }
}

impl std::error::Error for NativeStringError {}

#[cfg(test)]
mod tests {
    use super::{NativeEncoding, NativeString, NativeStringError};

    #[test]
    fn unix_values_preserve_non_utf8_bytes() {
        let value = NativeString::from_unix_bytes(vec![0x66, 0x80, 0x6f]);

        assert_eq!(NativeEncoding::UnixBytes, value.encoding());
        assert_eq!(&[0x66, 0x80, 0x6f], value.units());
        assert_eq!("f�o", value.to_string_lossy());
    }

    #[test]
    fn windows_values_preserve_unpaired_surrogates() {
        let value = NativeString::from_windows_wtf16(&[0x0061, 0xd800, 0x0062]);

        assert_eq!(NativeEncoding::WindowsWtf16LittleEndian, value.encoding());
        assert_eq!(&[0x61, 0x00, 0x00, 0xd8, 0x62, 0x00], value.units());
        assert_eq!("a�b", value.to_string_lossy());
    }

    #[test]
    fn odd_windows_byte_length_is_rejected() {
        let result = NativeString::from_windows_wtf16_bytes(vec![0x61]);

        assert_eq!(Err(NativeStringError::OddWindowsByteLength(1)), result);
    }
}
