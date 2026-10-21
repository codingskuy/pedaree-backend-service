//! Makro kecil untuk enum bernilai teks: serde + `as_str` + `parse` dari satu daftar.

macro_rules! enum_teks {
    ($(#[$meta:meta])* $nama:ident { $($varian:ident => $teks:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
        pub enum $nama {
            $( #[serde(rename = $teks)] $varian ),+
        }

        impl $nama {
            pub fn as_str(&self) -> &'static str {
                match self { $( Self::$varian => $teks ),+ }
            }

            pub fn parse(teks: &str) -> Option<Self> {
                match teks { $( $teks => Some(Self::$varian), )+ _ => None }
            }
        }
    };
}
