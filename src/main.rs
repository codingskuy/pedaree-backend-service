use pedaree_backend_service::config::Config;
use pedaree_backend_service::state::AppState;
use pedaree_backend_service::{cli, db, rute};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,sqlx=warn".into()),
        )
        .init();

    // Subperintah operator (ADR-0011): tidak menjalankan server.
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cli::urai(&args) {
        Ok(None) => {}
        Ok(Some(perintah)) => return jalankan_perintah(perintah).await,
        Err(pesan) => {
            eprintln!("{pesan}");
            std::process::exit(2);
        }
    }

    let config = Config::dari_env().expect("konfigurasi tidak valid");
    let pool = db::sambung(&config.database_url)
        .await
        .expect("gagal menyambung ke basis data");
    db::migrasi(&pool).await.expect("migrasi gagal");

    // Catatan idempotensi > 24 jam dibersihkan tiap jam.
    let pembersih = pool.clone();
    tokio::spawn(async move {
        let mut detak = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            detak.tick().await;
            if let Err(e) = pedaree_backend_service::idempotensi::bersihkan(&pembersih).await {
                tracing::warn!(galat = %e, "gagal membersihkan idempotensi");
            }
        }
    });

    // Outbox sinyal ke TitipO + expiry-sweep harian 06:00 WIB.
    match pedaree_backend_service::pengirim::Tujuan::dari_config(&config) {
        Some(tujuan) => {
            tokio::spawn(pedaree_backend_service::pengirim::jalankan_pengirim(pool.clone(), tujuan));
        }
        None => tracing::warn!(
            "TITIPO_BASE_URL/PEDAREE_SERVICE_KEY belum diatur: sinyal menumpuk di outbox, belum terkirim"
        ),
    }
    tokio::spawn(pedaree_backend_service::pengirim::jalankan_expiry_sweep(
        pool.clone(),
    ));

    let state = AppState::baru(pool, config.clone()).expect("kunci JWT tidak valid");
    let app = rute(state);
    let pendengar = tokio::net::TcpListener::bind(&config.bind_addr)
        .await
        .expect("gagal mengikat port");
    tracing::info!(alamat = %config.bind_addr, "pedaree-backend-service berjalan");
    axum::serve(pendengar, app).await.expect("server berhenti");
}

async fn jalankan_perintah(perintah: cli::Perintah) {
    let cli::Perintah::BuatAkun(permintaan) = perintah;
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        eprintln!("DATABASE_URL wajib diatur");
        std::process::exit(2);
    });
    let sandi = cli::baca_kata_sandi().unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });
    let pool = db::sambung(&url).await.unwrap_or_else(|e| {
        eprintln!("gagal menyambung ke basis data: {e}");
        std::process::exit(1);
    });
    db::migrasi(&pool).await.expect("migrasi gagal");
    match cli::buat_akun(&pool, &permintaan, &sandi, &argon2::Argon2::default()).await {
        Ok(a) => println!(
            "akun dibuat: pengguna={} household={}{}",
            a.pengguna_id,
            a.household_id,
            if a.household_baru { " (baru)" } else { "" }
        ),
        Err(e) => {
            eprintln!("gagal membuat akun: {e}");
            std::process::exit(1);
        }
    }
}
