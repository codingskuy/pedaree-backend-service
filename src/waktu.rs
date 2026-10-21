//! Jam yang dapat diganti di uji, dan tanggal kalender WIB (UTC+7).

use std::sync::Arc;

use chrono::{DateTime, Duration, NaiveDate, Utc};

pub type Jam = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

pub fn jam_sistem() -> Jam {
    Arc::new(Utc::now)
}

/// Tanggal kalender WIB untuk waktu UTC `sekarang` (hari kedaluwarsa dihitung per hari WIB).
pub fn hari_ini_wib(sekarang: DateTime<Utc>) -> NaiveDate {
    (sekarang + Duration::hours(7)).date_naive()
}

#[cfg(test)]
mod uji {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn hari_wib_maju_pukul_17_utc() {
        let t = Utc.with_ymd_and_hms(2026, 10, 20, 16, 59, 0).unwrap();
        assert_eq!(hari_ini_wib(t).to_string(), "2026-10-20");
        let t = Utc.with_ymd_and_hms(2026, 10, 20, 17, 0, 0).unwrap();
        assert_eq!(hari_ini_wib(t).to_string(), "2026-10-21");
    }
}
